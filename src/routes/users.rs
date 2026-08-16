use axum::extract::{Path, Query, State};
use axum::Json;
use axum::http::StatusCode;
use serde::Deserialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::AuthUser;
use crate::{api, config};
use crate::error::Result;
use crate::models::{GlobalRole, UserSummary};

// Data Structs //

#[derive(Deserialize)]
pub struct SetRoleRequest {
    pub role: GlobalRole,
}

#[derive(Deserialize)]
pub struct UsersQuery {
    #[serde(rename = "q")]
    pub match_user: Option<String>,
    pub after: Option<Uuid>,
    pub limit: Option<i64>,
}

// Routing Methods //

/// Get one page of users on the server
/// - pool: Pool of SQL Connections
/// - query: Cursor to page from, and how many users to return
pub async fn get_users(
    AuthUser(_): AuthUser,
    State(pool): State<SqlitePool>,
    Query(query): Query<UsersQuery>
) -> Result<Json<Vec<UserSummary>>> {

    let max = config::get().paging.users_page;
    let limit = query.limit.unwrap_or(max).clamp(1, max);

    let users = api::users::get_users(
        &pool,
        query.match_user,
        query.after,
        limit
    ).await?;
    Ok(Json(users))

}

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
