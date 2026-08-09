use std::path::{Path, PathBuf};
use uuid::Uuid;
use sqlx::SqlitePool;
use tokio::{self, io::{AsyncRead, AsyncReadExt, AsyncWriteExt}};
use sha2::{Digest, Sha256};

use crate::bytesize::ByteSize;
use crate::error::{AppError, Result};
use crate::{models::File, utils, config};

pub enum Stored {
    Created(File),
    Duplicate(File)
}

/// Produced by reading an upload
struct Stream {
    sha256: Vec<u8>,
    byte_size: i64,
    mime: &'static str
}

/// Stores an upload, returning the matching "files"
/// entry if the bytes are already stored.
/// - pool: Pool where uploaded file information is stored
/// - file: What the name of the file is saved as
/// - reader: Reader that reads byte stream of uploaded file
pub async fn store<R>(
    pool: &SqlitePool,
    filename: &str,
    reader: R,
) -> Result<Stored>
where
    R: AsyncRead + Unpin,
{
    let files_path = &config::get().storage.files_path;

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
    let stored = insert_file(&mut tx, filename, stream.sha256, stream.byte_size, &stream.mime).await?;
    tx.commit().await?;

    Ok(stored)
}

// Helper Methods //

/// Creates the shard directory and moves the temp file into it (via rename)
/// - tmp_path: Current tmp file containing uploaded file data
/// - shard_dir: Final directory where hex file is placed in
/// - hex: Hex representation of file sha representing the file name 
async fn move_to_shard(tmp_path: &Path, shard_dir: &Path, hex: &str) -> Result<()> {
    tokio::fs::create_dir_all(shard_dir).await?;
    tokio::fs::rename(tmp_path, shard_dir.join(hex)).await?;
    Ok(())
}

/// Read file stream and create tmp file containing its data and return file metadata.
/// - reader: Async reader
/// - tmp_path: Path were local copy of streamed data is stored
/// - max_bytes: Max allowable size on disk (file rejected if limit is reached while streaming)
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
    let mut buffer = [0u8; 16384]; // 16KiB chunks
    let mut read_bytes: i64 = 0;
    let mut hasher = Sha256::new();
    let mut mime = "application/octet-stream";
    let max = max_bytes.to_int();

    // Loop over file bytes
    loop {
        let n = reader.read(&mut buffer).await?; // Read bytes (dynamic size)
        if n == 0 { break; } // No more bytes, stop reading
        let chunk = &buffer[..n];

        // Infer matches leading bytes as file MIME
        if read_bytes == 0 {
            if let Some(val) = infer::get(chunk) {
                mime = val.mime_type();
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

/// Maps a hash to the directory holding it (such as files/ab/cd)
/// - files_path: Path to permanent file location
/// - hex: Hexadecimal representation of file to be stored
fn create_shard_path(files_path: &str, hex: &str) -> PathBuf {
    Path::new(files_path)
        .join(&hex[0..2]) // Top Level first 2 hex chars
        .join(&hex[2..4]) // Second Level second 2 hex chars
}

/// Insert file metadata into "files" DB table.
/// Returns a table entry based on the sha256 key
/// - conn: Connection to SQL DB
/// - filename: Name of file to store
/// - sha256: Hashed representation used as dudupe key
/// - byte_size: Size of file in bytes
/// - mime: MIME type of file for clients to handle
async fn insert_file(
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

        // Keyed on sha256 alone. The stored file has the first uploader's id,
        // never the one generated above
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

    // If an insert occured, return newly created file info
    Ok(Stored::Created(
        File {
            id,
            sha256: sha256.clone(),
            filename: filename.to_string(),
            mime: mime.to_string(),
            byte_size: byte_size,
            created_at: now
        }
    ))
}
