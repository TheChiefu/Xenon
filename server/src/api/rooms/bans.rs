//! The `room_bans` table: who is barred from a room, and until when.

use serde::Serialize;
use uuid::Uuid;

use crate::api::rooms;
use crate::db;
use crate::error::{AppError, Result};
use crate::models::Permission;
use crate::utils;

// Data Structs //

/// One ban a room holds.
#[derive(sqlx::FromRow, Serialize)]
pub struct Entry {
    pub user_id: Uuid,
    pub created_by: Uuid,
    pub reason: Option<String>,
    pub created_at: i64,
    pub expires_at: Option<i64>,
}

// API Methods //

/// Reports whether a user holds a ban on a room that has not expired.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `room_id` - Room the ban applies to.
/// * `user_id` - User the ban is against.
/// * `now` - Time the expiry is measured against.
pub async fn exists(
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

/// Lists the unexpired bans on a room.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room the entries are within.
/// * `caller_id` - Who is requesting the list.
///
/// # Errors
///
/// Returns `AppError::Forbidden` unless the caller holds `Permission::Ban`, or
/// is server staff in a Public room.
pub async fn list(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    caller_id: Uuid,
) -> Result<Vec<Entry>> {

    let mut conn = pool.acquire().await?;

    // Reading the list requires Permission::Ban
    require_permission(&mut conn, room_id, caller_id).await?;

    // Check for all users in ban list
    let now = utils::now_ms();
    let entries = sqlx::query_as::<_, Entry>(
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

/// Bans a user from a room and removes any membership they hold.
///
/// A repeat ban on the same user overwrites the stored reason and expiry.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room the user is being banned from.
/// * `caller_id` - Who is performing the ban.
/// * `target_id` - Who is being banned.
/// * `reason` - Why the ban was issued.
/// * `expire_delta` - How long (in ms) the ban lasts from now, or `None` to
///   never expire.
///
/// # Errors
///
/// Returns `AppError::Forbidden` unless the caller holds `Permission::Ban`, or
/// is server staff in a Public room.
pub async fn create(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    caller_id: Uuid,
    target_id: Uuid,
    reason: Option<String>,
    expire_delta: Option<i64>,
) -> Result<()> {

    let mut tx = pool.begin().await?;

    require_permission(&mut tx, room_id, caller_id).await?;

    // Caller tries to ban themselves
    if caller_id == target_id {
        return Err(AppError::Validation("cannot ban yourself".to_string()));
    }

    // Ban cannot be used against another holder
    let target = db::effective_permissions(&mut tx, room_id, target_id).await?;
    if target.is_some_and(|p| p.has(Permission::Ban)) {
        return Err(AppError::Forbidden);
    }

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

    // Remove the target's membership. A user who is not in the room can still
    // be banned, so NotFound is discarded and every other error is returned
    match rooms::access::remove(&mut tx, room_id, target_id).await {
        Err(AppError::NotFound) => (),
        Err(err) => return Err(err),
        Ok(()) => ()
    }

    tx.commit().await?;

    Ok(())
}

/// Lifts a user's ban on a room.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room to lift the ban from.
/// * `caller_id` - Who is lifting the ban.
/// * `target_id` - Who is being removed from the ban list.
///
/// # Errors
///
/// Returns `AppError::Forbidden` unless the caller holds `Permission::Ban` or is
/// server staff in a Public room, and `AppError::NotFound` if the user holds no
/// ban on the room.
pub async fn revoke(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    caller_id: Uuid,
    target_id: Uuid,
) -> Result<()> {

    let mut conn = pool.acquire().await?;

    require_permission(&mut conn, room_id, caller_id).await?;

    // Unban target user
    let result = sqlx::query("DELETE FROM room_bans WHERE room_id = ?1 AND user_id = ?2")
    .bind(room_id)
    .bind(target_id)
    .execute(&mut *conn)
    .await?;

    // No row deleted, so the user was not on the ban list
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    Ok(())
}

// Helper Methods //

/// Rejects the caller if they cannot issue bans for the room.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `room_id` - Room the permission applies to.
/// * `caller_id` - User being checked.
///
/// # Errors
///
/// Returns `AppError::Forbidden` unless the caller holds `Permission::Ban`, or
/// is server staff in a Public room.
async fn require_permission(
    conn: &mut sqlx::SqliteConnection,
    room_id: Uuid,
    caller_id: Uuid,
) -> Result<()> {

    let perms = db::effective_permissions(&mut *conn, room_id, caller_id).await?;
    let permitted = perms.is_some_and(|p| p.has(Permission::Ban));
    let staff = db::staff_over_room(&mut *conn, room_id, caller_id).await?;

    if !(permitted || staff) {
        return Err(AppError::Forbidden);
    }

    Ok(())
}
