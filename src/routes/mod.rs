mod auth;
mod files;
mod messages;
mod rooms;
mod users;
mod websockets;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use axum::extract::{DefaultBodyLimit, FromRef, FromRequestParts};
use axum::http::request::Parts;
use axum::http;
use axum::routing::{delete, get, patch, post};
use axum::Router;
use sqlx::SqlitePool;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::db;
use crate::error::{AppError, Result};

// Main Router //
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/register", post(auth::register))
        .route("/login", post(auth::login))
        .route("/register-code", post(auth::create_registration_code))
        .route("/rooms",
            post(rooms::create_room)
            .get(rooms::list_rooms)
        )
        .route("/rooms/public", get(rooms::list_public_rooms))
        .route("/rooms/{id}/join", post(rooms::join_room))
        .route("/rooms/{id}/members/me", delete(rooms::leave_room))
        .route("/rooms/{id}/messages",
            post(messages::post_message)
            .get(messages::fetch_messages)
        )
        .route("/rooms/{id}/messages/{message_id}",
            delete(messages::delete_message)
            .patch(messages::update_message)
        )
        .route("/files",
            post(files::upload)
            .layer(DefaultBodyLimit::max(files::max_body_bytes()))
        )
        .route("/files/{id}", get(files::download))
        .route("/me", get(users::get_me))
        .route("/users/{id}", get(users::get_user))
        .route("/users/{id}/role", patch(users::set_role))
        .route("/ws", get(websockets::ws_handler))
        .with_state(state)
}

/// The caller's id, resolved from the Authorization header, or from the
/// offered subprotocols on a WebSocket handshake.
/// Declared as a handler parameter, so axum authenticates before the body runs
pub struct AuthUser(pub Uuid);


/// Generic over the router's state, so it survives the state type changing
impl<S> FromRequestParts<S> for AuthUser
where 
    SqlitePool: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self> {

        let mut values = parts.headers.get_all(http::header::AUTHORIZATION).iter();
        let first = values.next();
        let second = values.next();

        // Repeated headers are joined by HTTP, so two values means two credentials were presented
        // Reject if headers are combined, rather than pick one
        let token: String = match (first, second) {
            (Some(header), None) => header
                .to_str()
                .ok()
                .and_then(|value| value.strip_prefix("Bearer "))
                .ok_or(AppError::InvalidCredentials)?
                .to_string(),

            // WebSocket handshakes carry the token as a subprotocol, since a
            // browser cannot set Authorization on one
            (None, None) => parts.headers.get(http::header::SEC_WEBSOCKET_PROTOCOL)
                .and_then(token_from_protocol)
                .ok_or(AppError::InvalidCredentials)?,

            _ => return Err(AppError::InvalidCredentials),
        };

        // Authenticate
        let pool = SqlitePool::from_ref(state);
        let mut conn = pool.acquire().await?;
        let user_id = db::authenticate(&mut conn, &token)
            .await?
            .ok_or(AppError::InvalidCredentials)?;

        Ok(AuthUser(user_id))
    }
}

/// Reads the session token from a handshake's offered subprotocols, which the
/// client sends as `Bearer, <token>` to mirror the Authorization header.
fn token_from_protocol(header: &http::HeaderValue) -> Option<String> {
    let mut offered = header.to_str().ok()?.split(',').map(str::trim);
    if offered.next()? != "Bearer" {
        return None;
    }
    let token = offered.next()?;
    (!token.is_empty()).then(|| token.to_string())
}

// App State //

/// One broadcast channel per connected user, keyed by user id.
///
/// Room membership stays in room_access and is read at broadcast time
///
/// - Arc: Counted pointer, so every clone of AppState reads and writes to the same map
/// - RwLock: (Many readers/one writer mutex) Connects and disconnects write, broadcasts read
/// - broadcast: a single send reaches every subscriber, so one user's several
///   devices each receive it. Sockets subscribe on connect and their
///   subscription ends when they drop. The map keeps the channel after its
///   last subscriber leaves.
type Registry = Arc<RwLock<HashMap<Uuid, broadcast::Sender<String>>>>;

/// Cloned per request
/// FromRef generates a per-field accessor, letting handlers keep asking for State<SqlitePool>
#[derive(Clone, FromRef)]
pub struct AppState {
    pool: SqlitePool,
    registry: Registry
}

impl AppState{
    pub fn new(pool: SqlitePool) -> Self {
        AppState{
            pool,
            registry: Registry::default()
        }
    }
}

