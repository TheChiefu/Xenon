use argon2::password_hash::rand_core::{OsRng, RngCore};
use argon2::password_hash::{Error as PhcError, PasswordHasher, SaltString};
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use sha2::{Digest, Sha256};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{AppError, Result};

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before 1970")
        .as_millis() as i64
}

/// Generates invite code from uppercase alphanumerics
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

/// Hashes a password with Argon2id, returning a PHC string
pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default().hash_password(password.as_bytes(), &salt)?;
    Ok(hash.to_string())
}

/// Ok (true)  - Password Matches
/// Ok (false) - Password does not match
/// Err        - Stored hash is unusable (corrupt / algorithm can't run)
pub fn verify_password(password: &str, stored_hash: &str) -> Result<bool> {
    let parsed = PasswordHash::new(stored_hash)?;

    match Argon2::default().verify_password(password.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(PhcError::Password) => Ok(false),
        Err(e) => Err(AppError::Hash(e)),
    }
}

// Session Management //
const SESSION_TOKEN_BYTES: usize = 32;
pub struct SessionToken {
    pub secret: String, // Goes to client (never stored on server)
    pub hash: [u8; 32] // Goes in sessions.token_hash (never leaves server)
}

/// Generates a session token and its stored hash
pub fn generate_session_token() -> SessionToken {
    let mut bytes = [0u8; SESSION_TOKEN_BYTES];
    OsRng.fill_bytes(&mut bytes);

    SessionToken {
        hash: Sha256::digest(bytes).into(),
        secret: hex::encode(bytes)
    }
}

pub fn hash_session_token(secret: &str) -> Option<[u8; 32]> {
    let bytes = hex::decode(secret).ok()?;
    if bytes.len() != SESSION_TOKEN_BYTES {
        return None;
    }
    Some(Sha256::digest(&bytes).into())
}

// Security Management //
static DECOY_HASH: LazyLock<String> = LazyLock::new(|| {
    hash_password("decoy password, never matches a realy login (hopefully)")
    .expect("hashing a fixed string cannot fail")
});

/// Used to burn CPU time so that an attacker who wants to detect MS differences
/// between username searches, password hash lookups, etc will get the same MS
/// response time for all
pub fn burn_verify(password: &str) {
    let _ = verify_password(password, &DECOY_HASH);
}
