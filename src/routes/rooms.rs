use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{AppState, AuthUser, websockets};
use crate::routes::websockets::ServerEvent;
use crate::api::rooms::invites::RoomInviteEntry;
use crate::{api, config};
use crate::error::Result;
use crate::models::{Permission, Permissions, Room, RoomInvite, Visibility};


// Data Structs //

#[derive(Deserialize)]
pub struct CreateRoomRequest { // POST body
    pub name: Option<String>,
    pub visibility: Visibility,
    pub default_permissions: Vec<Permission>,
    pub claim_all: bool,
}

#[derive(Deserialize)]
pub struct CreateRoomInvite { // POST body
    pub invitee: Uuid,
    pub expire_delta: Option<i64>
}

#[derive(Serialize)]
pub struct CreateRoomResponse {
    pub id: Uuid,
}

#[derive(Deserialize)]
pub struct DirectoryQuery {
    pub after: Option<Uuid>,
    pub limit: Option<i64>
}

// Routing Methods //

/// Create a room
/// - user_id: The room's creator
/// - pool: Pool of SQL Connections
/// - body: Properties to create the room with
pub async fn create_room(
    AuthUser(user_id): AuthUser,
    State(pool): State<SqlitePool>,
    Json(body): Json<CreateRoomRequest>
) -> Result<(StatusCode, Json<CreateRoomResponse>)> {

    // Convert permissions into single "permission" bitmask
    let mut default_permissions = Permissions::NONE;
    for permission in body.default_permissions {
        default_permissions = default_permissions.grant(permission);
    }
    
    // Check if creator wants to inherit default permissions
    // or have full access to room (Some - Full / None - Inherit)
    let creator_permissions = body.claim_all.then_some(Permissions::ALL);

    // Attempt to create room
    let id = api::rooms::create_room(
        &pool,
        user_id,
        body.name.as_deref(),
        creator_permissions,
        default_permissions,
        body.visibility
    ).await?;

    Ok((StatusCode::CREATED, Json(CreateRoomResponse {id})))
}

/// Join a room
/// - user_id: The user joining
/// - pool: Pool of SQL Connections
/// - room_id: Room to join
pub async fn join_room(
    AuthUser(user_id): AuthUser,
    State(pool): State<SqlitePool>,
    Path(room_id): Path<Uuid>
) -> Result<StatusCode> {

    api::rooms::join_room(&pool, user_id, room_id).await?;
    Ok(StatusCode::OK)
}

/// Leave a room
/// - user_id: The user leaving
/// - pool: Pool of SQL Connections
/// - room_id: Room to leave
pub async fn leave_room (
    AuthUser(user_id): AuthUser,
    State(pool): State<SqlitePool>,
    Path(room_id): Path<Uuid>
) -> Result<StatusCode> {

    api::rooms::leave_room(&pool, user_id, room_id).await?;
    Ok(StatusCode::OK)
}

/// Get the rooms the caller is a member of
/// - user_id: Whose rooms to list
/// - pool: Pool of SQL Connections
pub async fn list_my_rooms (
    AuthUser(user_id): AuthUser,
    State(pool): State<SqlitePool>,
) -> Result<Json<Vec<Room>>> {

    let rooms = api::rooms::list_my_rooms(&pool, user_id).await?;
    Ok(Json(rooms))
}

/// Get one page of the Public and Locked rooms on the server
/// - pool: Pool of SQL Connections
/// - query: Cursor to page from, and how many rooms to return
pub async fn list_discoverable_rooms (
    AuthUser(_): AuthUser,
    State(pool): State<SqlitePool>,
    Query(query): Query<DirectoryQuery>,
) -> Result<Json<Vec<Room>>> {

    let max = config::get().paging.room_page;
    let limit = query.limit.unwrap_or(max).clamp(1, max);

    let rooms = api::rooms::list_discoverable_rooms(&pool, query.after, limit).await?;
    Ok(Json(rooms))
}

/// Invite a user to a room
/// - inviter: Who is issuing the invite
/// - pool: Pool of SQL Connections
/// - room_id: Room being invited to
/// - body: Who to invite, and how long the invite lasts
pub async fn invite_user (
    AuthUser(inviter): AuthUser,
    State(app_state): State<AppState>,
    Path(room_id): Path<Uuid>,
    Json(body): Json<CreateRoomInvite>
) -> Result<StatusCode> {

    api::rooms::invites::create(
        &app_state.pool,
        room_id,
        body.invitee,
        inviter,
        body.expire_delta
    ).await?;

    let event = ServerEvent::Invited { room_id, invited_by: inviter };
    websockets::notify_user(&app_state, body.invitee, event).await;

    Ok(StatusCode::CREATED)
}

/// Get room invites the caller is a recipient of
/// - user_id: User who receives invites
/// - pool: Pool of SQL Connections
pub async fn list_my_invites(
    AuthUser(user_id): AuthUser,
    State(pool): State<SqlitePool>,
) -> Result<Json<Vec<RoomInvite>>> {

    let invites = api::rooms::invites::list_for_user(&pool, user_id).await?;
    Ok(Json(invites))

}

/// Get list of invites total in a given room
/// - user_id: User making request
/// - pool: Pool of SQL Connections
/// - room_id: Room to look in
pub async fn list_invites(
    AuthUser(user_id): AuthUser,
    State(pool): State<SqlitePool>,
    Path(room_id): Path<Uuid>,
) -> Result<Json<Vec<RoomInviteEntry>>> {

    let invites = api::rooms::invites::list_invites(&pool, user_id, room_id).await?;
    Ok(Json(invites))

}


/// Decline a room invite
/// - user_id: User making request
/// - pool: Pool of SQL Connections
/// - room_id: Relevant room
pub async fn decline(
    AuthUser(user_id): AuthUser,
    State(pool): State<SqlitePool>,
    Path(room_id): Path<Uuid>,
) -> Result<StatusCode> {

    api::rooms::invites::decline(&pool, user_id, room_id).await?;
    Ok(StatusCode::NO_CONTENT)

}

/// Withdraw an invite the room issued
/// - caller_id: User withdrawing the invite
/// - pool: Pool of SQL Connections
/// - room_id: Room the invite is to
/// - invitee: User the invite was addressed to
pub async fn revoke(
    AuthUser(caller_id): AuthUser,
    State(pool): State<SqlitePool>,
    Path((room_id, invitee)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode> {

    api::rooms::invites::revoke(&pool, caller_id, room_id, invitee).await?;
    Ok(StatusCode::NO_CONTENT)
}