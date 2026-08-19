//! Format checks run before a value reaches the database.

use crate::config;
use crate::db;
use crate::error::{AppError, Result};

/// How many registrations a code covers when the request names no count.
pub const INVITE_DEFAULT_MAX_USES: i64 = 1;

/// How long a registration code lasts when the request names no lifetime.
pub const INVITE_LIFETIME_MS: i64 = db::DAY * 7;

const _: () = assert!(INVITE_DEFAULT_MAX_USES >= 1);

/// Filesystem limit on one path component.
const FILE_NAME_MAX: usize = 255;

/// Checks a login name's length and character set.
///
/// # Arguments
///
/// * `name` - Login name being claimed.
///
/// # Errors
///
/// Returns `AppError::Validation` if the name is outside its length limits or
/// holds anything but lowercase letters, digits, underscores, and hyphens.
pub fn username(name: &str) -> Result<()> {
    let len = name.chars().count();
    let min = config::get().limits.username_min;
    let max = config::get().limits.username_max;

    // Enforce length limits
    if len < min || len > max {
        return Err(AppError::Validation(
            format!("username error: must be between {min} and {max} characters")
        ));
    }

    // Enforce username restrictions
    for c in name.chars() {
        let allowed = c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-';
        if !allowed {
            return Err(AppError::Validation(
                "username error: may only contain lowercase, digits, underscores and hyphens"
                    .to_string()
            ));
        }
    }

    Ok(())
}

/// Checks a display name's length.
///
/// # Arguments
///
/// * `name` - Name shown to other users.
///
/// # Errors
///
/// Returns `AppError::Validation` if the name is outside its length limits.
pub fn display_name(name: &str) -> Result<()> {
    let len = name.chars().count();
    let min = config::get().limits.display_name_min;
    let max = config::get().limits.display_name_max;

    if len < min || len > max {
        return Err(AppError::Validation(
            format!("display name error: must be between {min} and {max} characters")
        ));
    }

    Ok(())
}

/// Checks a password's length.
///
/// # Arguments
///
/// * `password` - Password being set.
///
/// # Errors
///
/// Returns `AppError::Validation` if the password is outside its length limits.
pub fn password(password: &str) -> Result<()> {
    let len = password.chars().count();
    let min = config::get().limits.password_min;
    let max = config::get().limits.password_max;

    if len < min || len > max {
        return Err(AppError::Validation(
            format!("password error: outside of available range ({min} - {max})")
        ));
    }

    Ok(())
}

/// Checks the use count and lifetime a registration code is created with.
///
/// # Arguments
///
/// * `max_uses` - How many registrations the code covers.
/// * `lifetime` - How long (in ms) the code lasts.
///
/// # Errors
///
/// Returns `AppError::Validation` if either value is below 1.
pub fn invite_params(max_uses: i64, lifetime: i64) -> Result<()> {

    if max_uses < 1 {
        return Err(AppError::Validation(
            "invite error: max uses must be at least 1".to_string()
        ));
    }

    if lifetime < 1 {
        return Err(AppError::Validation(
            "invite error: lifetime must be at least 1 ms".to_string()
        ));
    }

    Ok(())
}

/// Trims a room name, returning `None` for one that is empty once trimmed.
///
/// # Arguments
///
/// * `name` - Room name as the client sent it.
///
/// # Errors
///
/// Returns `AppError::Validation` if the trimmed name is over the length limit.
pub fn room_name(name: Option<&str>) -> Result<Option<&str>> {

    // Null values are returned as an "unnamed" room
    let Some(val) = name else {
        return Ok(None);
    };

    let clean = val.trim();
    let len = clean.chars().count();

    // Normalize empty names as an "unnamed" room
    if len == 0 {
        return Ok(None);
    }

    // Reject names longer than allowed limit
    let max = config::get().limits.room_name_max;
    if len > max {
        return Err(AppError::Validation(
            format!("room name error: Name longer than character limit [{max}]")
        ));
    }

    Ok(Some(clean))
}

/// Strips any directory a client sent and returns the name alone.
///
/// # Arguments
///
/// * `path` - File path as the client sent it.
///
/// # Errors
///
/// Returns `AppError::Validation` if the path names a directory or the
/// remaining name is over the filesystem limit.
pub fn file_name(path: &str) -> Result<String> {

    // Find separators in path (client's OS is unknown match both slashes)
    let index = match path.rfind(['/', '\\']) {
        Some(i) => i + 1,
        None => 0
    };
    let name = &path[index..];

    // Strip any folder leading dots (ie up dir / same dir)
    if name.is_empty() || name == "." || name == ".." {
        return Err(AppError::Validation(
            format!("file name error: [{path}] names a directory")
        ));
    }

    // If remaining filename is larger than OS limit, error
    let len = name.chars().count();
    if len > FILE_NAME_MAX {
        return Err(AppError::Validation(
            format!("file name error: longer than character limit [{FILE_NAME_MAX}]")
        ));
    }

    Ok(name.to_string())
}

/// Checks a message body's length.
///
/// # Arguments
///
/// * `content` - Message contents.
///
/// # Errors
///
/// Returns `AppError::Validation` if the body is over the length limit.
pub fn message_body(content: &str) -> Result<()> {
    let len = content.chars().count();
    let max = config::get().limits.message_body_max;
    if len > max {
        return Err(AppError::Validation(
            format!("message error: outside of max character limit ({max})")
        ));
    }

    Ok(())
}
