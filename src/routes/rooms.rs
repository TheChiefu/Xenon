//! HTTP handlers for rooms and the tables scoped to a single room.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::api::rooms::RoomPatch;
use crate::api::rooms::bans::Entry as BanEntry;
use crate::api::rooms::invites::{Issued, Received};
use crate::api::rooms::members::Entry as MemberEntry;
use crate::error::Result;
use crate::models::{Permission, Permissions, Room, Visibility};
use crate::routes::AuthUser;
use crate::sockets::events::ServerEvent;
use crate::sockets::registry;
use crate::state::AppState;
use crate::{api, config};

// Data Structs //

/// POST body for creating a room.
#[derive(Deserialize)]
pub struct CreateRoomRequest {
    pub name: String,
    pub visibility: Visibility,
    pub default_permissions: Vec<Permission>,
    pub claim_all: bool,
}

/// POST body for inviting a user to a room.
#[derive(Deserialize)]
pub struct CreateRoomInvite {
    pub invitee: Uuid,
    pub expire_delta: Option<i64>
}

/// POST body for banning a user from a room.
#[derive(Deserialize)]
pub struct CreateRoomBan {
    pub target_id: Uuid,
    pub reason: Option<String>,
    pub expire_delta: Option<i64>
}

/// Response carrying the id of a newly created room.
#[derive(Serialize)]
pub struct CreateRoomResponse {
    pub id: Uuid,
}

/// Query string for a paged room listing.
#[derive(Deserialize)]
pub struct DirectoryQuery {
    pub after: Option<Uuid>,
    pub limit: Option<i64>
}

// Routing Methods //



/// Get a room's information
/// 
/// # Arguments
/// 
/// * `caller_id` - User requesting informaion.
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room to query.
pub async fn get_room(
    AuthUser(caller_id, ..): AuthUser,
    State(pool): State<SqlitePool>,
    Path(room_id): Path<Uuid>,
) -> Result<Json<Room>> {
    let room = api::rooms::get(&pool, room_id, caller_id).await?;
    Ok(Json(room))
}


/// Lists the members of a room.
///
/// # Arguments
///
/// * `caller_id` - User making the request.
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room to list from.
pub async fn list_members(
    AuthUser(caller_id, ..): AuthUser,
    State(pool): State<SqlitePool>,
    Path(room_id): Path<Uuid>,
) -> Result<Json<Vec<MemberEntry>>> {

    let members = api::rooms::members::list(&pool, room_id, caller_id).await?;

    Ok(Json(members))
}

/// Sets a user's permissions in a room.
///
/// # Arguments
///
/// * `caller_id` - Who is setting the permissions.
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room the permissions apply to.
/// * `target_id` - Whose permissions are being set.
/// * `body` - Permissions the target receives.
pub async fn set_permissions(
    AuthUser(caller_id, ..): AuthUser,
    State(pool): State<SqlitePool>,
    Path((room_id, target_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<Vec<Permission>>,
) -> Result<StatusCode> {

    let perms = Permissions::from_list(&body);
    api::rooms::members::set_permissions(&pool, room_id, caller_id, target_id, perms).await?;

    Ok(StatusCode::OK)
}




/// Invites a user to a room.
///
/// # Arguments
///
/// * `caller_id` - Who is issuing the invite.
/// * `app_state` - Pool and socket registry.
/// * `room_id` - Room being invited to.
/// * `body` - Who to invite, and how long the invite lasts.
pub async fn invite_user(
    AuthUser(caller_id, ..): AuthUser,
    State(app_state): State<AppState>,
    Path(room_id): Path<Uuid>,
    Json(body): Json<CreateRoomInvite>,
) -> Result<StatusCode> {

    api::rooms::invites::create(
        &app_state.pool,
        room_id,
        caller_id,
        body.invitee,
        body.expire_delta
    ).await?;

    let event = ServerEvent::Invited { room_id, invited_by: caller_id };
    registry::notify_user(&app_state, body.invitee, event);

    Ok(StatusCode::CREATED)
}

/// Gets the room invites the caller is a recipient of.
///
/// # Arguments
///
/// * `user_id` - User who receives the invites.
/// * `pool` - Pool of SQL connections.
pub async fn list_my_invites(
    AuthUser(user_id, ..): AuthUser,
    State(pool): State<SqlitePool>,
) -> Result<Json<Vec<Received>>> {

    let invites = api::rooms::invites::list_for_user(&pool, user_id).await?;

    Ok(Json(invites))
}

/// Gets the invites a room has issued.
///
/// # Arguments
///
/// * `caller_id` - User making the request.
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room to look in.
pub async fn list_invites(
    AuthUser(caller_id, ..): AuthUser,
    State(pool): State<SqlitePool>,
    Path(room_id): Path<Uuid>,
) -> Result<Json<Vec<Issued>>> {

    let invites = api::rooms::invites::list(&pool, room_id, caller_id).await?;

    Ok(Json(invites))
}

/// Declines a room invite.
///
/// # Arguments
///
/// * `caller_id` - Recipient of the invite.
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room the invite is to.
pub async fn decline_invite(
    AuthUser(caller_id, ..): AuthUser,
    State(pool): State<SqlitePool>,
    Path(room_id): Path<Uuid>,
) -> Result<StatusCode> {

    api::rooms::invites::decline(&pool, room_id, caller_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Withdraws an invite the room issued.
///
/// # Arguments
///
/// * `caller_id` - Who is withdrawing the invite.
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room the invite is to.
/// * `target_id` - User the invite was addressed to.
pub async fn revoke_invite(
    AuthUser(caller_id, ..): AuthUser,
    State(pool): State<SqlitePool>,
    Path((room_id, target_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode> {

    api::rooms::invites::revoke(&pool, room_id, caller_id, target_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Gets the users banned from a room.
///
/// # Arguments
///
/// * `caller_id` - User making the request.
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room to look in.
pub async fn list_bans(
    AuthUser(caller_id, ..): AuthUser,
    State(pool): State<SqlitePool>,
    Path(room_id): Path<Uuid>,
) -> Result<Json<Vec<BanEntry>>> {

    let bans = api::rooms::bans::list(&pool, room_id, caller_id).await?;

    Ok(Json(bans))
}

/// Bans a user from a room.
///
/// # Arguments
///
/// * `caller_id` - Who is issuing the ban.
/// * `app_state` - Pool and socket registry.
/// * `room_id` - Room being banned from.
/// * `body` - Who to ban, how long the ban lasts, and the reason.
pub async fn ban_user(
    AuthUser(caller_id, ..): AuthUser,
    State(app_state): State<AppState>,
    Path(room_id): Path<Uuid>,
    Json(body): Json<CreateRoomBan>,
) -> Result<StatusCode> {

    api::rooms::bans::create(
        &app_state.pool,
        room_id,
        caller_id,
        body.target_id,
        body.reason,
        body.expire_delta
    ).await?;

    let event = ServerEvent::Banned { room_id };
    registry::notify_user(&app_state, body.target_id, event);

    Ok(StatusCode::NO_CONTENT)
}

/// Lifts a user's ban on a room.
///
/// # Arguments
///
/// * `caller_id` - Who is lifting the ban.
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room being unbanned from.
/// * `target_id` - Who is being unbanned.
pub async fn unban_user(
    AuthUser(caller_id, ..): AuthUser,
    State(pool): State<SqlitePool>,
    Path((room_id, target_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode> {

    api::rooms::bans::revoke(&pool, room_id, caller_id, target_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Updates a room.
/// 
/// # Arguments
/// 
/// * `caller_id` - Who is updating the room
/// * `app_state` - Pool and socket registry
/// * `room_id` - What room is being updated
/// * `body` - Patched room information
pub async fn update(
    AuthUser(caller_id, ..): AuthUser,
    State(app_state): State<AppState>,
    Path(room_id): Path<Uuid>,
    Json(body): Json<RoomPatch>,
) -> Result<StatusCode> {

    // Update the room with the given patch
    api::rooms::update(&app_state.pool, room_id, caller_id, body).await?;

    // Notify all room members, room has updated
    let event = ServerEvent::RoomUpdated { room_id };
    registry::broadcast(&app_state, room_id, event).await;

    Ok(StatusCode::NO_CONTENT)
}

/// Creates a room.
///
/// # Arguments
///
/// * `caller_id` - The room's creator.
/// * `pool` - Pool of SQL connections.
/// * `body` - Properties to create the room with.
pub async fn create_room(
    AuthUser(caller_id, ..): AuthUser,
    State(pool): State<SqlitePool>,
    Json(body): Json<CreateRoomRequest>,
) -> Result<(StatusCode, Json<CreateRoomResponse>)> {

    let default_permissions = Permissions::from_list(&body.default_permissions);

    // Check if creator wants to inherit default permissions
    // or have full access to room (Some - Full / None - Inherit)
    let caller_permissions = body.claim_all.then_some(Permissions::ALL);

    // Attempt to create room
    let id = api::rooms::create(
        &pool,
        caller_id,
        &body.name,
        caller_permissions,
        default_permissions,
        body.visibility
    ).await?;

    Ok((StatusCode::CREATED, Json(CreateRoomResponse { id })))
}

/// Deletes a room.
///
/// # Arguments
///
/// * `caller_id` - The user making the delete request.
/// * `app_state` - Pool and socket registry.
/// * `room_id` - Room to delete.
pub async fn delete_room(
    AuthUser(caller_id, ..): AuthUser,
    State(app_state): State<AppState>,
    Path(room_id): Path<Uuid>,
) -> Result<StatusCode> {

    let members = api::rooms::delete(&app_state.pool, room_id, caller_id).await?;

    // Notify everyone who was in the room
    let event = ServerEvent::RoomDeleted { room_id };
    registry::notify_users(&app_state, &members, event);

    Ok(StatusCode::NO_CONTENT)
}

/// Joins a room.
///
/// # Arguments
///
/// * `user_id` - The user joining.
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room to join.
pub async fn join_room(
    AuthUser(user_id, ..): AuthUser,
    State(pool): State<SqlitePool>,
    Path(room_id): Path<Uuid>,
) -> Result<StatusCode> {

    api::rooms::join(&pool, room_id, user_id).await?;

    Ok(StatusCode::OK)
}

/// Leaves a room.
///
/// # Arguments
///
/// * `user_id` - The user leaving.
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room to leave.
pub async fn leave_room(
    AuthUser(user_id, ..): AuthUser,
    State(pool): State<SqlitePool>,
    Path(room_id): Path<Uuid>,
) -> Result<StatusCode> {

    api::rooms::leave(&pool, room_id, user_id).await?;

    Ok(StatusCode::OK)
}

/// Gets the rooms the caller is a member of.
///
/// # Arguments
///
/// * `user_id` - Whose rooms to list.
/// * `pool` - Pool of SQL connections.
pub async fn list_my_rooms(
    AuthUser(user_id, ..): AuthUser,
    State(pool): State<SqlitePool>,
) -> Result<Json<Vec<Room>>> {

    let rooms = api::rooms::list_mine(&pool, user_id).await?;

    Ok(Json(rooms))
}

/// Gets one page of the Public and Locked rooms on the server.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `query` - Cursor to page from, and how many rooms to return.
pub async fn list_discoverable_rooms(
    AuthUser(..): AuthUser,
    State(pool): State<SqlitePool>,
    Query(query): Query<DirectoryQuery>,
) -> Result<Json<Vec<Room>>> {

    let max = config::get().limits.room_page;
    let limit = query.limit.unwrap_or(max).clamp(1, max);

    let rooms = api::rooms::list_discoverable(&pool, query.after, limit).await?;

    Ok(Json(rooms))
}