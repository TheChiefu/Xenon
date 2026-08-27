//! The `messages` table.

pub mod attachments;

use uuid::Uuid;

use crate::api::messages::attachments::{Attached, Incoming};
use crate::config;
use crate::db;
use crate::error::{AppError, Result};
use crate::models::{Message, Permission, Permissions};
use crate::utils;
use crate::validate;

// Data Structs //

/// Which page of a room's history to read.
pub enum Cursor {
    /// Newest page.
    Latest,
    /// Everything newer than the given seq, used on reconnect.
    After(i64),
    /// The page older than the given seq, used when scrolling up.
    Before(i64)
}

/// Outcome of a post. A retry carrying a nonce the server has already stored
/// returns that message rather than creating a second one.
pub enum Posted {
    Created(Message),
    Duplicate(Message),
}

/// Outcome of an edit.
pub struct Edited {
    pub room_id: Uuid,
    pub edited_at: i64,
}

// API Methods //

/// Posts a message to a room.
///
/// A retry carrying a `client_nonce` the server already stored returns the
/// stored message instead of writing a second one.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room the message is posted to.
/// * `author_id` - Who is posting.
/// * `body` - Message contents, `None` when the message is attachments only.
/// * `client_nonce` - Client's per-composition id, reused on retry.
/// * `attachments` - Files the message carries.
///
/// # Errors
///
/// Returns `AppError::Validation` if the message has neither body nor
/// attachments, exceeds a length or count limit, or repeats a file id, and
/// `AppError::Forbidden` if the author lacks `Permission::Post`, or
/// `Permission::Attach` on a message carrying files.
pub async fn post(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    author_id: Uuid,
    body: Option<&str>,
    client_nonce: [u8; 16],
    attachments: &[Incoming],
) -> Result<Posted> {

    // Format checks, before the transaction opens
    validate_post(body, attachments)?;

    // Start transaction
    let mut tx = pool.begin().await?;

    // Check user perms
    let perms = db::effective_permissions(&mut tx, room_id, author_id).await?;
    if !can_post(perms, !attachments.is_empty()) {
        return Err(AppError::Forbidden);
    }

    // Attempt to create new message
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

    // Nothing written means this nonce is already stored (client resent)
    let posted = if result.rows_affected() == 0 {

        // Send the stored message back rather than creating a new one
        let existing = fetch_by_nonce(&mut tx, author_id, client_nonce).await?;
        Posted::Duplicate(existing)

    } else {

        attachments::insert(&mut tx, message_id, attachments).await?;

        // Return newly created message as result
        Posted::Created(Message {
            seq: result.last_insert_rowid(),
            id: message_id,
            room_id,
            author_id,
            body: body.map(str::to_string),
            created_at: now,
            edited_at: None,
            deleted_at: None
        })
    };

    // Complete transaction
    tx.commit().await?;

    Ok(posted)
}

/// Reads one page of a room's messages, each with the files attached to it.
///
/// Messages are always returned oldest first, whichever direction the cursor
/// paged in.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `room_id` - Room to read from.
/// * `user_id` - Who the messages are fetched for.
/// * `cursor` - Which page to read.
///
/// # Errors
///
/// Returns `AppError::Forbidden` if the user is not a member of the room.
pub async fn fetch(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    user_id: Uuid,
    cursor: Cursor,
) -> Result<Vec<(Message, Vec<Attached>)>> {

    let mut conn = pool.acquire().await?;

    // Check if user has permission to access the room
    let perms = db::effective_permissions(&mut conn, room_id, user_id).await?;
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
    .bind(config::get().limits.message_page)
    .fetch_all(&mut *conn)
    .await?;

    // The descending query returns newest first, callers always get ascending
    if older {
        messages.reverse();
    }

    // Empty message list, exit
    if messages.is_empty() {
        return Ok(Vec::new());
    }

    // Get attachments for all messages on page
    let low = messages[0].seq;
    let high = messages[messages.len() - 1].seq;
    let mut pairs = attachments::for_message_range(&mut conn, room_id, low, high).await?;
    let mut result = Vec::with_capacity(messages.len());
    for message in messages {
        let files = pairs.remove(&message.id).unwrap_or_default();
        result.push((message, files));
    }

    Ok(result)
}

/// Tombstones a message, returning the room it was in.
///
/// The row stays, with its body and attachments cleared.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `message_id` - Message to delete.
/// * `caller_id` - Who is deleting it.
///
/// # Errors
///
/// Returns `AppError::NotFound` if no such message exists, and
/// `AppError::Forbidden` if the caller is neither the author nor a holder of
/// `Permission::DeleteMsg`.
pub async fn delete(
    pool: &sqlx::SqlitePool,
    message_id: Uuid,
    caller_id: Uuid,
) -> Result<Uuid> {

    let mut tx = pool.begin().await?;
    let message = fetch_by_id(&mut tx, message_id).await?;
    let room_id = message.room_id;

    // If caller has permission to delete
    let perms = db::effective_permissions(&mut tx, room_id, caller_id).await?;
    if !can_delete(perms, caller_id, message.author_id) {
        return Err(AppError::Forbidden);
    }

    // Tombstone the message
    let now = utils::now_ms();
    let tombstone_msg = sqlx::query(
        "
        UPDATE messages
        SET body = NULL, deleted_at = ?1
        WHERE id = ?2 AND deleted_at IS NULL
        "
    )
    .bind(now)
    .bind(message_id)
    .execute(&mut *tx)
    .await?;

    // Message already deleted, so nothing below should run
    if tombstone_msg.rows_affected() == 0 {
        return Ok(room_id);
    }

    // Clear its attachments
    sqlx::query("DELETE FROM message_attachments WHERE message_id = ?1")
    .bind(message_id)
    .execute(&mut *tx)
    .await?;

    // Bump the room
    sqlx::query("UPDATE rooms SET mutation_seq = mutation_seq + 1 WHERE id = ?1")
    .bind(room_id)
    .execute(&mut *tx)
    .await?;

    // Finish Transaction
    tx.commit().await?;

    Ok(room_id)
}

/// Replaces a message body, returning the room it is in and when it was edited.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `message_id` - Message to edit.
/// * `caller_id` - Who is editing it.
/// * `body` - New message contents, `None` to leave attachments alone.
///
/// # Errors
///
/// Returns `AppError::Validation` if the new body is over the length limit, or
/// is absent on a message that carries no attachments; `AppError::Forbidden` if
/// the caller is not the author; and `AppError::NotFound` if the message does
/// not exist or is already a tombstone.
pub async fn edit(
    pool: &sqlx::SqlitePool,
    message_id: Uuid,
    caller_id: Uuid,
    body: Option<&str>,
) -> Result<Edited> {

    if let Some(text) = body {
        validate::message_body(text)?;
    }

    let mut tx = pool.begin().await?;

    // Check author
    let message = fetch_by_id(&mut tx, message_id).await?;
    let room_id = message.room_id;

    // Does caller have permission to edit the message
    if message.author_id != caller_id {
        return Err(AppError::Forbidden);
    }

    // If there is no body, verify attachments are present
    if body.is_none() {

        // Check if original message has attachments
        let has_attachments: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM message_attachments WHERE message_id = ?1)"
        )
        .bind(message_id)
        .fetch_one(&mut *tx)
        .await?;

        if !has_attachments {
            let err = "edited message body cannot be empty and have no attachments".to_string();
            return Err(AppError::Validation(err));
        }
    }

    // Update message
    let now = utils::now_ms();
    let edited = sqlx::query(
        "
        UPDATE messages
        SET body = ?1, edited_at = ?2
        WHERE id = ?3 AND deleted_at IS NULL
        "
    )
    .bind(body)
    .bind(now)
    .bind(message_id)
    .execute(&mut *tx)
    .await?;

    // The guard above matched nothing, so the message is already a tombstone
    if edited.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    // Update mutation sequence
    sqlx::query("UPDATE rooms SET mutation_seq = mutation_seq + 1 WHERE id = ?1")
    .bind(room_id)
    .execute(&mut *tx)
    .await?;

    // Commit transaction
    tx.commit().await?;

    Ok(Edited { room_id, edited_at: now })
}

// Helper Methods //

/// Checks a post's body and attachment count.
///
/// # Arguments
///
/// * `body` - Message contents, `None` when the message is attachments only.
/// * `attachments` - Files the message carries.
///
/// # Errors
///
/// Returns `AppError::Validation` if the message has neither body nor
/// attachments, carries more attachments than the configured limit, repeats a
/// file id, or has a body over the length limit.
fn validate_post(body: Option<&str>, attachments: &[Incoming]) -> Result<()> {

    // No body or attachments
    if body.is_none() && attachments.is_empty() {
        return Err(AppError::Validation("No message body or attachments provided".to_string()));
    }

    // Over allowed attachment limit
    let max_attachments = config::get().limits.attachments_per_message_max;
    if attachments.len() > max_attachments {
        let err = format!("too many attachments, max allowed: {max_attachments}");
        return Err(AppError::Validation(err));
    }

    // Validate message body before using
    if let Some(text) = body {
        validate::message_body(text)?;
    }

    // Check for duplicate file IDs
    for (i, first) in attachments.iter().enumerate() {
        for second in &attachments[i + 1..] {
            if first.file_id == second.file_id {
                return Err(AppError::Validation("same file attached twice".to_string()));
            }
        }
    }

    Ok(())
}

/// Reads back the message a nonce already stored.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `author_id` - Who sent it.
/// * `client_nonce` - Client's per-composition id.
async fn fetch_by_nonce(
    conn: &mut sqlx::SqliteConnection,
    author_id: Uuid,
    client_nonce: [u8; 16],
) -> Result<Message> {

    let message = sqlx::query_as::<_, Message>(
        "
        SELECT seq, id, room_id, author_id, body, created_at, edited_at, deleted_at
        FROM messages
        WHERE author_id = ?1 AND client_nonce = ?2
        "
    )
    .bind(author_id)
    .bind(client_nonce.as_slice())
    .fetch_one(&mut *conn)
    .await?;

    Ok(message)
}

/// Reads a message by id.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `message_id` - Message to look up.
///
/// # Errors
///
/// Returns `AppError::NotFound` if no such message exists.
async fn fetch_by_id(
    conn: &mut sqlx::SqliteConnection,
    message_id: Uuid,
) -> Result<Message> {

    let message: Option<Message> = sqlx::query_as(
        "
        SELECT seq, id, room_id, author_id, body, created_at, edited_at, deleted_at
        FROM messages
        WHERE id = ?1
        "
    )
    .bind(message_id)
    .fetch_optional(&mut *conn)
    .await?;

    let Some(message) = message else {
        return Err(AppError::NotFound);
    };

    Ok(message)
}

/// Reports whether a user may post the message they sent.
///
/// # Arguments
///
/// * `perms` - The user's resolved permissions, `None` when they are not a member.
/// * `has_attachments` - Whether the message carries files.
fn can_post(perms: Option<Permissions>, has_attachments: bool) -> bool {

    // Not a member, so the room cannot be read either
    let Some(perms) = perms else {
        return false;
    };

    // Check if they can post text messages
    if !perms.has(Permission::Post) {
        return false;
    }

    // Check if they can post attachments (if any are there)
    if has_attachments && !perms.has(Permission::Attach) {
        return false;
    }

    true
}

/// Reports whether a user may delete a message.
///
/// # Arguments
///
/// * `perms` - The caller's resolved permissions, `None` when they are not a member.
/// * `caller_id` - Who is deleting the message.
/// * `author_id` - Who wrote the message.
fn can_delete(perms: Option<Permissions>, caller_id: Uuid, author_id: Uuid) -> bool {

    // Not a member, so the room cannot be read
    let Some(perms) = perms else {
        return false;
    };

    // Authors can always delete their own messages
    if caller_id == author_id {
        return true;
    }

    // Deleting someone else's message takes an explicit permission
    perms.has(Permission::DeleteMsg)
}
