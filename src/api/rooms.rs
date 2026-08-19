pub mod bans;
pub mod invites;

use uuid::Uuid;

use crate::db::effective_permissions;
use crate::error::{AppError, Result};
use crate::models::{GlobalRole, Permission, Permissions, Room, RoomMember, Visibility};
use crate::{db, utils, validate};


/// Request to create a room:
/// - pool: SQL Pool
/// - creator_id: ID of user attempting to create a room
/// - room_name: Optional name of room (empty rooms are automatically handled by clients)
/// - creator_permissions: Permission creator gives themselves on room creation
/// - default_permissions: Permission given to users when they join the channel
/// - visibility: Determines room public/private visibility
pub async fn create_room(
    pool: &sqlx::SqlitePool,
    creator_id: Uuid,
    name: Option<&str>,
    creator_permissions: Option<Permissions>,
    default_permissions: Permissions,
    visibility: Visibility,
) -> Result<Uuid> {

    // Public rooms need someone who can delete them, they never empty out on their own
    let requires_delete_room = [Visibility::Public];
    if requires_delete_room.contains(&visibility)
        && !creator_permissions.unwrap_or(default_permissions).has(Permission::DeleteRoom)
    {
        return Err(AppError::Validation(
            "a public room's creator must be able to delete it".to_string(),
        ));
    }

    // Format validation, before any write is in flight
    let clean_room_name = validate::room_name(name)?;

    // Open transaction
    let mut tx = pool.begin().await?;

    // Check if user can create a room
    let allowed = [GlobalRole::Owner, GlobalRole::Admin, GlobalRole::Member];
    db::require_role(&mut *tx, creator_id, &allowed).await?;

    // Create room
    let now = utils::now_ms();
    let room_id = Uuid::now_v7();

    sqlx::query(
        "
        INSERT INTO rooms (id, name, visibility, default_permissions, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        "
    )
    .bind(room_id)
    .bind(clean_room_name)
    .bind(visibility)
    .bind(default_permissions)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    // Create room access
    sqlx::query(
        "
        INSERT INTO room_access (room_id, user_id, permissions, granted_at)
        VALUES (?1, ?2, ?3, ?4)
        "
    )
    .bind(room_id)
    .bind(creator_id)
    .bind(creator_permissions)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    // If no errors, commit transaction
    tx.commit().await?;

    // Return room id
    Ok(room_id)
}

/// Grant a user access to a room
/// - pool: Pool of SQL Connections
/// - user_id: User joining
/// - room_id: Room being joined
pub async fn join_room(
    pool: &sqlx::SqlitePool,
    user_id: Uuid,
    room_id: Uuid,
) -> Result<()> {

    // Open transaction
    let mut tx = pool.begin().await?;

    // Check room visibility
    let visibility: Option<Visibility> = sqlx::query_scalar(
        "
        SELECT visibility
        FROM rooms
        WHERE id = ?1
        "
    )
    .bind(room_id)
    .fetch_optional(&mut *tx)
    .await?;

    let now = utils::now_ms();

    // Public is self service. Locked and Hidden require an invite, and a
    // missing room is treated as closed
    let requires_invite = match visibility {
        Some(Visibility::Public) => false,
        Some(_) => true,
        None => return Err(AppError::Forbidden)
    };

    if requires_invite && !invites::has_unexpired_invite(&mut tx, room_id, user_id, now).await? {
        return Err(AppError::Forbidden);
    }

    if bans::is_banned(&mut tx, room_id, user_id, now).await? {
        return Err(AppError::Forbidden);
    }

    // Grant user access
    sqlx::query(
        "
        INSERT INTO room_access (room_id, user_id, permissions, granted_at)
        VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT DO NOTHING
        "
    )
    .bind(room_id)
    .bind(user_id)
    .bind(None::<Permissions>) // NULL, inherit the room default
    .bind(now)
    .execute(&mut *tx)
    .await?;

    // Use user invite (remove user from room invite table)
    invites::delete(&mut tx, user_id, room_id).await?;

    // If no errors, commit transaction
    tx.commit().await?;

    Ok(())
}

/// Get list of members pertaining to a room
/// - pool: Pool of SQL Connections
/// - user_id: Who is making the request
/// - room_id: Room to look in
pub async fn list_members(
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
pub async fn remove_member (
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

/// Remove given user from a room
/// - pool: Pool of SQL Connections
/// - user_id: User to remove from room
/// - room_id: Room to remove user from
pub async fn leave_room(
    pool: &sqlx::SqlitePool,
    user_id: Uuid,
    room_id: Uuid,
) -> Result<()> {

    let mut tx = pool.begin().await?;
    remove_member(&mut *tx, user_id, room_id).await?;
    tx.commit().await?;

    Ok(())
}

/// Get list of rooms available to user
/// - pool: Pool of SQL Connections
/// - user_id: ID of user to filter on
pub async fn list_my_rooms(
    pool: &sqlx::SqlitePool,
    user_id: Uuid
) -> Result<Vec<Room>> {

    let mut conn = pool.acquire().await?;

    // Get list based on user's room access
    let rooms: Vec<Room> = sqlx::query_as(
        "
        SELECT r.id, r.name, r.visibility, r.default_permissions, r.created_at, r.mutation_seq
        FROM rooms r JOIN room_access a ON a.room_id = r.id
        WHERE a.user_id = ?1
        "
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;

    Ok(rooms)

}

/// One page of the discoverable rooms (Public and Locked)
/// - pool: Pool of SQL Connections
/// - after: Room id to page from ('None' shows first 'limit' amount of results)
/// - limit: How many rooms to return
pub async fn list_discoverable_rooms(
    pool: &sqlx::SqlitePool,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<Room>> {

    let mut conn = pool.acquire().await?;

    let rooms: Vec<Room> = sqlx::query_as(
        "
        SELECT r.id, r.name, r.visibility, r.default_permissions, r.created_at, r.mutation_seq
        FROM rooms r
        WHERE r.visibility IN (?1, ?2)
            AND (?3 IS NULL OR r.id > ?3)
        ORDER BY id
        LIMIT ?4
        "
    )
    .bind(Visibility::Public)
    .bind(Visibility::Locked)
    .bind(after)
    .bind(limit)
    .fetch_all(&mut *conn)
    .await?;

    Ok(rooms)
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
