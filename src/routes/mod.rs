mod auth;
mod messages;
mod rooms;

use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use axum::http;
use axum::routing::{delete, post};
use axum::Router;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db;
use crate::error::{AppError, Result};

// Main Router //
pub fn router(pool: SqlitePool) -> Router {
    Router::new()
        .route("/register", post(auth::register))
        .route("/login", post(auth::login))
        .route("/invite", post(auth::create_invite))
        .route("/rooms", post(rooms::create_room).get(rooms::list_rooms))
        .route("/rooms/{id}/join", post(rooms::join_room))
        .route("/rooms/{id}/members/me", delete(rooms::leave_room))
        .route("/rooms/{id}/messages", post(messages::post_message).get(messages::fetch_messages))
        .with_state(pool)
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
