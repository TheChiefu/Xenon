use crate::db;
use crate::error::{AppError, Result};
use crate::config;

// Invite Defaults
pub const INVITE_DEFAULT_MAX_USES: i64 = 1;
pub const INVITE_LIFETIME_MS: i64 = db::DAY * 7;
const _: () = assert!(INVITE_DEFAULT_MAX_USES >= 1);

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
                format!("username error: may only contain lowercase, digits, underscores and hyphens")
            ));
        }
    }

    Ok(())
}

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

pub fn invite_params(max_uses: i64, lifetime: i64) -> Result<()> {

    if max_uses < 1 {
        return Err(AppError::Validation(
            format!("invite error: max uses must be at least 1")
        ));
    }

    if lifetime < 1 {
        return Err(AppError::Validation(
            format!("invite error: lifetime must be at least 1 ms")
        ));
    }

    Ok(())
}

pub fn room_name(name: Option<&str>) -> Result<Option<&str>> {
    
    match name {
        Some(val) => {
            let clean = val.trim();
            let len = clean.chars().count();

            // Normalize empty names as an "unnamed" room
            if len <= 0 {
                return Ok(None);
            }

            // Reject names longer than allowed limit
            let max = config::get().limits.room_name_max;
            if len > max {
                return Err(AppError::Validation(
                    format!("room name error: Name longer than character limit [{max}]")
                ));
            }

            return Ok(Some(clean))
        }

        // Null values are automatically returned as "unnamed" room
        None => return Ok(None)
    }
}

// Filesystem limit on one path component, so it stays out of the config
const FILE_NAME_MAX: usize = 255;

/// Strips any directory a client sent and returns the name alone
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

pub fn message_body(content: &str) -> Result<()> {
    let len = content.chars().count();
    let max = config::get().limits.message_body_max;
    if len > max {
        return Err(AppError::Validation(
            format!("message error: outside of max character limit ({max})")
        )) 
    }

    Ok(())
}