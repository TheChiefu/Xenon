pub mod attachments;

use uuid::Uuid;

use crate::error::{AppError, Result};
use crate::models::{File, Message, Permission, Permissions};
use crate::{config, db, utils, validate};



/// Cursor for message type
/// - Latest: Give newest page
/// - After: Reconnect (everything newer than seq)
/// - Before: Scroll Up (page is older than seq)
pub enum Cursor {
    Latest,
    After(i64),
    Before(i64)
}

/// Outcome of a post. A retry carrying a nonce the server has already stored
/// returns that message rather than creating a second one.
pub enum Posted {
    Created(Message),
    Duplicate(Message),
}

/// Outcome of an edit
pub struct Edited {
    pub room_id: Uuid,
    pub edited_at: i64,
}


// Primary Methods //

/// Attempt to create a message
/// - pool: SQL Pool
/// - room_id: Where message is to be created
/// - author_id: Who is creating the message
/// - body: Contents of message
/// - client_nonce: Client's per-composition id, reused on retry
/// - attachments: Files the message carries
pub async fn post_message(
    pool: &sqlx::SqlitePool,
    room_id: Uuid,
    author_id: Uuid,
    body: Option<&str>,
    client_nonce: [u8; 16],
    attachments: &[Uuid],
) -> Result<Posted> {

    // Format checks, before any write is in flight
    validate_post(body, attachments)?;

    // Start transaction
    let mut tx = pool.begin().await?;

    // Check user perms
    let perms = db::effective_permissions(&mut *tx, author_id, room_id).await?;
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

/// Retrieves vector of messages (and possible attachments) within
/// a given cursor range and room to a specific user.
/// - pool: To get connections from
/// - user_id: Who to fetch the messages for
/// - room_id: What room to get messages from
/// - cursor: Range of messages to look for (before, after, or latest)
pub async fn fetch_messages(
    pool: &sqlx::SqlitePool,
    user_id: Uuid,
    room_id: Uuid,
    cursor: Cursor,
) -> Result<Vec<(Message, Vec<File>)>> {

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
        .bind(config::get().paging.message_page)
        .fetch_all(&mut *conn)
        .await?;

    // The descending query returns newest first, callers always get ascending
    if older { messages.reverse(); }

    // Empty message list, exit
    if messages.is_empty() {
        return Ok(Vec::new())
    }

    // Get attachments for all messages on page
    let low = messages[0].seq;
    let high = messages[messages.len() -1].seq;
    let mut pairs = attachments::for_message_range(&mut conn, room_id, low, high).await?;
    let mut result = Vec::with_capacity(messages.len());
    for message in messages {
        let files = pairs.remove(&message.id).unwrap_or_default();
        result.push((message, files));
    }

    Ok(result)
}

/// Delete a message, returning the room it was in
/// - pool: To get connections from
/// - user_id: Who is attempting to delete a message
/// - message_id: What message to delete by id
pub async fn delete_message(
    pool: &sqlx::SqlitePool,
    user_id: Uuid,
    message_id: Uuid,
) -> Result<Uuid> {
    let mut tx = pool.begin().await?;
    let message = fetch_message(&mut tx, message_id).await?;
    let room_id = message.room_id;

    // If author has permission to delete
    let perms = db::effective_permissions(&mut *tx, user_id, room_id).await?;
    if !can_delete_message(perms, user_id, message.author_id) {
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
        return Ok(room_id)
    }

    // Clear it's attachments
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

/// Edit a message, returning the room it is in and when it was edited
/// - pool: To get connections from
/// - user_id: Who is attempting to edit a message
/// - message_id: What message to edit is by id
/// - body: New message contents
pub async fn edit_message(
    pool: &sqlx::SqlitePool,
    user_id: Uuid,
    message_id: Uuid,
    body: Option<&str>,
) -> Result<Edited> {

    if let Some(text) = body {
        validate::message_body(text)?;
    }

    let mut tx = pool.begin().await?;

    // Check author
    let message = fetch_message(&mut tx, message_id).await?;
    let room_id = message.room_id;

    // Does user have permission to edit the message
    if message.author_id != user_id {
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
            return Err(AppError::Validation(err))
        }
    }

    // Update message
    let now = utils::now_ms();
    let edited = sqlx::query (
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

    // Update Mutation Seqquence
    sqlx::query("UPDATE rooms SET mutation_seq = mutation_seq + 1 WHERE id = ?1")
        .bind(room_id)
        .execute(&mut *tx)
        .await?;

    // Commit transaction
    tx.commit().await?;

    Ok(Edited { room_id, edited_at: now })

}


// Helper Methods //

/// Check for invalid post properties
/// - body: Contents of message (None when the message is attachments only)
/// - attachments: Files the message carries
fn validate_post(body: Option<&str>, attachments: &[Uuid]) -> Result<()> {

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
    for (i, i_id) in attachments.iter().enumerate() {
        for j_id in &attachments[i + 1..] {
            if i_id == j_id {
                return Err(AppError::Validation("same file attached twice".to_string()));
            }
        }
    }

    Ok(())
}

/// Reads back the message a nonce already stored
/// - conn: Connection to SQL DB
/// - author_id: Who sent it
/// - client_nonce: ID per message sent
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

/// Reads a message by id
/// - conn: Connection to SQL DB
/// - message_id: Message to look up
async fn fetch_message(
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

/// Whether a user may post the message they sent
/// - perms: The user's resolved permissions, None when they are not a member
/// - has_attachments: Whether the message carries files
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

fn can_delete_message(perms: Option<Permissions>, user_id: Uuid, author_id: Uuid) -> bool {

    let Some(perms) = perms else {
        return false;
    };

    // If the user is the same as author (self delete)
    if user_id == author_id {
        return true;
    }

    // If user is not the author (delete other's message)
    if !perms.has(Permission::DeleteMsg) {
        return false;
    }

    return true;
}