//! HTTP handlers for users.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::Result;
use crate::models::{GlobalRole, UserSummary};
use crate::routes::AuthUser;
use crate::{api, config};

// Data Structs //

/// PATCH body for changing a user's global role.
#[derive(Deserialize)]
pub struct SetRoleRequest {
    pub role: GlobalRole,
}

/// Query string for a paged user listing.
#[derive(Deserialize)]
pub struct UsersQuery {
    #[serde(rename = "q")]
    pub match_user: Option<String>,
    pub after: Option<Uuid>,
    pub limit: Option<i64>,
}

// Routing Methods //

/// Gets one page of users on the server.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `query` - Username to match, cursor to page from, and how many to return.
pub async fn get_users(
    AuthUser(_): AuthUser,
    State(pool): State<SqlitePool>,
    Query(query): Query<UsersQuery>,
) -> Result<Json<Vec<UserSummary>>> {

    let max = config::get().paging.users_page;
    let limit = query.limit.unwrap_or(max).clamp(1, max);

    let users = api::users::list(
        &pool,
        query.match_user,
        query.after,
        limit
    ).await?;

    Ok(Json(users))
}

/// Gets a user's public profile.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `user_id` - User to look up.
pub async fn get_user(
    AuthUser(_): AuthUser,
    State(pool): State<SqlitePool>,
    Path(user_id): Path<Uuid>,
) -> Result<Json<UserSummary>> {

    let user = api::users::get(&pool, user_id).await?;

    Ok(Json(user))
}

/// Gets the caller's own profile.
///
/// # Arguments
///
/// * `user_id` - Whose profile to return.
/// * `pool` - Pool of SQL connections.
pub async fn get_me(
    AuthUser(user_id): AuthUser,
    State(pool): State<SqlitePool>,
) -> Result<Json<UserSummary>> {

    let user = api::users::get(&pool, user_id).await?;

    Ok(Json(user))
}

/// Promotes or demotes a user.
///
/// # Arguments
///
/// * `caller_id` - Who is making the change.
/// * `pool` - Pool of SQL connections.
/// * `target_id` - User whose role changes.
/// * `body` - The role to set.
pub async fn set_role(
    AuthUser(caller_id): AuthUser,
    State(pool): State<SqlitePool>,
    Path(target_id): Path<Uuid>,
    Json(body): Json<SetRoleRequest>,
) -> Result<StatusCode> {

    api::users::set_role(&pool, caller_id, target_id, body.role).await?;

    Ok(StatusCode::NO_CONTENT)
}
