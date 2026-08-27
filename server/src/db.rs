//! Queries shared by more than one API module.
//!
//! Everything here takes a connection rather than a pool, so a caller can run
//! several of these inside one transaction.

use uuid::Uuid;

use crate::config;
use crate::error::{self, AppError, Result};
use crate::models::{GlobalRole, Permissions, Status};
use crate::utils;

/// One day in milliseconds.
pub const DAY: i64 = 86400000;

/// Resolves a session token to the user it belongs to, extending the session.
///
/// A session past its renewal threshold has its expiry pushed out, so an active
/// client never has to log in again. An unknown, revoked, or expired token
/// returns `Ok(None)`.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `token_hash` - Hash of the session token presented by the client.
pub async fn authenticate(
    conn: &mut sqlx::SqliteConnection,
    token_hash: &[u8],
) -> Result<Option<Uuid>> {

    // Fetch user id and session expiry information
    let now: i64 = utils::now_ms();
    let row = sqlx::query_as::<_, (Uuid, i64)>(
        "
        SELECT user_id, expires_at FROM sessions
        WHERE token_hash = ?1 AND revoked_at IS NULL AND expires_at > ?2
        "
    )
    .bind(token_hash)
    .bind(now)
    .fetch_optional(&mut *conn)
    .await?;

    let Some((user_id, expires_at)) = row else {
        return Ok(None);
    };

    // Extend session timer
    let lifetime = config::get().session.lifetime_days * DAY;
    let renew = config::get().session.renew_after_days_elapsed * DAY;
    if expires_at < now + lifetime - renew {
        sqlx::query(
            "
            UPDATE sessions SET expires_at = ?1 + ?2
            WHERE token_hash = ?3
            "
        )
        .bind(utils::now_ms())
        .bind(lifetime)
        .bind(token_hash)
        .execute(&mut *conn)
        .await?;
    }

    Ok(Some(user_id))
}

/// Writes a new row to the users table.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `id` - Id the new user is stored under.
/// * `username` - Login name being claimed.
/// * `display_name` - Name shown to other users.
/// * `password_hash` - PHC string produced by `utils::hash_password`.
/// * `global_role` - Server-wide role the user starts with.
///
/// # Errors
///
/// Returns `AppError::UsernameTaken` if the username exists, and
/// `AppError::OwnerExists` if an owner is already bootstrapped.
pub async fn insert_user(
    conn: &mut sqlx::SqliteConnection,
    id: Uuid,
    username: &str,
    display_name: &str,
    password_hash: &str,
    global_role: GlobalRole,
) -> Result<()> {

    sqlx::query(
        "
        INSERT INTO users (id, username, display_name, password_hash, global_role, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ",
    )
    .bind(id)
    .bind(username)
    .bind(display_name)
    .bind(password_hash)
    .bind(global_role)
    .bind(utils::now_ms())
    .execute(&mut *conn)
    .await
    .map_err(error::unique_violation)?;

    Ok(())
}

/// Opens a session for a user, returning the secret the client keeps.
///
/// Only the hash of the token reaches the database.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `user_id` - User the session belongs to.
pub async fn create_session(
    conn: &mut sqlx::SqliteConnection,
    user_id: Uuid,
) -> Result<String> {

    let token = utils::generate_session_token();
    let lifetime = config::get().session.lifetime_days * DAY;
    sqlx::query(
        "
        INSERT INTO sessions (token_hash, user_id, created_at, expires_at)
        VALUES (?1, ?2, ?3, ?3 + ?4)
        "
    )
    .bind(token.hash.as_slice())
    .bind(user_id)
    .bind(utils::now_ms())
    .bind(lifetime)
    .execute(&mut *conn)
    .await?;

    Ok(token.secret)
}

/// Creates a registration code, returning the code itself.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `created_by` - User issuing the code.
/// * `max_uses` - How many registrations the code covers, or `None` for unlimited.
/// * `lifetime` - How long (in ms) the code lasts, or `None` for no expiry.
pub async fn create_invite(
    conn: &mut sqlx::SqliteConnection,
    created_by: Uuid,
    max_uses: Option<i64>,
    lifetime: Option<i64>,
) -> Result<String> {

    let code = utils::generate_invite_code();
    let now = utils::now_ms();
    let expires_at = lifetime.map(|ms| now.saturating_add(ms));

    sqlx::query(
        "
        INSERT INTO invites (code, created_by, created_at, expires_at, max_uses)
        VALUES (?1, ?2, ?3, ?4, ?5)
        ",
    )
    .bind(&code)
    .bind(created_by)
    .bind(now)
    .bind(expires_at)
    .bind(max_uses)
    .execute(&mut *conn)
    .await?;

    Ok(code)
}

/// Resolves what a user is allowed to do in a room.
///
/// `None` means the user has no room_access row, so they can neither read the
/// room nor interact with it. `Some` is their mask, which falls back to the
/// room's defaults when their own column is NULL.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `room_id` - Room the permissions apply to.
/// * `user_id` - User being resolved.
pub async fn effective_permissions(
    conn: &mut sqlx::SqliteConnection,
    room_id: Uuid,
    user_id: Uuid,
) -> Result<Option<Permissions>> {

    let result: Option<Permissions> = sqlx::query_scalar(
        "
        SELECT COALESCE(a.permissions, r.default_permissions)
        FROM room_access a JOIN rooms r ON r.id = a.room_id
        WHERE a.room_id = ?1 AND a.user_id = ?2
        "
    )
    .bind(room_id)
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;

    Ok(result)
}

/// Reads a user's server-wide role.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `user_id` - User being read.
///
/// # Errors
///
/// Returns `AppError::Forbidden` if the user is missing or tombstoned.
pub async fn global_role(
    conn: &mut sqlx::SqliteConnection,
    user_id: Uuid,
) -> Result<GlobalRole> {

    let result = sqlx::query_scalar(
        "SELECT global_role FROM users WHERE id = ?1 AND deleted_at IS NULL"
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await;

    match result {
        Ok(val) => Ok(val),
        Err(sqlx::Error::RowNotFound) => {
            tracing::warn!("references missing or tombstoned user {user_id}");
            Err(AppError::Forbidden)
        }
        Err(other) => Err(AppError::Db(other))
    }
}

/// Rejects a user whose global role is outside the allowed set.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `user_id` - Who is performing the action.
/// * `allowed` - Roles permitted to perform it.
///
/// # Errors
///
/// Returns `AppError::Forbidden` if the role is outside `allowed`.
pub async fn require_role(
    conn: &mut sqlx::SqliteConnection,
    user_id: Uuid,
    allowed: &[GlobalRole],
) -> Result<()> {

    let role = global_role(&mut *conn, user_id).await?;
    if !allowed.contains(&role) {
        return Err(AppError::Forbidden);
    }

    Ok(())
}

/// Lists the ids of every member of a room.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `room_id` - Room to look in.
pub async fn room_member_ids(
    conn: &mut sqlx::SqliteConnection,
    room_id: Uuid,
) -> Result<Vec<Uuid>> {

    let members: Vec<Uuid> = sqlx::query_scalar(
        "
        SELECT user_id
        FROM room_access
        WHERE room_id = ?1
        "
    )
    .bind(room_id)
    .fetch_all(&mut *conn)
    .await?;

    Ok(members)
}

/// Reads the status a user's connections start at.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `user_id` - User to read.
pub async fn preferred_status(
    conn: &mut sqlx::SqliteConnection,
    user_id: Uuid,
) -> Result<Status> {

    let status: Status = sqlx::query_scalar(
        "
        SELECT preferred_status
        FROM users
        WHERE id = ?1
        "
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;

    Ok(status)
}

/// Lists the ids of everyone sharing a room with a user, the user included.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `user_id` - User whose rooms are read.
pub async fn shared_room_member_ids(
    conn: &mut sqlx::SqliteConnection,
    user_id: Uuid,
) -> Result<Vec<Uuid>> {

    let members: Vec<Uuid> = sqlx::query_scalar(
        "
        SELECT DISTINCT user_id
        FROM room_access
        WHERE room_id IN (SELECT room_id FROM room_access WHERE user_id = ?1)
        "
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;

    Ok(members)
}

/// Writes a user's global role, returning false when no user was matched.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `user_id` - User receiving the role.
/// * `role` - Role to store.
///
/// # Errors
///
/// Returns `AppError::OwnerExists` if the write would create a second owner.
pub async fn set_global_role(
    conn: &mut sqlx::SqliteConnection,
    user_id: Uuid,
    role: GlobalRole,
) -> Result<bool> {

    let updated = sqlx::query(
        "
        UPDATE users SET global_role = ?1
        WHERE id = ?2 AND deleted_at IS NULL
        "
    )
    .bind(role)
    .bind(user_id)
    .execute(&mut *conn)
    .await
    .map_err(error::unique_violation)?
    .rows_affected();

    Ok(updated == 1)
}
