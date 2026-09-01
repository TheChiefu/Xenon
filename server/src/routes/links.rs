//! HTTP handlers for linking a game account. Each one either reads
//! `linked_accounts` or hands a message to the sidecar.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use crate::api;
use crate::error::{AppError, Result};
use crate::models::Platform;
use crate::routes::AuthUser;
use crate::sockets::events::ServerEvent;
use crate::sockets::{links, registry, sidecar};
use crate::state::AppState;

/// What the browser is left on after a platform redirects to it. This page is
/// sent before the exchange has run, so it names no outcome: whether the link
/// was made is told to the tab that started it, over that tab's own socket.
const CALLBACK_PAGE: &str = "<!doctype html>
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">
<title>Xenon</title>
<body style=\"font: 16px system-ui; margin: 3rem; text-align: center\">
<p>You can close this tab and return to Xenon.</p>
<script>setTimeout(() => window.close(), 2000)</script>
";

// Data Structs //

/// One of the caller's linked accounts.
#[derive(Serialize)]
pub struct LinkResponse {
    pub platform: Platform,

    /// Name to show for the account, the gamertag on Xbox
    pub handle: String,

    /// Set once the link stopped renewing and has to be made again
    pub needs_reauth: bool
}

// Routing Methods //

/// Reads the caller's linked accounts.
///
/// # Arguments
///
/// * `user_id` - Account being read.
/// * `state` - Pool and socket registry.
pub async fn list(
    AuthUser(user_id, ..): AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<LinkResponse>>> {

    let rows = api::linked_accounts::list(&state.pool, user_id).await?;
    let needs_reauth = links::needs_reauth(&state, user_id);

    let links = rows
        .into_iter()
        .map(|link| LinkResponse {
            platform: link.platform,
            handle: link.handle,
            needs_reauth
        })
        .collect();

    Ok(Json(links))
}

/// Starts linking one platform to the caller's account, answering once the
/// request reaches the sidecar. The address to sign in at arrives on the
/// caller's socket.
///
/// # Arguments
///
/// * `user_id` - Account being linked.
/// * `platform` - Service to link.
/// * `state` - Pool and socket registry.
///
/// # Errors
///
/// Returns `AppError::Validation` when no sidecar is connected, since
/// nothing else can produce an address to sign in at.
pub async fn start(
    AuthUser(user_id, ..): AuthUser,
    Path(platform): Path<Platform>,
    State(state): State<AppState>,
) -> Result<StatusCode> {

    if state.to_sidecar.receiver_count() == 0 {
        return Err(AppError::Validation("account linking is unavailable".into()));
    }

    sidecar::send(&state, ServerEvent::LinkRequested { user_id, platform });

    Ok(StatusCode::ACCEPTED)
}

/// Removes one of the caller's links.
///
/// # Arguments
///
/// * `user_id` - Account the link belongs to.
/// * `platform` - Service to unlink.
/// * `state` - Pool and socket registry.
pub async fn unlink(
    AuthUser(user_id, ..): AuthUser,
    Path(platform): Path<Platform>,
    State(state): State<AppState>,
) -> Result<StatusCode> {

    api::linked_accounts::delete(&state.pool, user_id, platform).await?;

    let event = ServerEvent::AccountUnlinked { user_id, platform };
    registry::inform_user(&state, user_id, event);

    Ok(StatusCode::NO_CONTENT)
}

/// Receives the platform's redirect and sends its query parameters to the
/// sidecar. Answers immediately; the result reaches the user on their own
/// socket.
///
/// # Arguments
///
/// * `state` - Pool and socket registry.
/// * `params` - Query string the platform redirected with.
pub async fn callback(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {

    sidecar::send(&state, ServerEvent::LinkCallback { params });

    Html(CALLBACK_PAGE).into_response()
}
