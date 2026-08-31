//! HTTP handlers for Web Push subscriptions.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::api;
use crate::error::{AppError, Result};
use crate::routes::AuthUser;
use crate::sockets::events::{ServerEvent, Subscription};
use crate::state::AppState;

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

/// Forwards a browser's subscription to the push sidecar for the caller.
///
/// Dropped silently if the sidecar is not connected, same as a push event.
///
/// # Arguments
///
/// * `user_id` - Account the browser is signed in as.
/// * `state` - Push channel the sidecar reads.
/// * `subscription` - What the browser produced when it subscribed.
pub async fn subscribe(
    AuthUser(user_id, ..): AuthUser,
    State(state): State<AppState>,
    Json(subscription): Json<Subscription>,
) -> Result<StatusCode> {

    send(&state, ServerEvent::Subscribe { user_id, subscription });

    Ok(StatusCode::NO_CONTENT)
}

/// Forwards removal of one of the caller's subscriptions to the push sidecar.
///
/// Dropped silently if the sidecar is not connected, same as a push event.
///
/// # Arguments
///
/// * `user_id` - Account the subscription must belong to.
/// * `state` - Push channel the sidecar reads.
/// * `request` - Which subscription to remove.
pub async fn unsubscribe(
    AuthUser(user_id, ..): AuthUser,
    State(state): State<AppState>,
    Json(request): Json<UnsubscribeRequest>,
) -> Result<StatusCode> {

    send(&state, ServerEvent::Unsubscribe { user_id, endpoint: request.endpoint });

    Ok(StatusCode::NO_CONTENT)
}

// Helper Methods //

/// Serializes an event and sends it on the push channel.
///
/// # Arguments
///
/// * `state` - Push channel the sidecar reads.
/// * `event` - Event to send.
fn send(state: &AppState, event: ServerEvent) {
    match serde_json::to_string(&event) {
        Ok(payload) => { let _ = state.push_channel.send(payload); }
        Err(e) => tracing::error!("failed to serialize push event: {e}")
    }
}
