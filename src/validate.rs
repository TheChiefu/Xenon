use crate::error::{AppError, Result};

pub const USERNAME_MIN_LENGTH: usize = 3;
pub const USERNAME_MAX_LENGTH: usize = 32;
pub const DISPLAY_NAME_MIN_LENGTH: usize = 1;
pub const DISPLAY_NAME_MAX_LENGTH: usize = 64;

pub fn username(name: &str) -> Result<()> {
    let len = name.chars().count();
    if len < USERNAME_MIN_LENGTH || len > USERNAME_MAX_LENGTH {
        return Err(AppError::Validation(format!(
        "username must be between {USERNAME_MIN_LENGTH} and {USERNAME_MAX_LENGTH} characters"
    )));
    }

    for c in name.chars() {
        let allowed = c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-';
        if !allowed {
            return Err(AppError::Validation(format!(
                "username may only contain lowercase letters, digits, underscores and hyphens"
            )));
        }
    }

    Ok(())
}

pub fn display_name(name: &str) -> Result<()> {
    let len = name.chars().count();
    if len < DISPLAY_NAME_MIN_LENGTH || len > DISPLAY_NAME_MAX_LENGTH {
        return Err(AppError::Validation(format!(
            "display name must be between {DISPLAY_NAME_MIN_LENGTH} and {DISPLAY_NAME_MAX_LENGTH} characters",
        )));
    }

    Ok(())
}