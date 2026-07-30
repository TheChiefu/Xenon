use sqlx::sqlite::SqlitePool;
use uuid::Uuid;

use crate::db;
use crate::error::{AppError, Result};
use crate::models::GlobalRole;
use crate::utils;

/// Generates a default owner for the server for inital setup
pub async fn ensure_owner(pool: &SqlitePool) -> Result<()> {
    let password = utils::generate_invite_code();
    let hash = utils::hash_password(&password)?;
    let username = "owner";
    let display_name = "Owner";
    let id = Uuid::now_v7();

    // Attempt to create owner account
    let mut conn = pool.acquire().await?;
    let result = db::insert_user(
        &mut *conn,
        id,
        username,
        display_name,
        &hash,
        GlobalRole::Owner
    ).await;

    match result {
        Ok(()) => {
            let fmt_pass = utils::fancy_invite_fmt(&password, 6, '-');
            println!("Owner account created: ");
            println!("Username: {username}");
            println!("Password: [{fmt_pass}] Store this somewhere safe!!!");
            Ok(())
        }
        Err(AppError::OwnerExists) => Ok(()),
        Err(e) => Err(e),
    }
}