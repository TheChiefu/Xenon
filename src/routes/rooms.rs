use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::AuthUser;
use crate::{api, config};
use crate::error::Result;
use crate::models::{Permission, Permissions, Room, Visibility};


// Data Structs //

#[derive(Deserialize)]
pub struct CreateRoomRequest {
    pub name: Option<String>,
    pub visibility: Visibility,
    pub default_permissions: Vec<Permission>,
    pub claim_all: bool,
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
/// - AuthUser: The room's creator
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
/// - AuthUser: The user joining
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
/// - AuthUser: The user leaving
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
/// - AuthUser: Whose rooms to list
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
