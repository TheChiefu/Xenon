use crate::{db, error::{AppError, Result}};

const LEN_MIN_USERNAME: usize = 3;
const LEN_MAX_USERNAME: usize = 32;
const LEN_MIN_DISPLAY_NAME: usize = 1;
const LEN_MAX_DISPLAY_NAME: usize = 64;
const LEN_MIN_PASSWORD: usize = 8;
const LEN_MAX_PASSWORD: usize = 128;

// Invite Defaults
pub const INVITE_DEFAULT_MAX_USES: i64 = 1;
pub const INVITE_LIFETIME_MS: i64 = db::DAY * 7;
const _: () = assert!(INVITE_DEFAULT_MAX_USES >= 1);

pub fn username(name: &str) -> Result<()> {
    let len = name.chars().count();
    if len < LEN_MIN_USERNAME || len > LEN_MAX_USERNAME {
        return Err(AppError::Validation(format!(
        "username must be between {LEN_MIN_USERNAME} and {LEN_MAX_USERNAME} characters"
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
    if len < LEN_MIN_DISPLAY_NAME || len > LEN_MAX_DISPLAY_NAME {
        return Err(AppError::Validation(format!(
            "display name must be between {LEN_MIN_DISPLAY_NAME} and {LEN_MAX_DISPLAY_NAME} characters",
        )));
    }

    Ok(())
}

pub fn password(password: &str) -> Result<()> {
    let len = password.chars().count();
    if len < LEN_MIN_PASSWORD || len > LEN_MAX_PASSWORD {
        return Err(AppError::Validation(format!(
            "password error: outside of available range ({LEN_MIN_PASSWORD} - {LEN_MAX_PASSWORD})"
        )));
    }

    Ok(())
}

pub fn invite_params(max_uses: i64, lifetime: i64) -> Result<()> {

    if max_uses < 1 {
        return Err(AppError::Validation(format!(
            "invite error: max uses must be at least 1"
        )));
    }

    if lifetime < 1 {
        return Err(AppError::Validation(format!(
            "invite error: lifetime must be at least 1 ms"
        )));
    }

    Ok(())
}