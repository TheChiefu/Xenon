use axum::{Json, Router};
use axum::extract::State;
use axum::http::{self, StatusCode};
use axum::routing::post;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{api, db, validate};
use crate::error::{AppError, Result};

// Register
#[derive(Deserialize)]
pub struct RegisterRequest {
    pub invite_code: String,
    pub username: String,
    pub display_name: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct RegisterResponse {
    pub id: Uuid,
    pub session_token: String,
}

async fn register(
    State(pool): State<SqlitePool>,
    Json(body): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<RegisterResponse>)> {
    let (id, token) = api::register(
        &pool,
        &body.invite_code,
        &body.username,
        &body.display_name,
        &body.password,
    )
    .await?;

    Ok((StatusCode::CREATED, Json(RegisterResponse { id, session_token: token })))
}

// Login
#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
}

async fn login(
    State(pool): State<SqlitePool>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<LoginResponse>> {
    let token = api::login(&pool, &body.username, &body.password).await?;
    Ok(Json(LoginResponse { token }))
}

// Invite
#[derive(Deserialize)]
pub struct CreateInviteRequest {
    pub max_uses: Option<i64>,
    pub lifetime: Option<i64>,
}

#[derive(Serialize)]
pub struct CreateInviteResponse {
    pub code: String,
}

async fn create_invite(
    headers: http::HeaderMap,
    State(pool): State<SqlitePool>,
    Json(body): Json<CreateInviteRequest>,
) -> Result<(StatusCode, Json<CreateInviteResponse>)>
{
    let mut conn = pool.acquire().await?;

    let mut values = headers.get_all(http::header::AUTHORIZATION).iter();
    let token = match (values.next(), values.next()) {
        (Some(v), None) => v
            .to_str()
            .ok()
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or(AppError::InvalidCredentials)?,
        _ => return Err(AppError::InvalidCredentials),
    };

    // Authenticate
    let user_id = db::authenticate(&mut conn, token)
        .await?
        .ok_or(AppError::InvalidCredentials)?;

    
    // Check if user has permission to create invite
    db::require_admin(&mut conn, user_id).await?;

    // Create invite code
    let max_uses = body.max_uses.unwrap_or(validate::INVITE_DEFAULT_MAX_USES);
    let lifetime = body.lifetime.unwrap_or(validate::INVITE_LIFETIME_MS);
    validate::invite_params(max_uses, lifetime)?;
    let code = db::create_invite(&mut conn, user_id, Some(max_uses), Some(lifetime)).await?;
    
    Ok((StatusCode::CREATED, Json(CreateInviteResponse {code})))
}

// Main Router //
pub fn router(pool: SqlitePool) -> Router {
    Router::new()
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/invite",post(create_invite))
        .with_state(pool)
}