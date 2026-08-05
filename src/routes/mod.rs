mod auth;
mod messages;
mod rooms;
mod websockets;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use axum::http;
use axum::routing::{delete, get, post};
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
        .route("/invite", post(auth::create_invite))
        .route("/rooms", post(rooms::create_room).get(rooms::list_rooms))
        .route("/rooms/{id}/join", post(rooms::join_room))
        .route("/rooms/{id}/members/me", delete(rooms::leave_room))
        .route("/rooms/{id}/messages", post(messages::post_message).get(messages::fetch_messages))
        .route("/ws", get(websockets::ws_handler))
        .with_state(state)
}

/// The caller's id, resolved from the Authorization header.
/// Declared as a handler parameter, so axum authenticates before the body runs
/// and omitting it is a compile error rather than an open endpoint.
pub struct AuthUser(pub Uuid);


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
        let token = match (first, second) {
            (Some(header), None) => header
                .to_str()
                .ok()
                .and_then(|value| value.strip_prefix("Bearer "))
                .ok_or(AppError::InvalidCredentials)?,
            _ => return Err(AppError::InvalidCredentials),
        };

        // Authenticate
        let pool = SqlitePool::from_ref(state);
        let mut conn = pool.acquire().await?;
        let user_id = db::authenticate(&mut conn, token)
            .await?
            .ok_or(AppError::InvalidCredentials)?;

        Ok(AuthUser(user_id))
    }
}

// App State

/// One broadcast channel per room, keyed by room id:
///
/// - Arc: Counted pointer, so every clone of AppState reads and writes to the same map
/// - RwLock: (Many readers/one writer mutex) Connects and disconnects write, broadcasts read
/// - broadcast: one channel per room where a single send reaches every subscriber.
///   Sockets subscribe on connect and their subscription ends when they drop.
///   The map keeps the channel after its last subscriber leaves.
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

