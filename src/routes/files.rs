//! HTTP handlers for file upload and download.

use axum::body::Body;
use axum::extract::{Multipart, Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::TryStreamExt;
use serde::Serialize;
use sqlx::SqlitePool;
use tokio_util::io::{ReaderStream, StreamReader};
use uuid::Uuid;

use crate::api::files::Stored;
use crate::bytesize;
use crate::error::{AppError, Result};
use crate::models::{File, GlobalRole};
use crate::routes::AuthUser;
use crate::{api, config, db, validate};

// Data Structs //

/// A stored file, as sent to clients.
#[derive(Clone, Serialize)]
pub struct FileResponse {
    pub id: Uuid,
    pub filename: String,
    pub mime: String,
    pub byte_size: i64,
}

impl From<File> for FileResponse {
    fn from(file: File) -> Self {
        Self {
            id: file.id,
            filename: file.filename,
            mime: file.mime,
            byte_size: file.byte_size,
        }
    }
}

// Routing Methods //

/// Stores an uploaded file.
///
/// # Arguments
///
/// * `caller_id` - Who is uploading.
/// * `pool` - Pool of SQL connections.
/// * `multipart` - Request body carrying the file.
pub async fn upload(
    AuthUser(caller_id, ..): AuthUser,
    State(pool): State<SqlitePool>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<FileResponse>)> {

    let mut tx = pool.begin().await?;

    // Reject unallowed roles to upload
    let allowed = [GlobalRole::Owner, GlobalRole::Admin, GlobalRole::Member];
    db::require_role(&mut tx, caller_id, &allowed).await?;
    tx.commit().await?;

    // Read POST body
    let field = match multipart.next_field().await {
        Ok(Some(field)) => field,
        Ok(None) => {
            let err = "the file upload request had no multipart fields";
            return Err(AppError::Validation(err.to_string()));
        }
        Err(e) => return Err(AppError::Io(std::io::Error::other(e)))
    };

    // Get file name
    let file_path: &str = field.file_name()
        .ok_or(AppError::Validation("no file name provided on file upload".to_string()))?;

    // Setup reader
    let file_name = validate::file_name(file_path)?;
    let reader = StreamReader::new(field.map_err(std::io::Error::other));

    // Get result and return outcome
    let result = api::files::store(&pool, &file_name, reader).await?;
    match result {
        Stored::Created(file) => Ok((StatusCode::CREATED, Json(file.into()))),
        Stored::Duplicate(file) => Ok((StatusCode::OK, Json(file.into())))
    }
}

/// Streams a stored file.
///
/// # Arguments
///
/// * `pool` - Pool of SQL connections.
/// * `file_id` - File to send.
pub async fn download(
    AuthUser(..): AuthUser,
    State(pool): State<SqlitePool>,
    Path(file_id): Path<Uuid>,
) -> Result<Response> {

    // Destructure the result
    let (file, handle) = api::files::fetch(&pool, file_id).await?;

    // Wrap handle into body and provide headers
    let body = Body::from_stream(ReaderStream::new(handle));
    let headers = [
        (header::CONTENT_TYPE, "application/octet-stream".to_string()),
        (header::CONTENT_DISPOSITION, "attachment".to_string()),
        (header::CONTENT_LENGTH, file.byte_size.to_string()),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
        (header::CACHE_CONTROL, "private, max-age=31536000, immutable".to_string())
    ];

    // Send response
    Ok((StatusCode::OK, headers, body).into_response())
}

/// Returns the largest upload body accepted.
///
/// Sits above `file_bytes_max` so the streaming read rejects the file first.
pub fn max_body_bytes() -> usize {
    let headroom_bytes = bytesize::MEBIBYTE;
    let max_file_bytes = config::get().limits.file_bytes_max.to_int();
    (headroom_bytes + max_file_bytes) as usize
}
