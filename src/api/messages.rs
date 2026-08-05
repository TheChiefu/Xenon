use uuid::Uuid;

use crate::error::{AppError, Result};
use crate::models::{Message, Permission};
use crate::{db, utils, validate};


/// Outcome of a post. A retry carrying a nonce the server has already stored
/// returns that message rather than creating a second one.
pub enum Posted {
    Created(Message),
    Duplicate(Message),
}

/// Attempt to create a message
/// - pool: SQL Pool
/// - room_id: Where message is to be created
/// - author_id: Who is creating the message
/// - body: Contents of message
/// - client_nonce: Client's per-composition id, reused on retry
pub async fn post_message(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    author_id: Uuid,
    body: &str,
    client_nonce: [u8; 16]
) -> Result<Posted> {

    // Format validation, before any write is in flight
    validate::message_body(body)?;

    let mut tx = pool.begin().await?;

    // Check if user has permission to post in room
    // None is not a member, which cannot read the room either
    let perms = db::effective_permissions(&mut *tx, author_id, room_id).await?;
    match perms {
        Some(val) => {
            if !val.has(Permission::Post) {
                return Err(AppError::Forbidden);
            }
        }
        None => {
            return Err(AppError::Forbidden);
        }
    }

    // Attempt to create new message. mutation_seq is deliberately not bumped,
    // a new message already carries a new seq for clients to find
    let message_id = Uuid::now_v7();
    let now = utils::now_ms();
    let result = sqlx::query(
        "
        INSERT INTO messages (id, room_id, author_id, body, client_nonce, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT (author_id, client_nonce) DO NOTHING
        "
    )
    .bind(message_id)
    .bind(room_id)
    .bind(author_id)
    .bind(body)
    .bind(client_nonce.as_slice())
    .bind(now)
    .execute(&mut *tx)
    .await?;

    // Nothing written means this nonce is already stored, the client resent
    let posted = if result.rows_affected() == 0 {

        // Send the stored message back rather than creating a second one
        let existing = sqlx::query_as::<_, Message>(
            "
            SELECT seq, id, room_id, author_id, body, created_at, edited_at, deleted_at
            FROM messages
            WHERE author_id = ?1 AND client_nonce = ?2
            "
        )
        .bind(author_id)
        .bind(client_nonce.as_slice())
        .fetch_one(&mut *tx)
        .await?;

        Posted::Duplicate(existing)

    } else {
        Posted::Created(Message {
            seq: result.last_insert_rowid(), // seq is INTEGER PRIMARY KEY
            id: message_id,
            room_id,
            author_id,
            body: Some(body.to_string()),
            created_at: now,
            edited_at: None,
            deleted_at: None
        })
    };

    tx.commit().await?;

    Ok(posted)
}


/// Cursor for message type
/// - Latest: Give newest page
/// - After: Reconnect (everything newer than seq)
/// - Before: Scroll Up (page is older than seq)
pub enum Cursor {
    Latest,
    After(i64),
    Before(i64)
}

const MESSAGE_PAGE: i64 = 200;
pub async fn fetch_messages(
    pool: &sqlx::SqlitePool,
    user_id: Uuid,
    room_id: Uuid,
    cursor: Cursor,
) -> Result<Vec<Message>> {

    let mut conn = pool.acquire().await?;

    // Check if user has permission to access the room
    let perms = db::effective_permissions(&mut *conn, user_id, room_id).await?;
    if perms.is_none() {
        return Err(AppError::Forbidden);
    }

    // anchor is the seq to page from, older is the direction
    let (anchor, older) = match cursor {
        Cursor::Latest => (i64::MAX, true), // i64::MAX is above existing seq (returns the newest page)
        Cursor::Before(seq) => (seq, true),
        Cursor::After(seq) => (seq, false)
    };

    // Two directions from the anchor: below it descending, above it ascending
    let query = if older {
        "
        SELECT seq, id, room_id, author_id, body, created_at, edited_at, deleted_at
        FROM messages
        WHERE room_id = ?1 AND seq < ?2 AND deleted_at IS NULL
        ORDER BY seq DESC
        LIMIT ?3
        "
    } else {
        "
        SELECT seq, id, room_id, author_id, body, created_at, edited_at, deleted_at
        FROM messages
        WHERE room_id = ?1 AND seq > ?2 AND deleted_at IS NULL
        ORDER BY seq
        LIMIT ?3
        "
    };

    // Fetch messages and insert into vector
    let mut messages: Vec<Message> = sqlx::query_as(query)
        .bind(room_id)
        .bind(anchor)
        .bind(MESSAGE_PAGE)
        .fetch_all(&mut *conn)
        .await?;

    // The descending query returns newest first, callers always get ascending
    if older { messages.reverse(); }

    Ok(messages)
}
