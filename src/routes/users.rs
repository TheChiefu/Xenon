use axum::extract::{Path, State};
use axum::Json;
use axum::http::StatusCode;
use serde::Deserialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::AuthUser;
use crate::api;
use crate::error::Result;
use crate::models::{GlobalRole, UserSummary};

// Data Structs //

#[derive(Deserialize)]
pub struct SetRoleRequest {
    pub role: GlobalRole,
}

// Routing Methods //

/// Get a user's public profile
/// - pool: Pool of SQL Connections
/// - user_id: User to look up
pub async fn get_user(
    AuthUser(_): AuthUser,
    State(pool): State<SqlitePool>,
    Path(user_id): Path<Uuid>,
) -> Result<Json<UserSummary>> {

    let user = api::users::get_user(&pool, user_id).await?;
    Ok(Json(user))
}

/// Get the caller's own profile
/// - AuthUser: Whose profile to return
/// - pool: Pool of SQL Connections
pub async fn get_me(
    AuthUser(user_id): AuthUser,
    State(pool): State<SqlitePool>,
) -> Result<Json<UserSummary>> {

    let user = api::users::get_user(&pool, user_id).await?;
    Ok(Json(user))
}

/// Promote or demote a user
/// - AuthUser: Who is making the change
/// - pool: Pool of SQL Connections
/// - target_id: User whose role changes
/// - body: The role to set
pub async fn set_role(
    AuthUser(actor_id): AuthUser,
    State(pool): State<SqlitePool>,
    Path(target_id): Path<Uuid>,
    Json(body): Json<SetRoleRequest>
) -> Result<StatusCode> {

    api::users::set_role(&pool, actor_id, target_id, body.role).await?;
    Ok(StatusCode::NO_CONTENT)
}