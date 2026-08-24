//! The `files` table, and the bytes it points at on disk.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

use crate::bytesize::ByteSize;
use crate::config;
use crate::error::{AppError, Result};
use crate::models::File;
use crate::utils;

// Data Structs //

/// Outcome of an upload. Bytes the server already holds return the stored row
/// rather than writing a second copy.
pub enum Stored {
    Created(File),
    Duplicate(File)
}

/// Metadata produced by reading an upload.
struct Stream {
    sha256: Vec<u8>,
    byte_size: i64,
    mime: &'static str
}

// API Methods //

/// Stores an upload, keyed on the hash of its bytes.
///
/// The stream is written to a temp file first, since the final path is the hash
/// and is unknown until the last byte is read.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `filename` - Name the file is stored under.
/// * `reader` - Byte stream of the uploaded file.
///
/// # Errors
///
/// Returns `AppError::TooLarge` if the stream passes the configured byte limit,
/// `AppError::Validation` if it is empty, and `AppError::Io` if a disk
/// operation fails.
pub async fn store<R>(
    pool: &SqlitePool,
    filename: &str,
    reader: R,
) -> Result<Stored>
where
    R: AsyncRead + Unpin,
{
    let files_path = &config::get().storage.files;

    // End file location is unknown until hashed,
    // write with tmp location/name until finished
    let tmp_path = Path::new(files_path)
        .join("tmp")
        .join(Uuid::now_v7().to_string());

    // Read stream into tmp file
    let max_bytes = config::get().limits.file_bytes_max;
    let stream = match read_stream(reader, &tmp_path, max_bytes).await {
        Ok(val) => val,
        Err(e) => {
            // On any error, remove tmp file
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(e);
        }
    };

    // Build sharded path and move tmp file into it (permanent location)
    let hex = hex::encode(&stream.sha256);
    let shard_dir = create_shard_path(files_path, &hex);
    if let Err(e) = move_to_shard(&tmp_path, &shard_dir, &hex).await {
        // Move failed, delete tmp file to avoid leaving unused data on disk
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(e);
    }
    // NOTE: In the event of an orphaned file a sweeping event that looks for
    // them (ie. no references in the DB) can delete them at a later point

    // Insert file info into Database
    let mut tx = pool.begin().await?;
    let stored = insert(&mut tx, filename, stream.sha256, stream.byte_size, stream.mime).await?;
    tx.commit().await?;

    Ok(stored)
}

/// Reads a file's row and opens a handle to its bytes on disk.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `file_id` - File to look up.
///
/// # Errors
///
/// Returns `AppError::NotFound` if no such row exists, and `AppError::Io` if
/// the bytes cannot be opened.
pub async fn fetch(
    pool: &SqlitePool,
    file_id: Uuid,
) -> Result<(File, tokio::fs::File)> {

    // Select file from table by id
    let mut conn = pool.acquire().await?;
    let f = sqlx::query_as::<_, File>(
        "
        SELECT id, sha256, filename, mime, byte_size, created_at
        FROM files
        WHERE id = ?1
        "
    )
    .bind(file_id)
    .fetch_optional(&mut *conn)
    .await?;

    // No file found, report as not found error
    let Some(file) = f else {
        return Err(AppError::NotFound);
    };

    // Rebuild path based on SHA256
    let hex_sha = hex::encode(&file.sha256);
    let directory = create_shard_path(&config::get().storage.files, &hex_sha);
    let path = directory.join(&hex_sha);

    // Get file reader handle
    match tokio::fs::File::open(&path).await {
        Ok(handle) => Ok((file, handle)),
        Err(e) => {
            // Expected a "files" row, but has no bytes on disk (storage inconsistency)
            if e.kind() == std::io::ErrorKind::NotFound {
                tracing::error!("file {file_id} has no bytes at {}", path.display());
            }
            Err(AppError::Io(e))
        }
    }
}

// Helper Methods //

/// Creates the shard directory and renames the temp file into it.
///
/// # Arguments
///
/// * `tmp_path` - Temp file holding the uploaded bytes.
/// * `shard_dir` - Directory the hex file is placed in.
/// * `hex` - Hex representation of the file's hash, used as its name.
///
/// # Errors
///
/// Returns `AppError::Io` if the directory or the rename fails.
async fn move_to_shard(tmp_path: &Path, shard_dir: &Path, hex: &str) -> Result<()> {
    tokio::fs::create_dir_all(shard_dir).await?;
    tokio::fs::rename(tmp_path, shard_dir.join(hex)).await?;
    Ok(())
}

/// Reads a stream into a temp file, returning the metadata it gathered.
///
/// The MIME type is inferred from the leading bytes, falling back to
/// `text/plain` for valid UTF-8 and `application/octet-stream` otherwise.
///
/// # Arguments
///
/// * `reader` - Byte stream of the uploaded file.
/// * `tmp_path` - Path the local copy is written to.
/// * `max_bytes` - Size the file is rejected at, checked while streaming.
///
/// # Errors
///
/// Returns `AppError::TooLarge` if the stream passes `max_bytes`,
/// `AppError::Validation` if it holds no bytes, and `AppError::Io` if a read or
/// write fails.
async fn read_stream<R>(
    mut reader: R,
    tmp_path: &Path,
    max_bytes: ByteSize,
) -> Result<Stream>
where
    R: AsyncRead + Unpin,
{
    let mut tmp_file = tokio::fs::File::create(tmp_path).await?;

    // Setup reader and metadata properties
    let mut buffer = [0u8; 65536]; // 64KiB chunks
    let mut read_bytes: i64 = 0;
    let mut hasher = Sha256::new();
    let mut mime = "application/octet-stream";
    let max = max_bytes.to_int();

    // Loop over file bytes
    loop {
        let n = reader.read(&mut buffer).await?; // Read bytes (dynamic size)
        if n == 0 {
            break; // No more bytes, stop reading
        }
        let chunk = &buffer[..n];

        // Infer matches leading bytes as file MIME
        if read_bytes == 0 {
            if let Some(val) = infer::get(chunk) {
                mime = val.mime_type();
            } else if is_utf8_text(chunk) {
                mime = "text/plain";
            }
        }

        // Update read bytes
        read_bytes += n as i64;

        // File reached limit (disallow)
        if read_bytes > max {
            return Err(AppError::TooLarge(max_bytes));
        }

        // Update tmp file and hasher by chunks
        tmp_file.write_all(chunk).await?;
        hasher.update(chunk);
    }

    // Reject empty files
    if read_bytes == 0 {
        return Err(AppError::Validation("file is empty".to_string()));
    }

    // Finish/flush buffered writes to OS
    // (in case a failure needs be returned)
    tmp_file.flush().await?;

    // Return read file metadata and hash
    Ok(Stream {
        sha256: hasher.finalize().to_vec(),
        byte_size: read_bytes,
        mime
    })
}

/// Maps a hash to the directory holding it, such as `files/ab/cd`.
///
/// # Arguments
///
/// * `files_path` - Path to the permanent file location.
/// * `hex` - Hex representation of the file's hash.
fn create_shard_path(files_path: &str, hex: &str) -> PathBuf {
    Path::new(files_path)
        .join(&hex[0..2]) // Top Level first 2 hex chars
        .join(&hex[2..4]) // Second Level second 2 hex chars
}

/// Writes a row to the `files` table, keyed on the hash of its bytes.
///
/// Bytes already stored return the existing row, so the `id` and `filename` are
/// the ones from the first upload rather than the ones passed here.
///
/// # Arguments
///
/// * `conn` - Connection to SQL DB.
/// * `filename` - Name the file is stored under.
/// * `sha256` - Hash of the bytes, used as the dedupe key.
/// * `byte_size` - Size of the file in bytes.
/// * `mime` - MIME type for clients to handle.
async fn insert(
    conn: &mut sqlx::SqliteConnection,
    filename: &str,
    sha256: Vec<u8>,
    byte_size: i64,
    mime: &'static str,
) -> Result<Stored> {

    let id = Uuid::now_v7();
    let now = utils::now_ms();

    let affected = sqlx::query(
        "
        INSERT INTO files (id, sha256, filename, mime, byte_size, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT (sha256) DO NOTHING
        "
    )
    .bind(id)
    .bind(sha256.as_slice())
    .bind(filename)
    .bind(mime)
    .bind(byte_size)
    .bind(now)
    .execute(&mut *conn)
    .await?
    .rows_affected();

    // Nothing written means these bytes are already stored
    if affected == 0 {

        let file = sqlx::query_as::<_, File>(
            "
            SELECT id, sha256, filename, mime, byte_size, created_at
            FROM files
            WHERE sha256 = ?1
            "
        )
        .bind(sha256.as_slice())
        .fetch_one(&mut *conn)
        .await?;

        return Ok(Stored::Duplicate(file));
    }

    // If an insert occurred, return newly created file info
    Ok(Stored::Created(
        File {
            id,
            sha256,
            filename: filename.to_string(),
            mime: mime.to_string(),
            byte_size,
            created_at: now
        }
    ))
}

/// Reports whether a chunk of bytes reads as UTF-8 text.
///
/// # Arguments
///
/// * `chunk` - Leading bytes of the file.
fn is_utf8_text(chunk: &[u8]) -> bool {

    // Check for NULs
    if chunk.contains(&0) {
        return false;
    }

    match std::str::from_utf8(chunk) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none()
    }
}
