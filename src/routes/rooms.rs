use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::AuthUser;
use crate::api;
use crate::error::Result;
use crate::models::{Permission, Permissions, Room, Visibility};

// Rooms

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
    let id = api::create_room(
        &pool,
        user_id,
        body.name.as_deref(),
        creator_permissions,
        default_permissions,
        body.visibility
    ).await?;

    Ok((StatusCode::CREATED, Json(CreateRoomResponse {id})))
}

pub async fn join_room(
    AuthUser(user_id): AuthUser,
    State(pool): State<SqlitePool>,
    Path(room_id): Path<Uuid>
) -> Result<StatusCode> {

    api::join_room(&pool, user_id, room_id).await?;
    Ok(StatusCode::OK)
}

pub async fn leave_room (
    AuthUser(user_id): AuthUser,
    State(pool): State<SqlitePool>,
    Path(room_id): Path<Uuid>
) -> Result<StatusCode> {

    api::leave_room(&pool, user_id, room_id).await?;
    Ok(StatusCode::OK)
}

pub async fn list_rooms (
    AuthUser(user_id): AuthUser,
    State(pool): State<SqlitePool>,
) -> Result<Json<Vec<Room>>> {

    let rooms = api::list_rooms(&pool, user_id).await?;
    Ok(Json(rooms))
}
