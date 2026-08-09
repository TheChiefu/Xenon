use uuid::Uuid;
use tracing;

use crate::error::{self, AppError, Result};
use crate::models::{GlobalRole, Permissions};
use crate::utils;
use crate::config;

pub const DAY: i64 = 86400000; // Milliseconds

pub async fn authenticate(
    conn: &mut sqlx::SqliteConnection,
    secret: &str,
) -> Result<Option<Uuid>> {

    // No session token, quit exit
    let Some(hash) = utils::hash_session_token(secret) else {
        return Ok(None);
    };

    // Fetch user id and session expiry information
    let now: i64 = utils::now_ms();
    let row = sqlx::query_as::<_, (Uuid, i64)>(
    "
        SELECT user_id, expires_at FROM sessions
        WHERE token_hash = ?1 AND revoked_at IS NULL AND expires_at > ?2
        "
    )
    .bind(hash.as_slice())
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
        .bind(hash.as_slice())
        .execute(&mut *conn)
        .await?;
    }

    Ok(Some(user_id))
}

pub async fn insert_user(
    conn: &mut sqlx::SqliteConnection,
    id: Uuid,
    username: &str,
    display_name: &str,
    password_hash: &str,
    global_role: GlobalRole,
) -> Result<()>
{
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


/// Given a room and user id, get permission mask of that user within the room.
/// Result type, if any failure occurs bubble it up, otherwise return an Ok
/// type for the following cases:
/// 
/// - None: No permission to room (cannot access or interact with it)
/// - Some: Mask of allowed permissions in a room
pub async fn effective_permissions(
    conn: &mut sqlx::SqliteConnection,
    user_id: Uuid,
    room_id: Uuid
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


/// Check if a user has global admin permissions
pub async fn global_role(
    conn: &mut sqlx::SqliteConnection,
    user_id: Uuid
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
        },
        Err(other) => Err(AppError::Db(other))
    }
}

pub async fn room_member_ids(
    conn: &mut sqlx::SqliteConnection,
    room_id: Uuid,
) -> Result<Vec<Uuid>>
{
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