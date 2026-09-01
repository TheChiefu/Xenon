//! Which game accounts a user has linked.

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::Result;
use crate::models::{LinkedAccount, Platform};

/// Records a link, replacing whatever the user had on that platform.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `user_id` - Account the link belongs to.
/// * `platform` - Service linked.
/// * `handle` - Name shown for the account, the gamertag on Xbox.
pub async fn set(
    pool: &SqlitePool,
    user_id: Uuid,
    platform: Platform,
    handle: &str,
) -> Result<()> {
    let mut conn = pool.acquire().await?;

    sqlx::query(
        "
        INSERT INTO linked_accounts (user_id, platform, platform_handle)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(user_id, platform) DO UPDATE SET platform_handle = ?3
        "
    )
    .bind(user_id)
    .bind(platform)
    .bind(handle)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Removes one user's link on one platform.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `user_id` - Account the link belongs to.
/// * `platform` - Service to unlink.
pub async fn delete(pool: &SqlitePool, user_id: Uuid, platform: Platform) -> Result<()> {
    let mut conn = pool.acquire().await?;

    sqlx::query(
        "
        DELETE FROM linked_accounts
        WHERE user_id = ?1 AND platform = ?2
        "
    )
    .bind(user_id)
    .bind(platform)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Reads every platform one user has linked, with the name shown for each.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `user_id` - Account to read.
pub async fn list(pool: &SqlitePool, user_id: Uuid) -> Result<Vec<LinkedAccount>> {
    let mut conn = pool.acquire().await?;

    let rows = sqlx::query_as(
        "
        SELECT platform, platform_handle AS handle
        FROM linked_accounts
        WHERE user_id = ?1
        ORDER BY platform
        "
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;

    Ok(rows)
}

/// Reads the id of every user with a link on one platform.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `platform` - Service to list.
pub async fn list_users(pool: &SqlitePool, platform: Platform) -> Result<Vec<Uuid>> {
    let mut conn = pool.acquire().await?;

    let user_ids = sqlx::query_scalar(
        "
        SELECT user_id
        FROM linked_accounts
        WHERE platform = ?1
        "
    )
    .bind(platform)
    .fetch_all(&mut *conn)
    .await?;

    Ok(user_ids)
}
