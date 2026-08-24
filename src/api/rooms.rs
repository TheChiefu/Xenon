//! The `rooms` table, and the tables scoped to a single room.

pub mod bans;
pub mod invites;
pub mod members;

use serde::Deserialize;
use uuid::Uuid;

use crate::db::{self, effective_permissions};
use crate::error::{AppError, Result};
use crate::models::{GlobalRole, Permission, Permissions, Room, Visibility};
use crate::utils;
use crate::validate;

// API Methods //


/// Gets a room's information by ID (hidden rooms only show to users in them)
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room to query
/// * `caller_id` - Who is requesting room information
pub async fn get(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    caller_id: Uuid,
) -> Result<Room> {

    let mut conn = pool.acquire().await?;
    let room: Room = sqlx::query_as(
        "
        SELECT r.id, r.name, r.visibility, r.default_permissions, r.created_at, r.mutation_seq
        FROM rooms r
        WHERE r.id = ?1
            AND (r.visibility IN (?2, ?3)
            OR EXISTS(SELECT 1 FROM room_access a WHERE a.room_id = r.id AND a.user_id = ?4))
        "
    )
    .bind(room_id)
    .bind(Visibility::Public)
    .bind(Visibility::Locked)
    .bind(caller_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(AppError::NotFound)?;

    Ok(room)
}

/// PATCH body for room
#[derive(Deserialize)]
pub struct RoomPatch {
    pub name: Option<String>,
    pub visibility: Option<Visibility>,
    pub default_permissions: Option<Vec<Permission>>,
}

/// Updates a given room's properties given patch.
/// 
/// # Arguments
/// 
/// * `pool` - Pool of SQL Connections
/// * `room_id` - Room to be patched
/// * `caller_id` - Who is patching the room
/// * `patch` - Patch to be applied
pub async fn update(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    caller_id: Uuid,
    patch: RoomPatch,
) -> Result<()> {

    // Extract name if available
    let renaming = patch.name.is_some();
    let mut name = patch.name.as_deref().unwrap_or("");

    // A name of nothing but spaces is stored as empty
    if name.trim().is_empty() {
        name = "";
    }

    validate::room_name(name)?;

    // Check if performing managing actions
    let managing = patch.visibility.is_some() || patch.default_permissions.is_some();
    let default_permissions = patch.default_permissions.as_deref().map(Permissions::from_list);

    let mut tx = pool.begin().await?;

    // Check if user has permissions
    let perms = effective_permissions(&mut tx, room_id, caller_id).await?;
    let perms = perms.ok_or(AppError::Forbidden)?;

    // Check permissions for requested actions
    if renaming && !perms.has(Permission::Rename) {
        return Err(AppError::Forbidden);
    }
    if managing && !perms.has(Permission::Manage) {
        return Err(AppError::Forbidden);
    }

    // - Flag decides whether the name is written
    // - NULL coalesced parameter leaves the columns as they are unchanged
    sqlx::query(
        "
        UPDATE rooms SET
            name = CASE WHEN ?1 THEN ?2 ELSE name END,
            visibility = COALESCE(?3, visibility),
            default_permissions = COALESCE(?4, default_permissions)
        WHERE id = ?5
        "
    )
    .bind(renaming)
    .bind(name)
    .bind(patch.visibility)
    .bind(default_permissions)
    .bind(room_id)
    .execute(&mut *tx)
    .await?;

    // Commit transaction
    tx.commit().await?;

    Ok(())
}

/// Creates a room and grants its creator access, returning the new room id.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `caller_id` - User creating the room.
/// * `name` - Room name. (NULL/None not allowed)
/// * `caller_permissions` - Mask the creator takes, or `None` to inherit the
///   room defaults.
/// * `default_permissions` - Mask a user takes on joining the room.
/// * `visibility` - Whether the room is discoverable and self-service.
///
/// # Errors
///
/// Returns `AppError::Validation` if a Public room's creator could not delete
/// it or the name is over the length limit, and `AppError::Forbidden` if the
/// caller's global role cannot create rooms.
pub async fn create(
    pool: &sqlx::SqlitePool,
    caller_id: Uuid,
    name: &str,
    caller_permissions: Option<Permissions>,
    default_permissions: Permissions,
    visibility: Visibility,
) -> Result<Uuid> {

    // Public rooms need someone who can delete them, they never empty out on their own
    let requires_delete_room = [Visibility::Public];
    if requires_delete_room.contains(&visibility)
        && !caller_permissions.unwrap_or(default_permissions).has(Permission::DeleteRoom)
    {
        return Err(AppError::Validation(
            "a public room's creator must be able to delete it".to_string(),
        ));
    }

    // A name of nothing but spaces is stored as the empty name
    let mut clean_room_name = name;
    if clean_room_name.trim().is_empty() {
        clean_room_name = "";
    }

    validate::room_name(clean_room_name)?;

    // Open transaction
    let mut tx = pool.begin().await?;

    // Check if user can create a room
    let allowed = [GlobalRole::Owner, GlobalRole::Admin, GlobalRole::Member];
    db::require_role(&mut tx, caller_id, &allowed).await?;

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
    .bind(caller_id)
    .bind(caller_permissions)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    // If no errors, commit transaction
    tx.commit().await?;

    // Return room id
    Ok(room_id)
}

/// Deletes a room, returning the members who were in it so the caller can tell
/// them it is gone.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room to be deleted.
/// * `caller_id` - Who is attempting to delete the room.
///
/// # Errors
///
/// Returns `AppError::Forbidden` if the caller is not a member of the room or
/// holds no `Permission::DeleteRoom`. A room that does not exist is also
/// `Forbidden`, since a non-member cannot tell the two cases apart.
pub async fn delete(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    caller_id: Uuid,
) -> Result<Vec<Uuid>> {

    let mut tx = pool.begin().await?;

    // Deleting a room requires Permission::DeleteRoom
    let perms = db::effective_permissions(&mut tx, room_id, caller_id).await?;
    if !perms.is_some_and(|p| p.has(Permission::DeleteRoom)) {
        return Err(AppError::Forbidden);
    }

    // Read the members before the delete
    let members = db::room_member_ids(&mut tx, room_id).await?;

    // Deletion automatically cascades removes messages, attachments, etc
    sqlx::query("DELETE FROM rooms WHERE id = ?1")
    .bind(room_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(members)
}


/// Grants a user access to a room, spending any invite they hold on it.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room being joined.
/// * `user_id` - User joining.
///
/// # Errors
///
/// Returns `AppError::Forbidden` if the room does not exist, the room needs an
/// invite the user does not hold, or the user is banned from it.
pub async fn join(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    user_id: Uuid,
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

    if requires_invite && !invites::exists(&mut tx, room_id, user_id, now).await? {
        return Err(AppError::Forbidden);
    }

    if bans::exists(&mut tx, room_id, user_id, now).await? {
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

    // Spend the invite the join was allowed on
    invites::delete(&mut tx, room_id, user_id).await?;

    // If no errors, commit transaction
    tx.commit().await?;

    Ok(())
}

/// Removes a user from a room at their own request.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room to leave.
/// * `user_id` - User leaving.
///
/// # Errors
///
/// Returns `AppError::NotFound` if the user is not in the room.
pub async fn leave(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    user_id: Uuid,
) -> Result<()> {

    let mut tx = pool.begin().await?;
    members::remove(&mut tx, room_id, user_id).await?;
    tx.commit().await?;

    Ok(())
}

/// Lists the rooms a user is a member of.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `user_id` - User to filter on.
pub async fn list_mine(
    pool: &sqlx::SqlitePool,
    user_id: Uuid,
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

/// Lists one page of the discoverable rooms, meaning Public and Locked.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `after` - Room id to page from, or `None` for the first page.
/// * `limit` - How many rooms to return.
pub async fn list_discoverable(
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
