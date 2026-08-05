use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::AuthUser;
use crate::api;
use crate::error::{AppError, Result};
use crate::models::Message;

/// Messages

#[derive(Deserialize)]
pub struct PostMessageRequest {
    pub body: String,
    pub client_nonce: String,
}

#[derive(Serialize)]
pub struct MessageResponse {
    pub seq: i64,
    pub id: Uuid,
    pub room_id: Uuid,
    pub author_id: Uuid,
    pub body: Option<String>,
    pub created_at: i64,
    pub edited_at: Option<i64>,
    pub deleted_at: Option<i64>,
}

impl From<Message> for MessageResponse {
    fn from(m: Message) -> Self {
        Self {
            seq: m.seq,
            id: m.id,
            room_id: m.room_id,
            author_id: m.author_id,
            body: m.body,
            created_at: m.created_at,
            edited_at: m.edited_at,
            deleted_at: m.deleted_at,
        }
    }
}

pub async fn post_message (
    AuthUser(user_id): AuthUser,
    State(pool): State<SqlitePool>,
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
    let result = api::post_message(
        &pool,
        room_id,
        user_id,
        &body.body,
        nonce
    ).await?;

    match result {
        api::Posted::Created(msg) => Ok((StatusCode::CREATED, Json(msg.into()))),
        api::Posted::Duplicate(msg) => Ok((StatusCode::OK, Json(msg.into()))),
    }
}

#[derive(Deserialize)]
pub struct FetchQuery {
    pub after: Option<i64>,
    pub before: Option<i64>,
}

pub async fn fetch_messages (
    AuthUser(user_id): AuthUser,
    State(pool): State<SqlitePool>,
    Path(room_id): Path<Uuid>,
    Query(query): Query<FetchQuery>
) -> Result<Json<Vec<MessageResponse>>> {

    // Get http query (?before, ?after) and convert it into a cursor 
    let cursor = match (query.after, query.before) {
        (None, None) => api::Cursor::Latest,
        (Some(seq), None) => api::Cursor::After(seq),
        (None, Some(seq)) => api::Cursor::Before(seq),
        (Some(_), Some(_)) => return Err(AppError::Validation(format!("cannot use before/after")))
    };

    // Attempt to fetch messages
    let result = api::fetch_messages(&pool, user_id, room_id, cursor).await?;

    // Convert returned vector of internal messages into vector of HTTP message responses
    let mut response = Vec::with_capacity(result.len());
    for message in result {
        response.push(MessageResponse::from(message));
    }
    
    Ok(Json(response))
}
