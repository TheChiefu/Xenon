//! The `users` table.

use serde::Deserialize;
use uuid::Uuid;

use crate::api::rooms::members;
use crate::db;
use crate::error::{AppError, Result};
use crate::models::{GlobalRole, Status, UserProfile, UserSummary};
use crate::utils;
use crate::validate;

// Data Structs //

/// PATCH body for a user's own profile. An absent field is left as it stands.
#[derive(Deserialize)]
pub struct ProfilePatch {
    pub display_name: Option<String>,

    /// Empty string clears the text
    pub description: Option<String>,

    /// Nil UUID clears the avatar
    pub avatar_file_id: Option<Uuid>,

    /// Nil UUID clears the banner
    pub banner_file_id: Option<Uuid>,
}

// API Methods //

/// Lists one page of users, optionally filtered by a username prefix.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `match_user` - Username prefix to match on, or `None` for every user.
/// * `after` - User id to page from, or `None` for the first page.
/// * `limit` - How many users to return.
pub async fn list(
    pool: &sqlx::SqlitePool,
    match_user: Option<String>,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<UserSummary>> {

    let mut conn = pool.acquire().await?;

    // Escape patterns for SQL string
    let pattern = match_user.map(|name| {
        let escaped = name
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        format!("{escaped}%")
    });

    let users: Vec<UserSummary> = sqlx::query_as(
        "
        SELECT id, username, display_name
        FROM users u
        WHERE (?1 IS NULL OR u.id > ?1)
            AND (?2 IS NULL OR u.username LIKE ?2 ESCAPE '\\')
            AND u.deleted_at IS NULL
        ORDER BY id
        LIMIT ?3
        "
    )
    .bind(after)
    .bind(pattern)
    .bind(limit)
    .fetch_all(&mut *conn)
    .await?;

    Ok(users)
}

/// Reads a user's public profile by id.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `user_id` - User to look up.
///
/// # Errors
///
/// Returns `AppError::NotFound` if no such user exists.
pub async fn get(
    pool: &sqlx::SqlitePool,
    user_id: Uuid,
) -> Result<UserProfile> {

    let mut conn = pool.acquire().await?;

    let user: Option<UserProfile> = sqlx::query_as(
        "
        SELECT id, username, display_name, description, avatar_file_id,
               banner_file_id, global_role, created_at, deleted_at
        FROM users
        WHERE id = ?1
        "
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;

    user.ok_or(AppError::NotFound)
}

/// Writes a user's own profile, returning it as stored, or `None` if the patch
/// asked for no change.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `user_id` - User whose profile is being written.
/// * `patch` - Fields to change; an absent field is left unchanged.
///
/// # Errors
///
/// Returns `AppError::Validation` if the display name or the description is
/// outside its length limits.
pub async fn update(
    pool: &sqlx::SqlitePool,
    user_id: Uuid,
    patch: ProfilePatch,
) -> Result<Option<UserProfile>> {

    // Check if any field has changed from update patch
    let changed = patch.display_name.is_some()
        || patch.description.is_some()
        || patch.avatar_file_id.is_some()
        || patch.banner_file_id.is_some();

    if !changed {
        return Ok(None);
    }

    // Validate strings
    if let Some(name) = &patch.display_name {
        validate::display_name(name)?;
    }

    if let Some(text) = &patch.description {
        validate::profile_description(text)?;
    }

    let mut conn = pool.acquire().await?;

    // - COALESCE takes the sent value, or the column's own when none is sent
    // - nullif returns NULL when both arguments match, so the nil id clears
    // - RETURNING hands back what the row ends up holding, which is neither the
    //   value a patch left out nor the nil id it clears with
    let stored: UserProfile = sqlx::query_as(
        "
        UPDATE users SET
            display_name = COALESCE(?1, display_name),
            description = COALESCE(?2, description),
            avatar_file_id = nullif(COALESCE(?3, avatar_file_id), ?5),
            banner_file_id = nullif(COALESCE(?4, banner_file_id), ?5)
        WHERE id = ?6
        RETURNING id, username, display_name, description, avatar_file_id,
                  banner_file_id, global_role, created_at, deleted_at
        "
    )
    .bind(patch.display_name.as_deref())
    .bind(patch.description.as_deref())
    .bind(patch.avatar_file_id)
    .bind(patch.banner_file_id)
    .bind(Uuid::nil())
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;

    Ok(Some(stored))
}

/// Writes the status a user's connections start at.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `user_id` - User whose preference is being written.
/// * `status` - Status their next connection starts at.
pub async fn set_preferred_status(
    pool: &sqlx::SqlitePool,
    user_id: Uuid,
    status: Status,
) -> Result<()> {

    let mut conn = pool.acquire().await?;

    sqlx::query(
        "
        UPDATE users SET preferred_status = ?1
        WHERE id = ?2
        "
    )
    .bind(status)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Tombstones an account, stripping its credentials and profile while leaving
/// the row so messages and membership history still resolve.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `user_id` - Account being closed.
/// * `anonymize` - Whether the names are replaced and the username released.
/// * `delete_history` - Whether every message the account wrote is tombstoned.
///
/// # Errors
///
/// Returns `AppError::Forbidden` if the account holds Owner.
pub async fn delete(
    pool: &sqlx::SqlitePool,
    user_id: Uuid,
    anonymize: bool,
    delete_history: bool,
) -> Result<()> {

    let mut tx = pool.begin().await?;

    // one_owner needs a live owner, so the server is handed over first
    if db::global_role(&mut tx, user_id).await? == GlobalRole::Owner {
        return Err(AppError::Forbidden);
    }

    if delete_history {
        tombstone_messages(&mut tx, user_id).await?;
    }

    leave_every_room(&mut tx, user_id).await?;
    clear_owned_rows(&mut tx, user_id).await?;
    strip_profile(&mut tx, user_id, anonymize).await?;

    tx.commit().await?;

    Ok(())
}

/// Closes someone else's account.
///
/// The messages are always kept: destroying another member's history is a
/// room's decision, made through its own wipe.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `caller_id` - User closing the account.
/// * `target_id` - Account being closed.
/// * `anonymize` - Whether the names are replaced and the username released.
///
/// # Errors
///
/// Returns `AppError::Forbidden` if the caller is neither Owner nor Admin, if
/// an Admin targets another Admin, or if the target holds Owner.
pub async fn delete_other(
    pool: &sqlx::SqlitePool,
    caller_id: Uuid,
    target_id: Uuid,
    anonymize: bool,
) -> Result<()> {

    let mut conn = pool.acquire().await?;

    // Is the caller ranked high enough to close another account
    let caller_role = db::global_role(&mut conn, caller_id).await?;
    let permissible_roles = [GlobalRole::Owner, GlobalRole::Admin];
    if !permissible_roles.contains(&caller_role) {
        return Err(AppError::Forbidden);
    }

    // Admins can't close each other's accounts
    let target_role = db::global_role(&mut conn, target_id).await?;
    if caller_role == GlobalRole::Admin && target_role == GlobalRole::Admin {
        return Err(AppError::Forbidden);
    }

    delete(pool, target_id, anonymize, false).await
}

/// Hands the server to another account, setting the caller to `demote_to`.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `caller_id` - The Owner giving the server away.
/// * `target_id` - Account receiving Owner.
/// * `demote_to` - Role the caller keeps.
///
/// # Errors
///
/// Returns `AppError::Forbidden` if the caller is not the Owner or `demote_to`
/// is Owner, and `AppError::NotFound` if the target does not exist or is
/// tombstoned.
pub async fn transfer_ownership(
    pool: &sqlx::SqlitePool,
    caller_id: Uuid,
    target_id: Uuid,
    demote_to: GlobalRole,
) -> Result<()> {

    // Deny "demoting" target user to owner (ie. two owners, not allowed)
    if demote_to == GlobalRole::Owner {
        return Err(AppError::Forbidden);
    }

    let mut tx = pool.begin().await?;

    // User who isn't owner cannot transfer ownership
    if db::global_role(&mut tx, caller_id).await? != GlobalRole::Owner {
        return Err(AppError::Forbidden);
    }

    // The caller is demoted first
    if !db::set_global_role(&mut tx, caller_id, demote_to).await? {
        return Err(AppError::NotFound);
    }

    // Target is promoted second
    if !db::set_global_role(&mut tx, target_id, GlobalRole::Owner).await? {
        return Err(AppError::NotFound);
    }

    tx.commit().await?;

    Ok(())
}

/// Changes a user's server-wide role.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `caller_id` - User performing the role change.
/// * `target_id` - User receiving the role change.
/// * `role` - Role the target receives.
///
/// # Errors
///
/// Returns `AppError::Forbidden` if the new role is Owner, if the caller is
/// neither Owner nor Admin, if the target is the Owner, or if an Admin targets
/// another Admin; and `AppError::NotFound` if the target does not exist.
pub async fn set_role(
    pool: &sqlx::SqlitePool,
    caller_id: Uuid,
    target_id: Uuid,
    role: GlobalRole,
) -> Result<()> {

    // Ownership is not transferable through this path
    if role == GlobalRole::Owner {
        return Err(AppError::Forbidden);
    }

    // Start transaction
    let mut tx = pool.begin().await?;

    let caller_role = db::global_role(&mut tx, caller_id).await?;

    // Is the caller ranked high enough to change other's roles
    let permissible_roles = [GlobalRole::Owner, GlobalRole::Admin];
    if !permissible_roles.contains(&caller_role) {
        return Err(AppError::Forbidden);
    }

    // Can't change owner's role
    let target_role = db::global_role(&mut tx, target_id).await?;
    if target_role == GlobalRole::Owner {
        return Err(AppError::Forbidden);
    }

    // Admins can't change each other's roles
    if caller_role == GlobalRole::Admin && target_role == GlobalRole::Admin {
        return Err(AppError::Forbidden);
    }

    // No row updated, so the target does not exist
    if !db::set_global_role(&mut tx, target_id, role).await? {
        return Err(AppError::NotFound);
    }

    tx.commit().await?;

    Ok(())
}

// Helper Methods //

/// Tombstones every message an account wrote, destroying the text and bumping
/// the rooms that held them so clients refetch.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `user_id` - Author whose messages are tombstoned.
async fn tombstone_messages(
    conn: &mut sqlx::SqliteConnection,
    user_id: Uuid,
) -> Result<()> {

    sqlx::query(
        "
        UPDATE rooms SET mutation_seq = mutation_seq + 1
        WHERE id IN (SELECT DISTINCT room_id FROM messages WHERE author_id = ?1)
        "
    )
    .bind(user_id)
    .execute(&mut *conn)
    .await?;

    sqlx::query(
        "
        DELETE FROM message_attachments
        WHERE message_id IN (SELECT id FROM messages WHERE author_id = ?1)
        "
    )
    .bind(user_id)
    .execute(&mut *conn)
    .await?;

    // Tombstoned rather than deleted, so an offline client learns they are gone
    sqlx::query(
        "
        UPDATE messages SET body = NULL, deleted_at = ?1
        WHERE author_id = ?2 AND deleted_at IS NULL
        "
    )
    .bind(utils::now_ms())
    .bind(user_id)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Removes an account from every room it belongs to, one room at a time so each
/// runs the same cull and promotion checks a single removal does.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `user_id` - Account being removed.
async fn leave_every_room(
    conn: &mut sqlx::SqliteConnection,
    user_id: Uuid,
) -> Result<()> {

    let rooms: Vec<Uuid> = sqlx::query_scalar(
        "SELECT room_id FROM room_access WHERE user_id = ?1"
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;

    for room_id in rooms {
        members::remove(&mut *conn, room_id, user_id).await?;
    }

    Ok(())
}

/// Deletes the rows an account owns
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `user_id` - Account whose rows are dropped.
async fn clear_owned_rows(
    conn: &mut sqlx::SqliteConnection,
    user_id: Uuid,
) -> Result<()> {

    let statements = [
        "DELETE FROM sessions WHERE user_id = ?1",
        "DELETE FROM read_state WHERE user_id = ?1",
        "DELETE FROM linked_accounts WHERE user_id = ?1",
        "DELETE FROM user_files WHERE user_id = ?1",
        "DELETE FROM room_invites WHERE user_id = ?1",
    ];

    for statement in statements {
        sqlx::query(statement)
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    }

    Ok(())
}

/// Strips the credentials and profile from an account's row, optionally
/// replacing the names with ones derived from its own id.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `user_id` - Account being stripped.
/// * `anonymize` - Whether the names are replaced and the username released.
async fn strip_profile(
    conn: &mut sqlx::SqliteConnection,
    user_id: Uuid,
    anonymize: bool,
) -> Result<()> {

    // Set profile username to ID to tombstone it and free it for new users
    let hex = user_id.simple().to_string();
    let display_name = format!("Deleted User {}", &hex[..6]);

    // NULL the password_hash to make the account unloginable
    sqlx::query(
        "
        UPDATE users SET
            username = CASE WHEN ?1 THEN ?2 ELSE username END,
            display_name = CASE WHEN ?1 THEN ?3 ELSE display_name END,
            password_hash = NULL,
            description = '',
            avatar_file_id = NULL,
            banner_file_id = NULL,
            email = NULL,
            deleted_at = ?4
        WHERE id = ?5
        "
    )
    .bind(anonymize)
    .bind(&hex)
    .bind(&display_name)
    .bind(utils::now_ms())
    .bind(user_id)
    .execute(&mut *conn)
    .await?;

    Ok(())
}
