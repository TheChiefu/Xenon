use serde::Serialize;
use uuid::Uuid;

use crate::{api::rooms::self, db, error::{AppError, Result}, models::Permission, utils};


// Data Structs //

#[derive(sqlx::FromRow, Serialize)] 
pub struct RoomBanEntry {
    pub user_id: Uuid,
    pub created_by: Uuid,
    pub reason: Option<String>,
    pub created_at: i64,
    pub expires_at: Option<i64>,
}



/// Whether the user has a ban on the room that has not expired
/// - conn: Connection to SQL DB
/// - room_id: Room the ban applies to
/// - user_id: User the ban is against
/// - now: Time the expiry is measured against
pub async fn is_banned(
    conn: &mut sqlx::SqliteConnection,
    room_id: Uuid,
    user_id: Uuid,
    now: i64,
) -> Result<bool> {

    let exists: bool = sqlx::query_scalar(
        "
        SELECT EXISTS(
            SELECT 1
            FROM room_bans
            WHERE room_id = ?1 AND user_id = ?2 AND (expires_at IS NULL OR expires_at > ?3)
        )
        "
    )
    .bind(room_id)
    .bind(user_id)
    .bind(now)
    .fetch_one(&mut *conn)
    .await?;

    Ok(exists)
}

/// Retrieve list of banned users in room
/// - pool: Pool of SQL Connections
/// - room_id: Room entries are within
/// - user_id: Who is requesting the list
pub async fn list(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<RoomBanEntry>> {
    
    let mut conn = pool.acquire().await?;

    // Check if user has permission to view ban list
    require_ban_permission(&mut *conn, user_id, room_id).await?;

    // Check for all users in ban list
    let now = utils::now_ms();
    let entries= sqlx::query_as::<_, RoomBanEntry>(
        "
        SELECT user_id, created_by, reason, created_at, expires_at
        FROM room_bans
        WHERE room_id = ?1 AND (expires_at IS NULL OR expires_at > ?2)
        "
    )
    .bind(room_id)
    .bind(now)
    .fetch_all(&mut *conn)
    .await?;

    Ok(entries)

}

/// Ban a user from room
/// - pool: Pool of SQL Connections
/// - room_id: Room user is being banned from
/// - caller_id: Who is performing the ban
/// - target_id: Who is being banned
/// - reason: The reason someone is being banned
/// - expire_delta: How much time (in ms) an ban has before it expires
///     (based on creation time + expire delta) / or if at all
pub async fn ban_user(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    caller_id: Uuid,
    target_id: Uuid,
    reason: Option<String>,
    expire_delta: Option<i64>,
) -> Result<()> {

    let mut tx = pool.begin().await?;

    // Check if caller has permission to effect ban list
    require_ban_permission(&mut *tx, caller_id, room_id).await?;

    // Ban target user
    let now = utils::now_ms();
    let expires_at = expire_delta.map(|delta| now + delta);
    sqlx::query(
        "
        INSERT INTO room_bans (room_id, user_id, created_by, reason, created_at, expires_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT (room_id, user_id) DO UPDATE SET
            created_by = ?3,
            reason = ?4,
            created_at = ?5,
            expires_at = ?6
        "
    )
    .bind(room_id)
    .bind(target_id)
    .bind(caller_id)
    .bind(reason)
    .bind(now)
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;

    // Kick user from room (if they are in it)
    // If they are not in room (404, but ignore that case)
    // Return other relevant errors
    match rooms::members::remove(&mut tx, target_id, room_id).await {
        Err(AppError::NotFound) => (),
        Err(err) => return Err(err),
        Ok(_)=> ()
    }

    tx.commit().await?;

    Ok(())

}

/// Unban a user from room
/// - pool: Pool of SQL Connections
/// - room_id: Room to lift ban from
/// - caller_id: Who is lifting the ban
/// - target_id: Who is being removed from ban list
pub async fn unban_user(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    caller_id: Uuid,
    target_id: Uuid
) -> Result<()> {

    let mut conn = pool.acquire().await?;

    // Check if caller has permission to effect ban list
    require_ban_permission(&mut *conn, caller_id, room_id).await?;

    // Unban target user
    let result = sqlx::query("DELETE FROM room_bans WHERE room_id = ?1 AND user_id = ?2")
    .bind(room_id)
    .bind(target_id)
    .execute(&mut *conn)
    .await?;

    // User not found in ban list, 404
    if result.rows_affected() <= 0 {
        return Err(AppError::NotFound);
    }

    Ok(())

}

// Helper Methods //

/// Reject the caller if they cannot issue suspends for the room
/// - conn: Connection to SQL DB
/// - user_id: User being checked
/// - room_id: Room the permission applies to
async fn require_ban_permission(
    conn: &mut sqlx::SqliteConnection,
    user_id: Uuid,
    room_id: Uuid,
) -> Result<()> {

    let perms = db::effective_permissions(&mut *conn, user_id, room_id).await?;
    if !perms.is_some_and(|p| p.has(Permission::Ban)) {
        return Err(AppError::Forbidden);
    }

    Ok(())
}