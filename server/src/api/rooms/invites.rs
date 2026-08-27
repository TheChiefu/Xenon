//! The `room_invites` table: who has been offered access to a room.

use serde::Serialize;
use uuid::Uuid;

use crate::db;
use crate::error::{AppError, Result};
use crate::models::{Permission, Permissions};
use crate::utils;

// Data Structs //

/// One invite a room has issued.
#[derive(sqlx::FromRow, Serialize)]
pub struct Issued {
    pub user_id: Uuid,
    pub invited_by: Uuid,
    pub created_at: i64,
    pub expires_at: Option<i64>,
}

/// One invite a user has received, with the room's name joined in.
#[derive(sqlx::FromRow, Serialize)]
pub struct Received {
    pub room_id: Uuid,
    pub room_name: Option<String>,
    pub invited_by: Uuid,
    pub created_at: i64,
    pub expires_at: Option<i64>,
}

// API Methods //

/// Invites a user to a room.
///
/// A repeat invite for the same user overwrites the stored issuer and expiry.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room being invited to.
/// * `caller_id` - Who is issuing the invite.
/// * `target_id` - Who is being invited.
/// * `expire_delta` - How long (in ms) the invite lasts from now, or `None` to
///   never expire.
///
/// # Errors
///
/// Returns `AppError::Forbidden` if the caller lacks `Permission::Invite`, and
/// `AppError::Validation` if the caller invites themselves.
pub async fn create(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    caller_id: Uuid,
    target_id: Uuid,
    expire_delta: Option<i64>,
) -> Result<()> {

    let mut tx = pool.begin().await?;

    // Check if user has permission to create an invite
    let perms = db::effective_permissions(&mut tx, room_id, caller_id).await?;
    can_invite(perms, caller_id, target_id)?;

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
    .bind(target_id)
    .bind(caller_id)
    .bind(now)
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;

    // Commit transaction
    tx.commit().await?;

    Ok(())
}

/// Lists the unexpired invites addressed to a user, across every room.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `user_id` - User receiving the invites.
pub async fn list_for_user(
    pool: &sqlx::SqlitePool,
    user_id: Uuid,
) -> Result<Vec<Received>> {

    let now = utils::now_ms();
    let mut conn = pool.acquire().await?;
    let invites: Vec<Received> = sqlx::query_as(
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

/// Reports whether a user holds an invite to a room that has not expired.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `room_id` - Room the invite is to.
/// * `user_id` - User the invite is addressed to.
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

/// Lists the unexpired invites a room has issued.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room to query.
/// * `caller_id` - Who is requesting the list.
///
/// # Errors
///
/// Returns `AppError::Forbidden` if the caller lacks `Permission::Invite`.
pub async fn list(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    caller_id: Uuid,
) -> Result<Vec<Issued>> {

    let mut conn = pool.acquire().await?;

    // Reading the list requires Permission::Invite
    require_permission(&mut conn, room_id, caller_id).await?;

    let now = utils::now_ms();
    let entries: Vec<Issued> = sqlx::query_as(
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

/// Declines an invite the caller received.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room the invite is to.
/// * `caller_id` - Recipient of the invite.
///
/// # Errors
///
/// Returns `AppError::NotFound` if the caller holds no invite to the room.
pub async fn decline(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    caller_id: Uuid,
) -> Result<()> {

    let mut conn = pool.acquire().await?;
    if !delete(&mut conn, room_id, caller_id).await? {
        return Err(AppError::NotFound);
    }

    Ok(())
}

/// Withdraws an invite the caller's room issued.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room the invite is to.
/// * `caller_id` - Who is withdrawing the invite.
/// * `target_id` - User the invite was addressed to.
///
/// # Errors
///
/// Returns `AppError::Forbidden` if the caller lacks `Permission::Invite`, and
/// `AppError::NotFound` if the target holds no invite to the room.
pub async fn revoke(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    caller_id: Uuid,
    target_id: Uuid,
) -> Result<()> {

    let mut conn = pool.acquire().await?;
    require_permission(&mut conn, room_id, caller_id).await?;

    if !delete(&mut conn, room_id, target_id).await? {
        return Err(AppError::NotFound);
    }

    Ok(())
}

/// Deletes a user's invite to a room, returning false when there was none.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `room_id` - Room the invite is to.
/// * `user_id` - Recipient of the invite.
pub async fn delete(
    conn: &mut sqlx::SqliteConnection,
    room_id: Uuid,
    user_id: Uuid,
) -> Result<bool> {

    let result = sqlx::query("DELETE FROM room_invites WHERE room_id = ?1 AND user_id = ?2")
    .bind(room_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;

    Ok(result.rows_affected() > 0)
}

// Helper Methods //

/// Rejects the caller if they cannot issue invites for the room.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `room_id` - Room the permission applies to.
/// * `caller_id` - User being checked.
///
/// # Errors
///
/// Returns `AppError::Forbidden` if the caller lacks `Permission::Invite`.
async fn require_permission(
    conn: &mut sqlx::SqliteConnection,
    room_id: Uuid,
    caller_id: Uuid,
) -> Result<()> {

    let perms = db::effective_permissions(&mut *conn, room_id, caller_id).await?;
    if !perms.is_some_and(|p| p.has(Permission::Invite)) {
        return Err(AppError::Forbidden);
    }

    Ok(())
}

/// Checks whether the caller may invite the target.
///
/// # Arguments
///
/// * `perms` - Caller's resolved permissions, `None` when they are not a member.
/// * `caller_id` - Who is issuing the invite.
/// * `target_id` - Who is being invited.
///
/// # Errors
///
/// Returns `AppError::Forbidden` if the caller is not a member or lacks
/// `Permission::Invite`, and `AppError::Validation` if the two ids are equal.
fn can_invite(perms: Option<Permissions>, caller_id: Uuid, target_id: Uuid) -> Result<()> {

    // Permissions could not be found
    let Some(perms) = perms else {
        return Err(AppError::Forbidden);
    };

    // Caller tries to invite themselves
    if caller_id == target_id {
        return Err(AppError::Validation("cannot invite yourself".to_string()));
    }

    // If caller does not have invite permissions
    if !perms.has(Permission::Invite) {
        return Err(AppError::Forbidden);
    }

    Ok(())
}
