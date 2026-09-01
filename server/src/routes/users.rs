//! HTTP handlers for users.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::api::users::{ProfilePatch, UserProfile};
use crate::db;
use crate::error::Result;
use crate::models::{GlobalRole, Status, UserSummary};
use crate::routes::AuthUser;
use crate::sockets::events::ServerEvent;
use crate::sockets::{presence, registry};
use crate::state::AppState;
use crate::{api, config};

// Data Structs //

/// PATCH body for changing a user's global role.
#[derive(Deserialize)]
pub struct SetRoleRequest {
    pub role: GlobalRole,
}

/// PATCH body for replacing a password.
#[derive(Deserialize)]
pub struct PasswordRequest {
    pub current_password: String,
    pub new_password: String,

    /// Revokes every session but the one making the request
    #[serde(default)]
    pub revoke_others: bool,
}

/// POST body for handing the server to another account.
#[derive(Deserialize)]
pub struct TransferOwnershipRequest {
    pub user_id: Uuid,

    /// Role the outgoing Owner keeps
    pub demote_to: GlobalRole,
}

/// DELETE body for closing your own account.
#[derive(Deserialize)]
pub struct DeleteAccountRequest {
    /// Replaces the names and releases the username
    #[serde(default)]
    pub anonymize: bool,

    /// Tombstones every message the account wrote
    #[serde(default)]
    pub delete_history: bool,
}

/// PUT body for the status a user's connections start at.
#[derive(Deserialize)]
pub struct StatusRequest {
    pub status: Status
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
    AuthUser(..): AuthUser,
    State(pool): State<SqlitePool>,
    Query(query): Query<UsersQuery>,
) -> Result<Json<Vec<UserSummary>>> {

    let max = config::get().limits.users_page;
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
    AuthUser(..): AuthUser,
    State(pool): State<SqlitePool>,
    Path(user_id): Path<Uuid>,
) -> Result<Json<UserProfile>> {

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
    AuthUser(user_id, ..): AuthUser,
    State(pool): State<SqlitePool>,
) -> Result<Json<UserProfile>> {

    let user = api::users::get(&pool, user_id).await?;

    Ok(Json(user))
}

/// Writes the caller's own profile.
///
/// # Arguments
///
/// * `user_id` - Whose profile is being written.
/// * `app_state` - Pool and socket registry.
/// * `body` - Fields to change.
pub async fn update_me(
    AuthUser(user_id, ..): AuthUser,
    State(app_state): State<AppState>,
    Json(body): Json<ProfilePatch>,
) -> Result<StatusCode> {

    // Profile as stored, or None if no such user
    let updated = api::users::update(&app_state.pool, user_id, body).await?;

    // Notify everyone sharing a room of the new name and pictures
    if let Some(profile) = updated {
        let mut conn = app_state.pool.acquire().await?;
        let members = db::shared_room_member_ids(&mut conn, user_id).await?;

        let event = ServerEvent::ProfileUpdated {
            user_id,
            display_name: profile.display_name,
            description: profile.description,
            avatar_file_id: profile.avatar_file_id,
            banner_file_id: profile.banner_file_id,
        };
        registry::inform_users(&app_state, &members, event);
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Writes the status the caller's connections start at, and applies it to the
/// ones they hold now.
///
/// # Arguments
///
/// * `user_id` - Whose status is being written.
/// * `app_state` - Pool and socket registry.
/// * `body` - Status to store.
pub async fn update_my_status(
    AuthUser(user_id, ..): AuthUser,
    State(app_state): State<AppState>,
    Json(body): Json<StatusRequest>,
) -> Result<StatusCode> {

    api::users::set_preferred_status(&app_state.pool, user_id, body.status).await?;

    // A status comes back only when the caller holds a connection to change
    if let Some(previous) = registry::set_status(&app_state, user_id, body.status) {
        presence::on_change(&app_state, user_id, Some(previous), Some(body.status)).await;
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Replaces the caller's password.
///
/// # Arguments
///
/// * `user_id` - Whose password is being replaced.
/// * `session_hash` - The caller's own session, kept when revoking the rest.
/// * `pool` - Pool of SQL connections.
/// * `body` - Current password, replacement, and whether to revoke elsewhere.
pub async fn update_my_password(
    AuthUser(user_id, session_hash): AuthUser,
    State(pool): State<SqlitePool>,
    Json(body): Json<PasswordRequest>,
) -> Result<StatusCode> {

    api::auth::change_password(
        &pool,
        user_id,
        &body.current_password,
        &body.new_password,
        body.revoke_others,
        &session_hash,
    ).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Hands the server to another account.
///
/// # Arguments
///
/// * `caller_id` - The Owner giving the server away.
/// * `pool` - Pool of SQL connections.
/// * `body` - Account receiving Owner, and the role the caller keeps.
pub async fn transfer_ownership(
    AuthUser(caller_id, ..): AuthUser,
    State(pool): State<SqlitePool>,
    Json(body): Json<TransferOwnershipRequest>,
) -> Result<StatusCode> {

    api::users::transfer_ownership(&pool, caller_id, body.user_id, body.demote_to).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Closes the caller's own account.
///
/// # Arguments
///
/// * `user_id` - Account being closed.
/// * `app_state` - Pool and socket registry.
/// * `body` - Whether to anonymize the names and whether to drop the history.
pub async fn delete_me(
    AuthUser(user_id, ..): AuthUser,
    State(app_state): State<AppState>,
    Json(body): Json<DeleteAccountRequest>,
) -> Result<StatusCode> {

    let rooms = api::users::delete(
        &app_state.pool,
        user_id,
        body.anonymize,
        body.delete_history
    ).await?;

    broadcast_member_left(&app_state, user_id, rooms).await;

    Ok(StatusCode::NO_CONTENT)
}

/// Closes someone else's account.
///
/// # Arguments
///
/// * `caller_id` - Who is closing the account.
/// * `app_state` - Pool and socket registry.
/// * `target_id` - Account being closed.
/// * `body` - Whether to anonymize the names.
pub async fn delete_user(
    AuthUser(caller_id, ..): AuthUser,
    State(app_state): State<AppState>,
    Path(target_id): Path<Uuid>,
    Json(body): Json<DeleteAccountRequest>,
) -> Result<StatusCode> {

    let rooms = api::users::delete_other(
        &app_state.pool,
        caller_id,
        target_id,
        body.anonymize
    ).await?;

    broadcast_member_left(&app_state, target_id, rooms).await;

    Ok(StatusCode::NO_CONTENT)
}

/// Tells each of several rooms that a member is no longer in it.
///
/// # Arguments
///
/// * `app_state` - Pool and socket registry.
/// * `user_id` - Member that was removed.
/// * `rooms` - Rooms the membership was removed from.
async fn broadcast_member_left(
    app_state: &AppState,
    user_id: Uuid,
    rooms: Vec<Uuid>,
) {
    for room_id in rooms {
        let event = ServerEvent::MemberLeft { room_id, user_id };
        registry::broadcast(app_state, room_id, event).await;
    }
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
    AuthUser(caller_id, ..): AuthUser,
    State(pool): State<SqlitePool>,
    Path(target_id): Path<Uuid>,
    Json(body): Json<SetRoleRequest>,
) -> Result<StatusCode> {

    api::users::set_role(&pool, caller_id, target_id, body.role).await?;

    Ok(StatusCode::NO_CONTENT)
}

