use std::fmt;
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use uuid::Uuid;
use tracing;

#[derive(Debug)]
pub enum AppError {
    InvalidCredentials,
    InvalidInvite,
    UsernameTaken,
    EmailTaken,
    OwnerExists,
    Forbidden,
    Db(sqlx::Error),
    Hash(argon2::password_hash::Error),
    Validation(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::InvalidCredentials => write!(f, "invalid username or password"),
            AppError::InvalidInvite => write!(f, "invalid or unusable invite code"),
            AppError::UsernameTaken => write!(f, "username is already taken"),
            AppError::Db(e) => write!(f, "database error: {e}"),
            AppError::Hash(e) => write!(f, "password hashing error: {e}"),
            AppError::Validation(msg) => write!(f, "{msg}"),
            AppError::EmailTaken => write!(f, "email already in use"),
            AppError::OwnerExists => write!(f, "server owner already exists"),
            AppError::Forbidden => write!(f,"action not allowed"),
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AppError::Db(e) => Some(e),
            AppError::Hash(e) => Some(e),
            _ => None
        }
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        AppError::Db(e)
    }
}

impl From<argon2::password_hash::Error> for AppError {
    fn from(e: argon2::password_hash::Error) -> Self {
        AppError::Hash(e)
    }
}

impl From<sqlx::migrate::MigrateError> for AppError {
    fn from(e: sqlx::migrate::MigrateError) -> Self {
        AppError::Db(e.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::InvalidCredentials => (StatusCode::UNAUTHORIZED, self.to_string()),
            AppError::InvalidInvite => (StatusCode::FORBIDDEN, self.to_string()),
            AppError::UsernameTaken | AppError::EmailTaken => (StatusCode::CONFLICT, self.to_string()),
            AppError::Validation(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            AppError::Forbidden => (StatusCode::FORBIDDEN, self.to_string()),

            // Never reaches client
            AppError::OwnerExists | AppError::Db(_) | AppError::Hash(_) => {
                let id = Uuid::now_v7();
                tracing::error!("internal error {id}: {self}");
                (StatusCode::INTERNAL_SERVER_ERROR, format!("internal server error ({id})"))
            }
        };

        (status, Json(serde_json::json!({"error": message}))).into_response()
    }
}

/// Shorthand: `Result<Uuid>` instead of `Result<Uuid, AppError>`
pub type Result<T> = std::result::Result<T, AppError>;


pub fn unique_violation(e: sqlx::Error) -> AppError {
    let mapped = match &e {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            let msg = db.message();
            if msg.contains("users.username") {
                Some(AppError::UsernameTaken)
            } else if msg.contains("users.email") {
                Some(AppError::EmailTaken)
            } else if msg.contains("users.global_role") {
                Some(AppError::OwnerExists)   // one_owner; bootstrap only
            } else {
                None
            }
        }

        _ => None,
    };

    mapped.unwrap_or(AppError::Db(e))
}