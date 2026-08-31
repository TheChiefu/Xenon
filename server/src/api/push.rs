//! The VAPID public key browsers subscribe against, and the browsers subscribed.

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::Result;
use crate::sockets::events::Subscription;
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

/// Reads the browsers subscribed for a set of accounts.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `users` - Accounts to read.
pub async fn subscriptions_for(pool: &SqlitePool, users: &[Uuid]) -> Result<Vec<Subscription>> {
    if users.is_empty() {
        return Ok(Vec::new());
    }

    let mut conn = pool.acquire().await?;

    // SQL cannot bind `users` to a single `?`, so the query is assembled here
    let mut builder = sqlx::QueryBuilder::new(
        "SELECT endpoint, p256dh, auth FROM push_subscriptions WHERE user_id IN ("
    );

    // Each push adds one `?` and stores the value bound to it
    let mut list = builder.separated(", ");
    for user_id in users {
        list.push_bind(*user_id);
    }

    // Closes the list, with no comma in front of it
    list.push_unseparated(")");

    // Get all users' push subscription rows
    let rows: Vec<(String, Vec<u8>, Vec<u8>)> = builder
        .build_query_as()
        .fetch_all(&mut *conn)
        .await?;

    let mut subscriptions = Vec::with_capacity(rows.len());
    for (endpoint, p256dh, auth) in rows {
        subscriptions.push(Subscription { endpoint, p256dh, auth });
    }

    Ok(subscriptions)
}

/// Stores a browser's subscription, replacing any row for the same endpoint.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `user_id` - Account the browser is signed in as.
/// * `subscription` - What the browser produced when it subscribed.
pub async fn subscribe(
    pool: &SqlitePool,
    user_id: Uuid,
    subscription: &Subscription,
) -> Result<()> {
    let mut conn = pool.acquire().await?;

    sqlx::query(
        "
        INSERT INTO push_subscriptions (endpoint, user_id, p256dh, auth, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT(endpoint) DO UPDATE SET
            user_id = ?2,
            p256dh = ?3,
            auth = ?4,
            created_at = ?5
        "
    )
    .bind(&subscription.endpoint)
    .bind(user_id)
    .bind(&subscription.p256dh)
    .bind(&subscription.auth)
    .bind(now_ms())
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Removes one browser's subscription.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `user_id` - Account the row must belong to.
/// * `endpoint` - Push resource the row is keyed by.
pub async fn unsubscribe(
    pool: &SqlitePool,
    user_id: Uuid,
    endpoint: &str
) -> Result<()> {
    let mut conn = pool.acquire().await?;

    sqlx::query("DELETE FROM push_subscriptions WHERE endpoint = ?1 AND user_id = ?2")
        .bind(endpoint)
        .bind(user_id)
        .execute(&mut *conn)
        .await?;

    Ok(())
}
