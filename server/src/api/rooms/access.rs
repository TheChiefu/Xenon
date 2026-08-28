//! The `room_access` table: who belongs to a room, and what they may do in it.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db;
use crate::error::{AppError, Result};
use crate::models::{Notify, Permission, Permissions};

// Data Structs //

/// One member of a room, with their resolved permission mask.
#[derive(sqlx::FromRow, Serialize, Clone)]
pub struct Entry {
    pub user_id: Uuid,
    pub permissions: Permissions,
    pub granted_at: i64,
    /// Present on a caller's own row, members never sees another's
    pub notify: Option<Notify>
}

/// PATCH body for a room_access row
#[derive(Deserialize)]
pub struct RoomAccessPatch {
    pub permissions: Option<Vec<Permission>>,
    pub notify: Option<Notify>
}

/// Pairing of a user and their room notification preference
#[derive(sqlx::FromRow)]
pub struct NotifyUserPair {
    pub user_id: Uuid,
    pub notify: Notify,
}

// API Methods //

/// Lists the members of a room, oldest membership first.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room to look in.
/// * `caller_id` - Who is requesting the list.
///
/// # Errors
///
/// Returns `AppError::Forbidden` if the caller is not a member of the room.
pub async fn list(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    caller_id: Uuid,
) -> Result<Vec<Entry>> {

    let mut conn = pool.acquire().await?;

    // Check if the caller is a member of the room
    let is_member: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM room_access WHERE room_id = ?1 AND user_id = ?2)"
    )
    .bind(room_id)
    .bind(caller_id)
    .fetch_one(&mut *conn)
    .await?;

    if !is_member {
        return Err(AppError::Forbidden);
    }

    // Get resulting vector of room members
    let result: Vec<Entry> = sqlx::query_as(
        "
        SELECT a.user_id, a.permissions, a.granted_at,
            CASE WHEN a.user_id = ?2 THEN a.notify END AS notify
        FROM room_access a
        WHERE a.room_id = ?1
        ORDER BY a.granted_at, a.user_id
        "
    )
    .bind(room_id)
    .bind(caller_id)
    .fetch_all(&mut *conn)
    .await?;

    Ok(result)
}

/// Replaces a member's permission mask in a room.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room the permissions apply to.
/// * `caller_id` - Who is making the change.
/// * `target_id` - Whose permissions are being set.
/// * `permissions` - New mask to store.
///
/// # Errors
///
/// Returns `AppError::Forbidden` if the caller lacks `Permission::Grant`,
/// `AppError::Validation` if the caller grants themselves a permission they do
/// not hold, and `AppError::NotFound` if the target is not in the room.
pub async fn update(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    caller_id: Uuid,
    target_id: Uuid,
    permissions: Permissions,
) -> Result<()> {

    // One transaction, so the permissions read here cannot change before the write
    let mut tx = pool.begin().await?;

    // Check caller's permission in the room
    let perms = db::effective_permissions(&mut tx, room_id, caller_id).await?;
    let Some(perms) = perms else {
        return Err(AppError::Forbidden);
    };

    if !perms.has(Permission::Grant) {
        return Err(AppError::Forbidden);
    }

    // A grant is bounded by the caller's own mask
    if !perms.contains(permissions) {
        let err = "cannot grant permissions you do not hold".to_string();
        return Err(AppError::Validation(err));
    }

    // Grant cannot be used against another holder. Your own row is exempt
    if caller_id != target_id {
        let target = db::effective_permissions(&mut tx, room_id, target_id).await?;
        let Some(target) = target else {
            return Err(AppError::NotFound);
        };

        if target.has(Permission::Grant) {
            return Err(AppError::Forbidden);
        }
    }

    // Change target's permissions to new mask
    let affected = sqlx::query(
        "
        UPDATE room_access
        SET permissions = ?1
        WHERE room_id = ?2 AND user_id = ?3
        "
    )
    .bind(permissions)
    .bind(room_id)
    .bind(target_id)
    .execute(&mut *tx)
    .await?;

    // No row affected means the target is not in the room
    if affected.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    tx.commit().await?;

    Ok(())
}

/// Sets how much of a room the caller is told about.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room the preference applies to.
/// * `caller_id` - Whose membership is written.
/// * `notify` - New level to store.
///
/// # Errors
///
/// Returns `AppError::NotFound` if the caller is not in the room.
pub async fn set_notify(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    caller_id: Uuid,
    notify: Notify,
) -> Result<()> {

    let mut conn = pool.acquire().await?;

    let affected = sqlx::query(
        "
        UPDATE room_access
        SET notify = ?1
        WHERE room_id = ?2 AND user_id = ?3
        "
    )
    .bind(notify)
    .bind(room_id)
    .bind(caller_id)
    .execute(&mut *conn)
    .await?;

    if affected.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    Ok(())
}

/// Removes one member from one room, culling it if they were the last.
///
/// The single lifecycle path: voluntary leave, removal, and account deletion
/// all land here. Takes a connection rather than a pool so the caller owns the
/// transaction, since account deletion removes a user from every room at once.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `room_id` - Room to remove them from.
/// * `user_id` - User being removed.
///
/// # Errors
///
/// Returns `AppError::NotFound` if the user is not in the room.
pub async fn remove(
    conn: &mut sqlx::SqliteConnection,
    room_id: Uuid,
    user_id: Uuid,
) -> Result<()> {

    // Attempt to remove user from room access
    let result = sqlx::query(
        "DELETE FROM room_access WHERE room_id = ?1 AND user_id = ?2"
    )
    .bind(room_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;

    // No membership to remove (404)
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    // They were a member, so remove their read state as well
    sqlx::query("DELETE FROM read_state WHERE room_id = ?1 AND user_id = ?2")
    .bind(room_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;

    // Check if any members still exist in the room
    let has_members: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM room_access WHERE room_id = ?1)"
    )
    .bind(room_id)
    .fetch_one(&mut *conn)
    .await?;

    // If no members, delete the room. Cascades take the messages and the rest
    if !has_members {
        sqlx::query(
            "
            DELETE FROM rooms
            WHERE id = ?1
            "
        )
        .bind(room_id)
        .execute(&mut *conn)
        .await?;

        // No room left, exit
        return Ok(());
    }

    Ok(())
}

/// List the notify and user ID pair in a given room
/// 
/// # Arguments
///
/// * `conn` - An SQL connections
/// * `room_id` - Room to look in
/// * `caller_id` - Who is requesting the list
pub async fn list_notify_pairs(
    conn: &mut sqlx::SqliteConnection,
    room_id: Uuid,
) -> Result<Vec<NotifyUserPair>> {

    // Get resulting vector of room members
    let result: Vec<NotifyUserPair> = sqlx::query_as(
        "
        SELECT user_id, notify
        FROM room_access
        WHERE room_id = ?1
        "
    )
    .bind(room_id)
    .fetch_all(&mut *conn)
    .await?;

    Ok(result)
}

