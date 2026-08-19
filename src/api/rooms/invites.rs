use serde::Serialize;
use uuid::Uuid;

use crate::db;
use crate::error::{AppError, Result};
use crate::models::{Permission, Permissions, RoomInvite};
use crate::utils;

// Data Structs //

#[derive(sqlx::FromRow, Serialize)] 
pub struct RoomInviteEntry {
    pub user_id: Uuid,
    pub invited_by: Uuid,
    pub created_at: i64,
    pub expires_at: Option<i64>,
}


// API Methods //

/// Invites a user to a room and store the invite request
/// to the room invites table.
/// - pool: Pool of SQL Connections
/// - room_id: The room invited to
/// - invitee: The user who is being invited
/// - inviter: The user who made the invite
/// - expire_delta: How much time (in ms) an invite has before it expires
///     (based on invite creation time + expire delta) / or if at all
pub async fn create(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    invitee: Uuid,
    inviter: Uuid,
    expire_delta: Option<i64>
) -> Result<()> {

    let mut tx = pool.begin().await?;

    // Check if user has permission to create an invite
    let perms = db::effective_permissions(&mut *tx, inviter, room_id).await?;
    can_invite(perms, invitee, inviter)?;

    // Create invite
    let now = utils::now_ms();
    let expires_at = expire_delta.map(|delta| now + delta);
    sqlx::query(
        "
        INSERT INTO room_invites (room_id, user_id, invited_by, created_at, expires_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT (room_id, user_id) DO UPDATE SET
            invited_by = ?3,
            created_at = ?4,
            expires_at = ?5
        " // Re-invites update table
    )
    .bind(room_id)
    .bind(invitee)
    .bind(inviter)
    .bind(now)
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;

    // Commit transaction
    tx.commit().await?;

    Ok(())

}

/// Get list of room invites available to user
/// - pool: Pool of SQL Connections
/// - user_id: ID of user who is receiving invites
pub async fn list_for_user(
    pool: &sqlx::SqlitePool,
    user_id: Uuid,
) -> Result<Vec<RoomInvite>> {

    let now = utils::now_ms();
    let mut conn = pool.acquire().await?;
    let invites: Vec<RoomInvite> = sqlx::query_as(
        "
        SELECT i.room_id, r.name AS room_name, i.invited_by, i.created_at, i.expires_at
        FROM room_invites i
        JOIN rooms r ON r.id = i.room_id
        WHERE i.user_id = ?1
        AND (i.expires_at IS NULL OR i.expires_at > ?2)
        "
    )
    .bind(user_id)
    .bind(now)
    .fetch_all(&mut *conn)
    .await?;

    Ok(invites)

}

/// Whether the user has an invite to the room that has not expired
/// - conn: Connection to SQL DB
/// - room_id: Room the invite is to
/// - user_id: User the invite is addressed to
/// - now: Time the expiry is measured against
pub async fn has_unexpired_invite(
    conn: &mut sqlx::SqliteConnection,
    room_id: Uuid,
    user_id: Uuid,
    now: i64,
) -> Result<bool> {

    let exists: bool = sqlx::query_scalar(
        "
        SELECT EXISTS(
            SELECT 1
            FROM room_invites
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

/// List all invitations associated with the given room
/// - pool: Pool of SQL Connections
/// - user_id: User making request
/// - room_id: Room to query
pub async fn list(
    pool: &sqlx::SqlitePool,
    user_id: Uuid,
    room_id: Uuid
) -> Result<Vec<RoomInviteEntry>> {

    let mut conn = pool.acquire().await?;
    require_invite_permission(&mut conn, user_id, room_id).await?;

    let now = utils::now_ms();
    let entries: Vec<RoomInviteEntry> = sqlx::query_as(
        "
        SELECT user_id, invited_by, created_at, expires_at
        FROM room_invites
        WHERE room_id = ?1 AND (expires_at IS NULL OR expires_at > ?2)
        "
    )
    .bind(room_id)
    .bind(now)
    .fetch_all(&mut *conn)
    .await?;

    Ok(entries)
}

/// Decline an invite the caller received
/// - pool: Pool of SQL Connections
/// - user_id: Recipient of the invite
/// - room_id: Room the invite is to
pub async fn decline(
    pool: &sqlx::SqlitePool,
    user_id: Uuid,
    room_id: Uuid,
) -> Result<()> {

    let mut conn = pool.acquire().await?;
    if !delete(&mut conn, user_id, room_id).await? {
        return Err(AppError::NotFound)
    }

    Ok(())
}

/// Withdraw an invite the caller's room issued
/// - pool: Pool of SQL Connections
/// - caller_id: User withdrawing the invite
/// - room_id: Room the invite is to
/// - invitee: User the invite was addressed to
pub async fn revoke(
    pool: &sqlx::SqlitePool,
    caller_id: Uuid,
    room_id: Uuid,
    invitee: Uuid
) -> Result<()> {

    let mut conn = pool.acquire().await?;
    require_invite_permission(&mut conn, caller_id, room_id).await?;

    if !delete(&mut conn, invitee, room_id).await? {
        return Err(AppError::NotFound)
    }

    Ok(())
}

/// Delete user from room invite table
/// - conn: Connection to SQL DB
/// - user_id: Recipient of invite
/// - room_id: Related room of invite
pub async fn delete(
    conn: &mut sqlx::SqliteConnection,
    user_id: Uuid,
    room_id: Uuid
) -> Result<bool> {

    let result = sqlx::query("DELETE FROM room_invites WHERE room_id = ?1 AND user_id = ?2")
    .bind(room_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;

    Ok(result.rows_affected() > 0)
}

// Helper Methods //

/// Reject the caller if they cannot issue invites for the room
/// - conn: Connection to SQL DB
/// - user_id: User being checked
/// - room_id: Room the permission applies to
async fn require_invite_permission(
    conn: &mut sqlx::SqliteConnection,
    user_id: Uuid,
    room_id: Uuid,
) -> Result<()> {

    let perms = db::effective_permissions(&mut *conn, user_id, room_id).await?;
    if !perms.is_some_and(|p| p.has(Permission::Invite)) {
        return Err(AppError::Forbidden);
    }

    Ok(())
}

/// Check if inviter has permission to create invites
/// - perms: Permission mask inviter has
/// - invitee: User who is being invited
/// - inviter: User who is creating invite
fn can_invite(perms: Option<Permissions>, invitee: Uuid, inviter: Uuid) -> Result<()> {

    // Permissions could not be found
    let Some(perms) = perms else {
        return Err(AppError::Forbidden);
    };

    // Inviter tries to invite themselves
    if invitee == inviter {
        return Err(AppError::Validation("cannot invite yourself".to_string()))
    }

    // If inviter does not have invite permissions
    if !perms.has(Permission::Invite) {
        return Err(AppError::Forbidden)
    }

    return Ok(())
}
