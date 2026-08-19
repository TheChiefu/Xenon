//! Clock, random-token, and password-hashing helpers.

use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use argon2::password_hash::rand_core::{OsRng, RngCore};
use argon2::password_hash::{Error as PhcError, PasswordHasher, SaltString};
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use sha2::{Digest, Sha256};

use crate::error::{AppError, Result};

/// How many bytes a session token carries.
const SESSION_TOKEN_BYTES: usize = 32;

/// A session token in both the form the client keeps and the form the server stores.
pub struct SessionToken {
    pub secret: String, // Goes to client (never stored on server)
    pub hash: [u8; 32] // Goes in sessions.token_hash (never leaves server)
}

/// Returns the current time in milliseconds since the Unix epoch.
///
/// # Panics
///
/// Panics if the system clock reads before 1970.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before 1970")
        .as_millis() as i64
}

/// Generates a registration code from uppercase alphanumerics.
pub fn generate_invite_code() -> String {
    const CODE_ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    const CODE_LEN: usize = 12;

    let mut code = String::with_capacity(CODE_LEN);

    for _ in 0..CODE_LEN {
        let random = OsRng.next_u32() as usize; // Random Number from 0 to 4 billion
        let index = random % CODE_ALPHABET.len();   // Condense to range (0..36)
        code.push(CODE_ALPHABET[index] as char);    // Pick random char from allowed array
    }
    code
}

// Password Management //

/// Hashes a password with Argon2id, returning a PHC string.
///
/// # Arguments
///
/// * `password` - Password to hash.
///
/// # Errors
///
/// Returns `AppError::Hash` if the algorithm cannot run.
pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default().hash_password(password.as_bytes(), &salt)?;
    Ok(hash.to_string())
}

/// Reports whether a password matches a stored hash.
///
/// # Arguments
///
/// * `password` - Password to check.
/// * `stored_hash` - PHC string read from the database.
///
/// # Errors
///
/// Returns `AppError::Hash` if the stored hash is unreadable or the algorithm
/// cannot run.
pub fn verify_password(password: &str, stored_hash: &str) -> Result<bool> {
    let parsed = PasswordHash::new(stored_hash)?;

    match Argon2::default().verify_password(password.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(PhcError::Password) => Ok(false),
        Err(e) => Err(AppError::Hash(e)),
    }
}

// Session Management //

/// Generates a session token and its stored hash.
pub fn generate_session_token() -> SessionToken {
    let mut bytes = [0u8; SESSION_TOKEN_BYTES];
    OsRng.fill_bytes(&mut bytes);

    SessionToken {
        hash: Sha256::digest(bytes).into(),
        secret: hex::encode(bytes)
    }
}

/// Hashes a session token the client presented, matching the stored form.
///
/// # Arguments
///
/// * `secret` - Session token as the client sent it.
pub fn hash_session_token(secret: &str) -> Option<[u8; 32]> {
    let bytes = hex::decode(secret).ok()?;
    if bytes.len() != SESSION_TOKEN_BYTES {
        return None;
    }
    Some(Sha256::digest(&bytes).into())
}

// Security Management //

static DECOY_HASH: LazyLock<String> = LazyLock::new(|| {
    hash_password("decoy password, never matches a real login (hopefully)")
    .expect("hashing a fixed string cannot fail")
});

/// Burns the CPU time a password verification costs, against a fixed hash.
///
/// An unknown username runs this, so it takes the same time as a wrong password.
///
/// # Arguments
///
/// * `password` - Password to verify against the decoy hash.
pub fn burn_verify(password: &str) {
    let _ = verify_password(password, &DECOY_HASH);
}
