//! HTTP handlers for registration, login, and registration codes.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::Result;
use crate::models::GlobalRole;
use crate::routes::AuthUser;
use crate::{api, db, validate};

// Data Structs //

/// POST body for creating an account.
#[derive(Deserialize)]
pub struct RegisterRequest {
    pub invite_code: String,
    pub username: String,
    pub display_name: String,
    pub password: String,
}

/// POST body for starting a session.
#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// POST body for creating a registration code.
#[derive(Deserialize)]
pub struct CreateInviteRequest {
    pub max_uses: Option<i64>,
    pub lifetime: Option<i64>,
}

/// Response carrying a new account's id and its first session token.
#[derive(Serialize)]
pub struct RegisterResponse {
    pub id: Uuid,
    pub session_token: String,
}

/// Response carrying a session token.
#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
}

/// Response carrying a registration code.
#[derive(Serialize)]
pub struct CreateInviteResponse {
    pub code: String,
}

// Routing Methods //

/// Creates an account.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `body` - Details for the new account.
pub async fn register(
    State(pool): State<SqlitePool>,
    Json(body): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<RegisterResponse>)> {

    let (id, token) = api::auth::register(
        &pool,
        &body.invite_code,
        &body.username,
        &body.display_name,
        &body.password,
    ).await?;

    Ok((StatusCode::CREATED, Json(RegisterResponse { id, session_token: token })))
}

/// Starts a session.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `body` - Credentials to authenticate with.
pub async fn login(
    State(pool): State<SqlitePool>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<LoginResponse>> {

    let token = api::auth::login(&pool, &body.username, &body.password).await?;

    Ok(Json(LoginResponse { token }))
}

/// Creates a code that lets someone register.
///
/// # Arguments
///
/// * `caller_id` - Who is issuing the code.
/// * `pool` - Pool of SQL connections.
/// * `body` - Use count and lifetime, each optional.
pub async fn create_registration_code(
    AuthUser(caller_id, ..): AuthUser,
    State(pool): State<SqlitePool>,
    Json(body): Json<CreateInviteRequest>,
) -> Result<(StatusCode, Json<CreateInviteResponse>)> {

    let mut conn = pool.acquire().await?;

    // Check if user has permission to create an invite
    let allowed = [GlobalRole::Owner, GlobalRole::Admin];
    db::require_role(&mut conn, caller_id, &allowed).await?;

    // Create invite code
    let max_uses = body.max_uses.unwrap_or(validate::INVITE_DEFAULT_MAX_USES);
    let lifetime = body.lifetime.unwrap_or(validate::INVITE_LIFETIME_MS);
    validate::invite_params(max_uses, lifetime)?;
    let code = db::create_invite(&mut conn, caller_id, Some(max_uses), Some(lifetime)).await?;

    Ok((StatusCode::CREATED, Json(CreateInviteResponse { code })))
}
