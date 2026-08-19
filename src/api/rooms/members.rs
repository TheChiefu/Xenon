//! The `room_access` table: who belongs to a room, and what they may do in it.

use serde::Serialize;
use uuid::Uuid;

use crate::db;
use crate::error::{AppError, Result};
use crate::models::{Permission, Permissions, Visibility};

// Data Structs //

/// One member of a room, with their resolved permission mask.
#[derive(sqlx::FromRow, Serialize)]
pub struct Entry {
    pub user_id: Uuid,
    pub permissions: Permissions,
    pub granted_at: i64
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
        SELECT a.user_id, COALESCE(a.permissions, r.default_permissions) AS permissions, a.granted_at
        FROM room_access a JOIN rooms r ON r.id = a.room_id
        WHERE a.room_id = ?1
        ORDER BY a.granted_at, a.user_id
        "
    )
    .bind(room_id)
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
/// Returns `AppError::Forbidden` if the caller lacks `Permission::Manage`,
/// `AppError::Validation` if the caller grants themselves a permission they do
/// not hold, and `AppError::NotFound` if the target is not in the room.
pub async fn set_permissions(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    caller_id: Uuid,
    target_id: Uuid,
    permissions: Permissions,
) -> Result<()> {

    let mut conn = pool.acquire().await?;

    // Check caller's permission in the room
    let perms = db::effective_permissions(&mut conn, room_id, caller_id).await?;
    let Some(perms) = perms else {
        return Err(AppError::Forbidden);
    };

    if !perms.has(Permission::Manage) {
        return Err(AppError::Forbidden);
    }

    // A caller editing their own row cannot add a permission they do not hold
    if caller_id == target_id && !perms.contains(permissions) {
        let err = "cannot grant yourself permissions you do not hold".to_string();
        return Err(AppError::Validation(err));
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
    .execute(&mut *conn)
    .await?;

    // No row affected means the target is not in the room
    if affected.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    Ok(())
}

/// Removes one member from one room, culling or promoting as needed.
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

    // Members remain, check if room is public.
    // Public: No one can delete it so promote oldest user to have deletion privileges
    // Private/Locked: Do no promotion
    let members: Vec<(Uuid, Permissions)> = sqlx::query_as(
        "
        SELECT a.user_id, COALESCE(a.permissions, r.default_permissions)
        FROM room_access a JOIN rooms r ON r.id = a.room_id
        WHERE a.room_id = ?1 AND r.visibility = ?2
        ORDER BY a.granted_at, a.user_id
        "
    )
    .bind(room_id)
    .bind(Visibility::Public)
    .fetch_all(&mut *conn)
    .await?;

    // Empty on Locked and Hidden rooms, which the query filters out
    let Some(promoted) = pick_promotion(&members) else {
        return Ok(());
    };

    sqlx::query(
        "
        UPDATE room_access
        SET permissions = ?1
        WHERE room_id = ?2 AND user_id = ?3
        "
    )
    .bind(Permissions::ALL)
    .bind(room_id)
    .bind(promoted)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

// Helper Methods //

/// Picks who inherits a Public room that just lost its last `DeleteRoom` holder.
///
/// # Arguments
///
/// * `members` - Remaining members, ordered oldest membership first.
fn pick_promotion(members: &[(Uuid, Permissions)]) -> Option<Uuid> {

    // Someone can still delete the room, leave it alone
    for (_, perms) in members {
        if perms.has(Permission::DeleteRoom) {
            return None;
        }
    }

    // Nobody can, the longest-standing member inherits it
    members.first().map(|(user_id, _)| *user_id)
}
