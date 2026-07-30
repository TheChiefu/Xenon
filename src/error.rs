use std::fmt;

#[derive(Debug)]
pub enum AppError {
    InvalidCredentials,
    InvalidInvite,
    UsernameTaken,
    EmailTaken,
    OwnerExists,
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
            AppError::OwnerExists => write!(f, "server owner already exists")
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