use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::api;
use crate::error::{AppError, Result};
use crate::models::Message;
use crate::routes::files::FileResponse;
use crate::routes::websockets::ServerEvent;
use crate::routes::{AuthUser, AppState, websockets};

// Data Structs & Implementations //

#[derive(Deserialize)]
pub struct PostMessageRequest {
    pub body: Option<String>,
    #[serde(default)]
    pub spoiler: bool,
    pub client_nonce: String,
    #[serde(default)]
    pub attachments: Vec<Uuid>
}

#[derive(Clone, Serialize)]
pub struct MessageResponse {
    pub seq: i64,
    pub id: Uuid,
    pub room_id: Uuid,
    pub author_id: Uuid,
    pub body: Option<String>,
    pub created_at: i64,
    pub edited_at: Option<i64>,
    pub deleted_at: Option<i64>,
    pub spoiler: bool,
    pub attachments: Vec<FileResponse>
}

impl MessageResponse {

    pub fn new(m: Message, attachments: Vec<FileResponse>) -> Self {
        Self {
            seq: m.seq,
            id: m.id,
            room_id: m.room_id,
            author_id: m.author_id,
            body: m.body,
            created_at: m.created_at,
            edited_at: m.edited_at,
            deleted_at: m.deleted_at,
            spoiler: m.spoiler,
            attachments
        }
    }
}

// Routing Methods //

/// Post a message to a room
/// - AuthUser: The message's author
/// - app_state: Pool and socket registry
/// - room_id: Room to post in
/// - body: Message contents, nonce, and any attachments
pub async fn post_message (
    AuthUser(user_id): AuthUser,
    State(app_state): State<AppState>,
    Path(room_id): Path<Uuid>,
    Json(body): Json<PostMessageRequest>
) -> Result<(StatusCode, Json<MessageResponse>)> {

    // Check for malformed nonce (check if hex)
    let bytes = match hex::decode(&body.client_nonce) {
        Ok(bytes) => bytes,
        Err(_) => return Err(AppError::Validation("client_nonce must be hex".to_string()))
    };

    // Check nonce byte count
    let nonce: [u8; 16] = match bytes.try_into() {
        Ok(array) => array,
        Err(_) => return Err(AppError::Validation("client_nonce must be 16 bytes".to_string()))
    };

    // Attempt to post a message
    let result = api::messages::post_message(
        &app_state.pool,
        room_id,
        user_id,
        body.body.as_deref(),
        body.spoiler,
        nonce,
        &body.attachments
    ).await?;

    let (status, message) = match result {
        api::messages::Posted::Created(msg) => (StatusCode::CREATED, msg),
        api::messages::Posted::Duplicate(msg) => (StatusCode::OK, msg),
    };

    // Get all attachments in message and attach to response
    let mut conn = app_state.pool.acquire().await?;
    let files = api::messages::attachments::for_message(&mut *conn, message.id).await?;
    let attachments = files.into_iter().map(FileResponse::from).collect();
    let response = MessageResponse::new(message, attachments);

    // If message is posted, broadcast to all subscribed users in room
    if status == StatusCode::CREATED {
        let event = ServerEvent::Message { room_id, message: response.clone() };
        websockets::broadcast(&app_state, room_id, event).await;
    }

    // Message is duplicate (no broadcast)
    Ok((status, Json(response)))
}

// Fetch Messages //

#[derive(Deserialize)]
pub struct FetchQuery {
    pub after: Option<i64>,
    pub before: Option<i64>,
}

/// Get one page of a room's messages
/// - AuthUser: Who the messages are fetched for
/// - pool: Pool of SQL Connections
/// - room_id: Room to read from
/// - query: Cursor to page from
pub async fn fetch_messages (
    AuthUser(user_id): AuthUser,
    State(pool): State<SqlitePool>,
    Path(room_id): Path<Uuid>,
    Query(query): Query<FetchQuery>
) -> Result<Json<Vec<MessageResponse>>> {

    // Get http query (?before, ?after) and convert it into a cursor 
    let cursor = match (query.after, query.before) {
        (None, None) => api::messages::Cursor::Latest,
        (Some(seq), None) => api::messages::Cursor::After(seq),
        (None, Some(seq)) => api::messages::Cursor::Before(seq),
        (Some(_), Some(_)) => return Err(AppError::Validation(format!("cannot use before/after")))
    };

    // Attempt to fetch messages
    let result = api::messages::fetch_messages(&pool, user_id, room_id, cursor).await?;

    // Convert returned vector of internal messages into vector of HTTP message responses
    let mut response = Vec::with_capacity(result.len());
    for (message, files) in result {
        let attachments = files.into_iter().map(FileResponse::from).collect();
        response.push(MessageResponse::new(message, attachments));
    }
    
    Ok(Json(response))
}

/// Delete a message
/// - AuthUser: Who is deleting
/// - app_state: Pool and socket registry
/// - message_id: Message to delete
pub async fn delete_message (
    AuthUser(user_id): AuthUser,
    State(app_state): State<AppState>,
    Path(message_id): Path<Uuid>
) -> Result<StatusCode> {
    let room_id = api::messages::delete_message(&app_state.pool, user_id, message_id).await?;

    let event = ServerEvent::MessageDeleted { room_id, message_id };
    websockets::broadcast(&app_state, room_id, event).await;

    Ok(StatusCode::NO_CONTENT)
}

//  Edit Messages //

#[derive(Deserialize)]
pub struct EditMessageRequest {
    pub body: Option<String>
}

#[derive(Serialize)]
pub struct EditMessageResponse {
    pub edited_at: i64
}

/// Edit a message (delta update)
/// - AuthUser: Who is editing
/// - app_state: Pool and socket registry
/// - message_id: Message to edit
/// - request: The new body
pub async fn update_message (
    AuthUser(user_id): AuthUser,
    State(app_state): State<AppState>,
    Path(message_id): Path<Uuid>,
    Json(request): Json<EditMessageRequest>
) -> Result<Json<EditMessageResponse>> {

    let result = api::messages::edit_message(
        &app_state.pool,
        user_id,
        message_id,
        request.body.as_deref(),
    ).await?;

    let event = ServerEvent::MessageEdited {
        room_id: result.room_id,
        message_id: message_id,
        body: request.body,
        edited_at: result.edited_at
    };
    websockets::broadcast(&app_state, result.room_id, event).await;

    Ok(Json(EditMessageResponse { edited_at: result.edited_at }))

}