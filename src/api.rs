use uuid::Uuid;

use crate::db;
use crate::error::{AppError, Result};
use crate::{models::GlobalRole, utils, validate};


pub async fn register(
    pool: &sqlx::SqlitePool,
    code: &str,
    username: &str,
    display_name: &str,
    password: &str
 ) -> Result<(Uuid, String)> {

    // Format Validation
    let username = username.trim();
    let display_name = display_name.trim();
    validate::username(username)?;
    validate::display_name(display_name)?;
    validate::password(password)?;

    // Open transcation
    let mut tx = pool.begin().await?; // Transaction

    // Claim invite first
    let claimed = sqlx::query(
        "
        UPDATE invites SET uses = uses + 1
        WHERE code = ?
            AND revoked_at IS NULL
            AND (expires_at IS NULL OR expires_at > unixepoch())
            AND (max_uses IS NULL OR uses < max_uses)
        ",
    )
    .bind(code.trim().to_ascii_uppercase()) // Invites code are stored uppercase
    .execute(&mut *tx)
    .await?
    .rows_affected();

    if claimed == 0 {
        // Transaction drops here, roll back claim that didn't happen
        return Err(AppError::InvalidInvite);
    }

    // Hash password for storage in DB
    let password_hash = utils::hash_password(password)?;

    // Create user
    let id = Uuid::now_v7();
    db::insert_user(
        &mut *tx,
        id,
        username,
        display_name,
        &password_hash,
        GlobalRole::Member
    ).await?;

    // Create session key
    let token = db::create_session(&mut *tx, id).await?;
    tx.commit().await?;
    Ok((id, token))
}

pub async fn login(
    pool: &sqlx::SqlitePool,
    username: &str,
    password: &str,
) -> Result<String> {

    let mut conn = pool.acquire().await?;

    // Username lookup
    let lookup = sqlx::query_as::<_, (Uuid, Option<String>)>(
    "SELECT id, password_hash FROM users WHERE username = ? AND deleted_at IS NULL",
    )
    .bind(username)
    .fetch_one(&mut *conn)
    .await;

    // Attempt to retrieve username and password hash
    let (user_id, stored_hash) = match lookup {

        // Found creds
        Ok((id, Some(hash))) => (id, hash),
        
        // Not found, burn cycles to mask kind of API call
        Ok((_, None)) | Err(sqlx::Error::RowNotFound) => {
            utils::burn_verify(password);
            return Err(AppError::InvalidCredentials);
        }
        
        // DB error, exit...
        Err(e) => return Err(AppError::Db(e))
    };

    // Verify Password:
    // `?` is the corrupt-stored-hash case (server fault)
    // `false` is an incorrect password
    if !utils::verify_password(password, &stored_hash)? {
        return Err(AppError::InvalidCredentials);
    }

    // Create session key
    let token = db::create_session(&mut *conn, user_id).await?;
    Ok(token)

}
