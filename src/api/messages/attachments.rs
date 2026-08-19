//! The `message_attachments` table: which files hang off which message.

use std::collections::HashMap;

use uuid::Uuid;

use crate::error::{AppError, Result};
use crate::models::File;

// Data Structs //

/// A file joined to the message it is attached to.
///
/// One query covers many messages, so every row carries its `message_id`.
#[derive(sqlx::FromRow)]
struct AttachmentRow {
    message_id: Uuid,
    #[sqlx(flatten)]
    file: File,
}

// API Methods //

/// Links files to a message, ordered as the client sent them.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `message_id` - Message the files belong to.
/// * `attachments` - Files the message carries.
///
/// # Errors
///
/// Returns `AppError::Validation` if an id names a file that is not stored.
pub async fn insert(
    conn: &mut sqlx::SqliteConnection,
    message_id: Uuid,
    attachments: &[Uuid],
) -> Result<()> {

    let sql = "INSERT INTO message_attachments (message_id, file_id, ordinal) VALUES (?1, ?2, ?3)";

    // Iterate over each attachment and insert into DB
    for (pos, id) in attachments.iter().enumerate() {
        let result = sqlx::query(sql)
        .bind(message_id)
        .bind(*id)
        .bind(pos as i64)
        .execute(&mut *conn)
        .await;

        if let Err(e) = result {

            // No "files" row for that id (client named a file that is gone)
            if let sqlx::Error::Database(db) = &e {
                if db.is_foreign_key_violation() {
                    return Err(AppError::Validation(format!("no file with id {id}")));
                }
            }

            return Err(AppError::Db(e));
        }
    }

    Ok(())
}

/// Reads every file attached to one message, ordered by ordinal.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `message_id` - Message the files are attached to.
pub async fn for_message(
    conn: &mut sqlx::SqliteConnection,
    message_id: Uuid,
) -> Result<Vec<File>> {

    let result = sqlx::query_as::<_, File>(
        "
        SELECT f.id, f.sha256, f.filename, f.mime, f.byte_size, f.created_at
        FROM message_attachments ma
        JOIN files f ON f.id = ma.file_id
        WHERE ma.message_id = ?1
        ORDER BY ma.ordinal
        "
    )
    .bind(message_id)
    .fetch_all(&mut *conn)
    .await?;

    Ok(result)
}

/// Reads the attachments for a page of messages, keyed by message.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `room_id` - Room the page was read from.
/// * `low` - Lowest seq in the page.
/// * `high` - Highest seq in the page.
pub async fn for_message_range(
    conn: &mut sqlx::SqliteConnection,
    room_id: Uuid,
    low: i64,
    high: i64,
) -> Result<HashMap<Uuid, Vec<File>>> {

    let rows = sqlx::query_as::<_, AttachmentRow>(
        "
        SELECT ma.message_id, f.id, f.sha256, f.filename, f.mime, f.byte_size, f.created_at
        FROM message_attachments ma
        JOIN messages m ON m.id = ma.message_id
        JOIN files f ON f.id = ma.file_id
        WHERE m.room_id = ?1 AND m.seq BETWEEN ?2 AND ?3
        ORDER BY ma.message_id, ma.ordinal
        "
    )
    .bind(room_id)
    .bind(low)
    .bind(high)
    .fetch_all(&mut *conn)
    .await?;

    let mut files: HashMap<Uuid, Vec<File>> = HashMap::new();
    for row in rows {
        files.entry(row.message_id).or_default().push(row.file);
    }

    Ok(files)
}
