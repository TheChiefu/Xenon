//! HTTP handlers for Web Push subscriptions.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::api;
use crate::error::{AppError, Result};
use crate::routes::AuthUser;
use crate::sockets::events::Subscription;

// Data Structs //

/// DELETE body naming which subscription to remove.
#[derive(Deserialize)]
pub struct UnsubscribeRequest {
    pub endpoint: String
}

// Routing Methods //

/// Gets the key browsers subscribe against, as its 65 bytes.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
pub async fn vapid_key(State(pool): State<SqlitePool>) -> Result<Json<Vec<u8>>> {

    match api::push::get_public_key(&pool).await? {
        Some(key) => Ok(Json(key)),
        None => Err(AppError::NotFound)
    }
}

/// Stores a browser's subscription for the caller.
///
/// # Arguments
///
/// * `user_id` - Account the browser is signed in as.
/// * `pool` - Pool of SQL connections.
/// * `subscription` - What the browser produced when it subscribed.
pub async fn subscribe(
    AuthUser(user_id, ..): AuthUser,
    State(pool): State<SqlitePool>,
    Json(subscription): Json<Subscription>,
) -> Result<StatusCode> {

    api::push::subscribe(&pool, user_id, &subscription).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Removes one of the caller's subscriptions.
///
/// # Arguments
///
/// * `user_id` - Account the row must belong to.
/// * `pool` - Pool of SQL connections.
/// * `request` - Which subscription to remove.
pub async fn unsubscribe(
    AuthUser(user_id, ..): AuthUser,
    State(pool): State<SqlitePool>,
    Json(request): Json<UnsubscribeRequest>,
) -> Result<StatusCode> {

    api::push::unsubscribe(&pool, user_id, &request.endpoint).await?;

    Ok(StatusCode::NO_CONTENT)
}
