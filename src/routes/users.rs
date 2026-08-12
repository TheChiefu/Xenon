use axum::extract::{Path, State};
use axum::Json;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::AuthUser;
use crate::api;
use crate::error::Result;
use crate::models::UserSummary;

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
