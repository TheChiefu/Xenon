//! HTTP handlers for messages.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::api;
use crate::api::messages::attachments::{Attached, Incoming};
use crate::error::{AppError, Result};
use crate::models::Message;
use crate::routes::AuthUser;
use crate::sockets::events::ServerEvent;
use crate::sockets::registry;
use crate::state::AppState;

// Data Structs //

/// POST body for a new message.
#[derive(Deserialize)]
pub struct PostMessageRequest {
    pub body: Option<String>,
    pub client_nonce: String,
    #[serde(default)]
    pub attachments: Vec<Incoming>
}

/// PATCH body for editing a message.
#[derive(Deserialize)]
pub struct EditMessageRequest {
    pub body: Option<String>
}

/// One file as a message attaches it.
#[derive(Clone, Serialize)]
pub struct AttachmentResponse {
    pub id: Uuid,
    pub filename: String,
    pub mime: String,
    pub byte_size: i64,
    pub spoiler: bool,
}

impl From<Attached> for AttachmentResponse {
    fn from(attached: Attached) -> Self {
        Self {
            id: attached.file.id,
            filename: attached.file.filename,
            mime: attached.file.mime,
            byte_size: attached.file.byte_size,
            spoiler: attached.spoiler,
        }
    }
}

/// A message and the files attached to it.
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
    pub attachments: Vec<AttachmentResponse>
}

/// Response carrying when a message was edited.
#[derive(Serialize)]
pub struct EditMessageResponse {
    pub edited_at: i64
}

/// Query string selecting which page of a room's history to read.
#[derive(Deserialize)]
pub struct FetchQuery {
    pub after: Option<i64>,
    pub before: Option<i64>,
}

impl MessageResponse {

    /// Joins a stored message to the files attached to it.
    ///
    /// # Arguments
    ///
    /// * `message` - The stored message.
    /// * `attachments` - Files the message carries.
    pub fn new(message: Message, attachments: Vec<AttachmentResponse>) -> Self {
        Self {
            seq: message.seq,
            id: message.id,
            room_id: message.room_id,
            author_id: message.author_id,
            body: message.body,
            created_at: message.created_at,
            edited_at: message.edited_at,
            deleted_at: message.deleted_at,
            attachments
        }
    }
}

// Routing Methods //

/// Posts a message to a room.
///
/// # Arguments
///
/// * `author_id` - The message's author.
/// * `app_state` - Pool and socket registry.
/// * `room_id` - Room to post in.
/// * `body` - Message contents, nonce, and any attachments.
pub async fn post_message(
    AuthUser(author_id, ..): AuthUser,
    State(app_state): State<AppState>,
    Path(room_id): Path<Uuid>,
    Json(body): Json<PostMessageRequest>,
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
    let result = api::messages::post(
        &app_state.pool,
        room_id,
        author_id,
        body.body.as_deref(),
        nonce,
        &body.attachments
    ).await?;

    let (status, message) = match result {
        api::messages::Posted::Created(msg) => (StatusCode::CREATED, msg),
        api::messages::Posted::Duplicate(msg) => (StatusCode::OK, msg),
    };

    // Get all attachments in message and attach to response
    let mut conn = app_state.pool.acquire().await?;
    let files = api::messages::attachments::for_message(&mut conn, message.id).await?;
    let attachments = files.into_iter().map(AttachmentResponse::from).collect();
    let response = MessageResponse::new(message, attachments);

    // If message is posted, broadcast to all subscribed users in room
    if status == StatusCode::CREATED {
        let event = ServerEvent::Message { room_id, message: response.clone() };
        registry::broadcast(&app_state, room_id, event).await;
    }

    // Message is duplicate (no broadcast)
    Ok((status, Json(response)))
}

/// Gets one page of a room's messages.
///
/// # Arguments
///
/// * `user_id` - Who the messages are fetched for.
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room to read from.
/// * `query` - Cursor to page from.
pub async fn fetch_messages(
    AuthUser(user_id, ..): AuthUser,
    State(pool): State<SqlitePool>,
    Path(room_id): Path<Uuid>,
    Query(query): Query<FetchQuery>,
) -> Result<Json<Vec<MessageResponse>>> {

    // Get http query (?before, ?after) and convert it into a cursor
    let cursor = match (query.after, query.before) {
        (None, None) => api::messages::Cursor::Latest,
        (Some(seq), None) => api::messages::Cursor::After(seq),
        (None, Some(seq)) => api::messages::Cursor::Before(seq),
        (Some(_), Some(_)) => {
            return Err(AppError::Validation("cannot use before/after".to_string()))
        }
    };

    // Attempt to fetch messages
    let result = api::messages::fetch(&pool, room_id, user_id, cursor).await?;

    // Convert returned vector of internal messages into vector of HTTP message responses
    let mut response = Vec::with_capacity(result.len());
    for (message, files) in result {
        let attachments = files.into_iter().map(AttachmentResponse::from).collect();
        response.push(MessageResponse::new(message, attachments));
    }

    Ok(Json(response))
}

/// Deletes a message.
///
/// # Arguments
///
/// * `caller_id` - Who is deleting the message.
/// * `app_state` - Pool and socket registry.
/// * `message_id` - Message to delete.
pub async fn delete_message(
    AuthUser(caller_id, ..): AuthUser,
    State(app_state): State<AppState>,
    Path(message_id): Path<Uuid>,
) -> Result<StatusCode> {

    let room_id = api::messages::delete(&app_state.pool, message_id, caller_id).await?;

    let event = ServerEvent::MessageDeleted { room_id, message_id };
    registry::broadcast(&app_state, room_id, event).await;

    Ok(StatusCode::NO_CONTENT)
}

/// Edits a message.
///
/// # Arguments
///
/// * `caller_id` - Who is editing the message.
/// * `app_state` - Pool and socket registry.
/// * `message_id` - Message to edit.
/// * `request` - The new body.
pub async fn update_message(
    AuthUser(caller_id, ..): AuthUser,
    State(app_state): State<AppState>,
    Path(message_id): Path<Uuid>,
    Json(request): Json<EditMessageRequest>,
) -> Result<Json<EditMessageResponse>> {

    let result = api::messages::edit(
        &app_state.pool,
        message_id,
        caller_id,
        request.body.as_deref(),
    ).await?;

    let event = ServerEvent::MessageEdited {
        room_id: result.room_id,
        message_id,
        body: request.body,
        edited_at: result.edited_at
    };
    registry::broadcast(&app_state, result.room_id, event).await;

    Ok(Json(EditMessageResponse { edited_at: result.edited_at }))
}
