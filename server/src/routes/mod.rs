//! The HTTP router and the auth extractor.

mod auth;
mod files;
mod links;
pub mod messages;
mod push;
mod rooms;
mod server;
mod users;

use axum::extract::{DefaultBodyLimit, FromRef, FromRequestParts};
use axum::http;
use axum::http::request::Parts;
use axum::routing::{delete, get, patch, post, put};
use axum::Router;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db;
use crate::error::{AppError, Result};
use crate::sockets;
use crate::sockets::connection;
use crate::state::AppState;
use crate::utils;

/// The authenticated caller.
///
/// Declared as a handler parameter, so axum authenticates before the body runs.
pub struct AuthUser(
    /// Id of the user the session belongs to
    pub Uuid,

    /// The `sessions.token_hash` the request authenticated with
    pub [u8; 32],
);

/// Generic over the router's state, so it survives the state type changing.
impl<S> FromRequestParts<S> for AuthUser
where
    SqlitePool: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self> {

        let token = bearer_token(parts)?;

        // Not a token this server could have issued
        let session_hash = utils::hash_session_token(&token)
            .ok_or(AppError::InvalidCredentials)?;

        // Authenticate
        let pool = SqlitePool::from_ref(state);
        let mut conn = pool.acquire().await?;
        let user_id = db::authenticate(&mut conn, &session_hash)
            .await?
            .ok_or(AppError::InvalidCredentials)?;

        Ok(AuthUser(user_id, session_hash))
    }
}

// Main Router //

/// Builds the router carrying every route on the server.
///
/// # Arguments
///
/// * `state` - Pool and socket registry.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/login", post(auth::login))
        .route("/register", post(auth::register))
        .route("/register-code", post(auth::create_registration_code))
        .route("/transfer-ownership", post(users::transfer_ownership))

        // Me
        .route("/me",
            get(users::get_me)
            .patch(users::update_me)
            .delete(users::delete_me)
        )
        .route("/me/password", patch(users::update_my_password))
        .route("/me/status", put(users::update_my_status))
        .route("/me/rooms", get(rooms::list_my_rooms))
        .route("/me/invites", get(rooms::list_my_invites))
        .route("/me/invites/{room_id}", delete(rooms::decline_invite))

        // Rooms
        .route("/rooms",
            post(rooms::create_room)
            .get(rooms::list_discoverable_rooms)
        )
        .route("/rooms/{id}",
            get(rooms::get_room)
            .delete(rooms::delete_room)
            .patch(rooms::update)
        )
        .route("/rooms/{id}/join", post(rooms::join_room))
        .route("/rooms/{id}/leave", post(rooms::leave_room))
        .route("/rooms/{id}/messages",
            post(messages::post_message)
            .get(messages::fetch_messages)
        )
        .route("/rooms/{id}/invites",
            post(rooms::invite_user)
            .get(rooms::list_invites)
        )
        .route("/rooms/{id}/invites/{user_id}", delete(rooms::revoke_invite))
        .route("/rooms/{id}/members", get(rooms::list_members))
        .route("/rooms/{id}/members/{user_id}", patch(rooms::patch))
        .route("/rooms/{id}/bans",
            get(rooms::list_bans)
            .post(rooms::ban_user)
        )
        .route("/rooms/{id}/bans/{user_id}", delete(rooms::unban_user))

        // Messages
        .route("/messages/{id}",
            delete(messages::delete_message)
            .patch(messages::update_message)
        )

        // Files
        .route("/files",
            post(files::upload)
            .layer(DefaultBodyLimit::max(files::max_body_bytes()))
        )
        .route("/files/{id}", get(files::download))

        // Users
        .route("/users", get(users::get_users))
        .route("/users/{id}",
            get(users::get_user)
            .delete(users::delete_user)
        )
        .route("/users/{id}/role", patch(users::set_role))

        // Push
        .route("/sidecar/ws", get(sockets::sidecar::handler))
        .route("/push/vapid", get(push::vapid_key))
        .route("/me/push",
            post(push::subscribe)
            .delete(push::unsubscribe)
        )

        // Linked game accounts
        .route("/me/links", get(links::list))
        .route("/me/links/{platform}",
            post(links::start)
            .delete(links::unlink)
        )
        .route("/links/callback", get(links::callback))

        // Other
        .route("/ws", get(connection::ws_handler))
        .route("/server", get(server::info))
        .route("/server/version", get(server::version))
        .route("/server/type", get(server::kind))
        .route("/health", get(server::health_check))
        .with_state(state)
}

// Helper Methods //

/// Reads the session token a request presents.
///
/// # Arguments
///
/// * `parts` - Head of the request being authenticated.
///
/// # Errors
///
/// Returns `AppError::InvalidCredentials` if no token is presented, or if more
/// than one Authorization header is.
fn bearer_token(parts: &Parts) -> Result<String> {

    let mut values = parts.headers.get_all(http::header::AUTHORIZATION).iter();
    let first = values.next();
    let second = values.next();

    // Repeated headers are joined by HTTP, so two values means two credentials were presented
    // Reject if headers are combined, rather than pick one
    match (first, second) {
        (Some(header), None) => Ok(header
            .to_str()
            .ok()
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or(AppError::InvalidCredentials)?
            .to_string()),

        // WebSocket handshakes carry the token as a subprotocol, since a
        // browser cannot set Authorization on one
        (None, None) => parts.headers.get(http::header::SEC_WEBSOCKET_PROTOCOL)
            .and_then(token_from_protocol)
            .ok_or(AppError::InvalidCredentials),

        _ => Err(AppError::InvalidCredentials),
    }
}

/// Reads the session token from a handshake's offered subprotocols, which the
/// client sends as `Bearer, <token>` to mirror the Authorization header.
///
/// # Arguments
///
/// * `header` - The `Sec-WebSocket-Protocol` header value.
fn token_from_protocol(header: &http::HeaderValue) -> Option<String> {
    let mut offered = header.to_str().ok()?.split(',').map(str::trim);
    if offered.next()? != "Bearer" {
        return None;
    }
    let token = offered.next()?;
    (!token.is_empty()).then(|| token.to_string())
}
