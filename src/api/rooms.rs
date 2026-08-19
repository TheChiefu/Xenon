pub mod bans;
pub mod members;
pub mod invites;

use uuid::Uuid;

use crate::error::{AppError, Result};
use crate::models::{GlobalRole, Permission, Permissions, Room, Visibility};
use crate::{db, utils, validate};


/// Request to create a room:
/// - pool: SQL Pool
/// - creator_id: ID of user attempting to create a room
/// - room_name: Optional name of room (empty rooms are automatically handled by clients)
/// - creator_permissions: Permission creator gives themselves on room creation
/// - default_permissions: Permission given to users when they join the channel
/// - visibility: Determines room public/private visibility
pub async fn create(
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
pub async fn join(
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

/// Remove given user from a room
/// - pool: Pool of SQL Connections
/// - user_id: User to remove from room
/// - room_id: Room to remove user from
pub async fn leave(
    pool: &sqlx::SqlitePool,
    user_id: Uuid,
    room_id: Uuid,
) -> Result<()> {

    let mut tx = pool.begin().await?;
    members::remove(&mut *tx, user_id, room_id).await?;
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
