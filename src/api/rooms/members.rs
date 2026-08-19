use crate::db::effective_permissions;

use uuid::Uuid;

use crate::error::{AppError, Result};
use crate::models::{Permission, Permissions, RoomMember, Visibility};

/// Get list of members pertaining to a room
/// - pool: Pool of SQL Connections
/// - user_id: Who is making the request
/// - room_id: Room to look in
pub async fn list(
    pool: &sqlx::SqlitePool,
    user_id: Uuid,
    room_id: Uuid
) -> Result<Vec<RoomMember>> {

    let mut conn = pool.acquire().await?;

    // Check if the user a member of the room
    let is_member: bool = sqlx::query_scalar(
    "SELECT EXISTS(SELECT 1 FROM room_access WHERE room_id = ?1 AND user_id = ?2)"
    )
    .bind(room_id)
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;

    if !is_member { return Err(AppError::Forbidden) }

    // Get resulting vector of room members
    let result: Vec<RoomMember> = sqlx::query_as(
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

/// Set's a target user's permissions in a room from caller
/// - pool: Pool of SQL Connections
/// - room_id: Room in which permissions pertain to
/// - caller_id: Who is making the request
/// - target_id: Who's permissions are being effected
/// - permissions: New mask of permissions
pub async fn set_permissions(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    caller_id: Uuid,
    target_id: Uuid,
    permissions: Permissions
) -> Result<()> {

    let mut conn = pool.acquire().await?;

    // Check user's permission in the room
    let perms = effective_permissions(&mut *conn, caller_id, room_id).await?;
    let Some(perms) = perms else {
        return Err(AppError::Forbidden);
    };

    // If user does not have permission to manage permission, deny
    if !perms.has(Permission::Manage) {
        return Err(AppError::Forbidden)
    }

    // If user is themselves and trying to give themselves permissions
    // that they don't already have, deny
    if caller_id == target_id {
        if !perms.contains(permissions) {
            let err = "cannot grant yourself permissions you do not hold".to_string();
            return Err(AppError::Validation(err))
        }
    }

    // Change target's permissions to new mask
    let affected = sqlx::query(
        "
        UPDATE room_access
        SET permissions = ?1
        WHERE room_id = ?2 AND user_id = ?3
    ")
    .bind(permissions)
    .bind(room_id)
    .bind(target_id)
    .execute(&mut *conn)
    .await?;

    // No users affected mean targeted user isn't in room
    if affected.rows_affected() <= 0 {
        return Err(AppError::NotFound)
    }

    Ok(())
}

/// Remove one member from one room, culling or promoting as needed.
///
/// The single lifecycle path: voluntary leave, removal, and account deletion
/// all land here. Takes a connection rather than a pool so the caller owns the
/// transaction, since account deletion removes a user from every room at once.
/// - conn: Connection to SQL DB
/// - user_id: User being removed
/// - room_id: Room to remove them from
pub async fn remove (
    conn: &mut sqlx::SqliteConnection,
    user_id: Uuid,
    room_id: Uuid,
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
        return Err(AppError::NotFound)
    }

    // There were a member, remove from the read state
    sqlx::query("DELETE FROM read_state WHERE room_id = ?1 AND user_id = ?2")
    .bind(room_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;

    // Check if any members still exit in room
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

/// Who inherits a Public room that just lost its last DeleteRoom holder.
/// `members` is ordered oldest first; None means nobody needs promoting.
fn pick_promotion(members: &[(Uuid, Permissions)]) -> Option<Uuid> {

    // Someone can still delete the room, leave it alone
    for (_, perms) in members {
        if perms.has(Permission::DeleteRoom) {
            return None;
        }
    }

    // Nobody can, the longest-standing member inherits it
    match members.first() {
        Some((user_id, _)) => Some(*user_id),
        None => None
    }
}
