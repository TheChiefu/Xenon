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

    // Locked and Hidden require an invite (unimplemented)
    // Also: A missing room is the same as a closed one (disallow entry)
    if visibility != Some(Visibility::Public) {
        return Err(AppError::Forbidden);
    }

    let now = utils::now_ms();

    // Check if user is banned from room. Expired rows are never swept, so the
    // expiry has to be tested here rather than trusting the row's presence
    let is_banned: bool = sqlx::query_scalar(
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
    .fetch_one(&mut *tx)
    .await?;

    // If user is still banned, disallow entry
    if is_banned {
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

    // If no errors, commit transaction
    tx.commit().await?;

    Ok(())
}

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

/// Remove one member from one room, culling or promoting as needed.
///
/// The single lifecycle path: voluntary leave, removal, and account deletion
/// all land here. Takes a connection rather than a pool so the caller owns the
/// transaction, since account deletion removes a user from every room at once.
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

    // They weren't a member, exit early
    if result.rows_affected() == 0 {
        return Err(AppError::Forbidden)
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

/// Voluntary leave. Owns the transaction that `remove_member` runs in.
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

pub async fn list_rooms(
    pool: &sqlx::SqlitePool,
    user_id: Uuid
) -> Result<Vec<Room>> {

    let mut conn = pool.acquire().await?;

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

/// Public rooms the user has not already joined, so "list_rooms" and this stay
/// disjoint: a room only ever needs to appear in one of the two lists.
pub async fn list_public_rooms(
    pool: &sqlx::SqlitePool,
    user_id: Uuid
) -> Result<Vec<Room>> {

    let mut conn = pool.acquire().await?;

    let rooms: Vec<Room> = sqlx::query_as(
        "
        SELECT r.id, r.name, r.visibility, r.default_permissions, r.created_at, r.mutation_seq
        FROM rooms r
        WHERE r.visibility = ?1
            AND NOT EXISTS (
                SELECT 1 FROM room_access a WHERE a.room_id = r.id AND a.user_id = ?2
            )
        "
    )
    .bind(Visibility::Public)
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;

    Ok(rooms)
}
