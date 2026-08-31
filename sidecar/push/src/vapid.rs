//! The signing key, and the token sent with every push request.
//!
//! The key pair, the header, and the claims are defined by RFC 8292 section 2.
//! <https://www.rfc-editor.org/rfc/rfc8292#section-2>

use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use p256::SecretKey;
use p256::ecdsa::{Signature, SigningKey, signature::Signer};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use serde::Serialize;

/// File the signing key is stored in, relative to the working directory
pub const KEY_FILE: &str = "vapid.key";

/// The only header VAPID allows, ECDSA on P-256
const HEADER: &str = r#"{"typ":"JWT","alg":"ES256"}"#;

/// What one token states
#[derive(Serialize)]
struct Claims {
    aud: String,
    exp: u64,
    sub: String
}

/// Builds the `Authorization` header for one push request.
/// <https://www.rfc-editor.org/rfc/rfc8292#section-3>
///
/// # Arguments
///
/// * `key` - The signing key.
/// * `audience` - Origin of the push resource URL, scheme and host only.
/// * `subject` - Contact URI for this server, `mailto:` or `https:`.
/// * `seconds` - How long the token stays valid, at most 86400.
pub fn authorization(
    key: &SecretKey,
    audience: &str,
    subject: &str,
    seconds: u64,
) -> Result<String, Box<dyn Error>> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

    let claims = Claims {
        aud: audience.to_string(),
        exp: now + seconds,
        sub: subject.to_string()
    };

    // What the signature covers, header and claims joined by a period
    let signed = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(HEADER),
        URL_SAFE_NO_PAD.encode(serde_json::to_string(&claims)?)
    );

    let signature: Signature = SigningKey::from(key).sign(signed.as_bytes());
    let token = URL_SAFE_NO_PAD.encode(signature.to_bytes());

    Ok(format!("vapid t={signed}.{token}, k={}", public_key(key)))
}

/// Reads the stored signing key, writing a new one when the file is absent.
///
/// # Arguments
///
/// * `path` - File the key is stored in.
pub fn read_or_create(path: &str) -> Result<SecretKey, Box<dyn Error>> {
    if Path::new(path).exists() {
        let text = fs::read_to_string(path)?;
        let bytes = URL_SAFE_NO_PAD.decode(text.trim())?;
        return Ok(SecretKey::from_slice(&bytes)?);
    }

    let key = SecretKey::random(&mut rand_core::OsRng);
    write_key(path, &key)?;

    Ok(key)
}

/// The public half, in the form a browser passes as `applicationServerKey`.
/// <https://www.w3.org/TR/push-api/#dom-pushsubscriptionoptionsinit-applicationserverkey>
///
/// # Arguments
///
/// * `key` - The signing key.
pub fn public_key(key: &SecretKey) -> String {
    URL_SAFE_NO_PAD.encode(public_key_bytes(key))
}

/// The public half as its 65 bytes, which is what Xenon stores.
///
/// # Arguments
///
/// * `key` - The signing key.
pub fn public_key_bytes(key: &SecretKey) -> Vec<u8> {
    key.public_key().to_encoded_point(false).as_bytes().to_vec()
}

// Helper Methods //

/// Writes a signing key to a file only its owner can read,
/// encoded base64url with the trailing padding omitted.
/// <https://www.rfc-editor.org/rfc/rfc7515#page-6>
///
/// # Arguments
///
/// * `path` - File to write.
/// * `key` - Key to store.
fn write_key(path: &str, key: &SecretKey) -> Result<(), Box<dyn Error>> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options.open(path)?;
    let text = URL_SAFE_NO_PAD.encode(&key.to_bytes());

    Ok(file.write_all(text.as_bytes())?)
}
