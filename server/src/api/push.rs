//! The VAPID public key browsers subscribe against.

use sqlx::SqlitePool;

use crate::error::Result;
use crate::utils::now_ms;

/// Stores the key the push sidecar reported, replacing any earlier one.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `public_key` - Uncompressed P-256 point, 65 bytes.
pub async fn set_key(pool: &SqlitePool, public_key: &[u8]) -> Result<()> {
    let mut conn = pool.acquire().await?;

    sqlx::query(
        "
        INSERT INTO push_keys (id, public_key, created_at)
        VALUES (1, ?1, ?2)
        ON CONFLICT(id) DO UPDATE SET public_key = ?1, created_at = ?2
        "
    )
    .bind(public_key)
    .bind(now_ms())
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Reads the stored key, or `None` before the sidecar has reported one.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
pub async fn get_public_key(pool: &SqlitePool) -> Result<Option<Vec<u8>>> {
    let mut conn = pool.acquire().await?;

    let row: Option<(Vec<u8>,)> = sqlx::query_as(
        "
        SELECT public_key
        FROM push_keys
        WHERE id = 1
        "
    )
    .fetch_optional(&mut *conn)
    .await?;

    Ok(row.map(|(key,)| key))
}
