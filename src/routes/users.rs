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

pub async fn get_user(
    AuthUser(_): AuthUser,
    State(pool): State<SqlitePool>,
    Path(user_id): Path<Uuid>,
) -> Result<Json<UserSummary>> {

    let user = api::users::get_user(&pool, user_id).await?;
    Ok(Json(user))
}

/// The caller's own profile (get user info)
pub async fn get_me(
    AuthUser(user_id): AuthUser,
    State(pool): State<SqlitePool>,
) -> Result<Json<UserSummary>> {

    let user = api::users::get_user(&pool, user_id).await?;
    Ok(Json(user))
}

// Role Changing
#[derive(Deserialize)]
pub struct SetRoleRequest {
    pub role: GlobalRole,
}

/// Promote or demote a user
pub async fn set_role(
    AuthUser(actor_id): AuthUser,
    State(pool): State<SqlitePool>,
    Path(target_id): Path<Uuid>,
    Json(body): Json<SetRoleRequest>
) -> Result<StatusCode> {

    api::users::set_role(&pool, actor_id, target_id, body.role).await?;
    Ok(StatusCode::NO_CONTENT)
}