use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::AuthUser;
use crate::error::Result;
use crate::models::GlobalRole;
use crate::{api, db, validate};

// Data Structs //

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

// Routing Methods //

/// Create an account
/// - pool: Pool of SQL Connections
/// - body: Details for the new account
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

/// Start a session
/// - pool: Pool of SQL Connections
/// - body: Credentials to authenticate with
pub async fn login(
    State(pool): State<SqlitePool>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<LoginResponse>> {
    let token = api::auth::login(&pool, &body.username, &body.password).await?;
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

/// Create code that lets someone register
/// - AuthUser: Who is issuing the code
/// - pool: Pool of SQL Connections
/// - body: Use count and lifetime, each optional
pub async fn create_registration_code(
    AuthUser(user_id): AuthUser,
    State(pool): State<SqlitePool>,
    Json(body): Json<CreateInviteRequest>,
) -> Result<(StatusCode, Json<CreateInviteResponse>)>
{
    let mut conn = pool.acquire().await?;
    
    // Check if user has permission to create an invite
    let allowed = [GlobalRole::Owner, GlobalRole::Admin];
    db::require_role(&mut conn, user_id, &allowed).await?;

    // Create invite code
    let max_uses = body.max_uses.unwrap_or(validate::INVITE_DEFAULT_MAX_USES);
    let lifetime = body.lifetime.unwrap_or(validate::INVITE_LIFETIME_MS);
    validate::invite_params(max_uses, lifetime)?;
    let code = db::create_invite(&mut conn, user_id, Some(max_uses), Some(lifetime)).await?;
    
    Ok((StatusCode::CREATED, Json(CreateInviteResponse {code})))
}
