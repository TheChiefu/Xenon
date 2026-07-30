use uuid::Uuid;

use crate::error::{self, Result};
use crate::models::GlobalRole;
use crate::utils;

const SESSION_LIFETIME_SECS: i64 = 60 * 60 * 24 * 30;

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
        VALUES (?, ?, ?, ?, ?, unixepoch())
        ",
    )
    .bind(id)
    .bind(username)
    .bind(display_name)
    .bind(password_hash)
    .bind(global_role)
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
        VALUES (?1, ?2, unixepoch(), unixepoch() + ?3)
        "
    )
    .bind(token.hash.as_slice())
    .bind(user_id)
    .bind(SESSION_LIFETIME_SECS)
    .execute(conn)
    .await?;

    Ok(token.secret)

}

pub async fn create_invite(
    conn: &mut sqlx::SqliteConnection,
    created_by: Uuid,
    max_uses: Option<i64>,
    lifetime_secs: Option<i64>,
) -> Result<String> {
    let code = utils::generate_invite_code();

    sqlx::query(
        "
        INSERT INTO invites (code, created_by, created_at, expires_at, max_uses)
        VALUES (?1, ?2, unixepoch(),
                CASE WHEN ?3 IS NULL THEN NULL ELSE unixepoch() + ?3 END,
                ?4)
        ",
    )
    .bind(&code)
    .bind(created_by)
    .bind(lifetime_secs)
    .bind(max_uses)
    .execute(conn)
    .await?;

    Ok(code)
}