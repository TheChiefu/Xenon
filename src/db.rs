use uuid::Uuid;

use crate::error::{self, AppError, Result};
use crate::models::GlobalRole;
use crate::utils;

pub const DAY: i64 = 86400000; // Milliseconds
const SESSION_LIFETIME: i64 = DAY * 30; // Default (30 Days)
const SESSION_RENEW_AFTER: i64 = DAY; // Default (1 Day)


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
    if expires_at < now + SESSION_LIFETIME - SESSION_RENEW_AFTER {
        sqlx::query(
            "
            UPDATE sessions SET expires_at = ?1 + ?2
            WHERE token_hash = ?3
            "
        )
        .bind(utils::now_ms())
        .bind(SESSION_LIFETIME)
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
        VALUES (?, ?, ?, ?, ?, ?)
        ",
    )
    .bind(id)
    .bind(username)
    .bind(display_name)
    .bind(password_hash)
    .bind(global_role)
    .bind(utils::now_ms())
    .execute(conn)
    .await
    .map_err(error::unique_violation)?;

    Ok(())
}

pub async fn create_session(
    conn: &mut sqlx::SqliteConnection,
    user_id: Uuid,
) -> Result<String> {

    let token = utils::generate_session_token();
    sqlx::query(
        "
        INSERT INTO sessions (token_hash, user_id, created_at, expires_at)
        VALUES (?1, ?2, ?3, ?3 + ?4)
        "
    )
    .bind(token.hash.as_slice())
    .bind(user_id)
    .bind(utils::now_ms())
    .bind(SESSION_LIFETIME)
    .execute(conn)
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
    .execute(conn)
    .await?;

    Ok(code)
}

pub async fn require_admin(
    conn: &mut sqlx::SqliteConnection,
    user_id: Uuid
) -> Result<()> {

    let result = sqlx::query_scalar(
        "SELECT global_role FROM users WHERE id = ? AND deleted_at IS NULL"
    )
    .bind(user_id)
    .fetch_one(conn)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => {
            eprintln!("references missing user {user_id}");
            AppError::Forbidden
        },
        other => AppError::Db(other),
    })?;

    match result {
        GlobalRole::Admin | GlobalRole::Owner => Ok(()),
        GlobalRole::Member => Err(AppError::Forbidden)
    }
    
}