//! Posts one encrypted message to a push service.
//!
//! <https://www.rfc-editor.org/rfc/rfc8030#section-5>

use std::error::Error;

use p256::SecretKey;
use reqwest::{Client, Response, StatusCode, Url};

use crate::encrypt::encrypt;
use crate::vapid::authorization;

/// Seconds a signed token stays valid
const TOKEN_SECONDS: u64 = 3600;

/// What a push service answered
pub enum Outcome {
    Sent,
    Gone,
    Rejected,
    TooLarge,
    Throttled(Option<u64>),
    Other(u16)
}

/// Encrypts a message and posts it to one subscription.
///
/// # Arguments
///
/// * `client` - Shared HTTP client.
/// * `key` - The signing key.
/// * `subject` - Contact URI for this server, `mailto:` or `https:`.
/// * `endpoint` - Push resource the browser was given.
/// * `p256dh` - Subscription's public key, 65 bytes.
/// * `auth` - Subscription's authentication secret, 16 bytes.
/// * `plaintext` - What the browser receives.
/// * `ttl` - Seconds the push service keeps the message for.
pub async fn send(
    client: &Client,
    key: &SecretKey,
    subject: &str,
    endpoint: &str,
    p256dh: &[u8],
    auth: &[u8],
    plaintext: &[u8],
    ttl: u32,
) -> Result<Outcome, Box<dyn Error>> {

    // The token names one service, so it cannot be used against another
    let origin = Url::parse(endpoint)?.origin().ascii_serialization();
    let token = authorization(key, &origin, subject, TOKEN_SECONDS)?;

    let body = encrypt(p256dh, auth, plaintext)?;

    let response = client
        .post(endpoint)
        .header("Authorization", token)
        .header("Content-Encoding", "aes128gcm")
        .header("TTL", ttl.to_string())
        .body(body)
        .send()
        .await?;

    Ok(match response.status() {
        StatusCode::CREATED => Outcome::Sent,
        StatusCode::NOT_FOUND | StatusCode::GONE => Outcome::Gone,
        StatusCode::FORBIDDEN => Outcome::Rejected,
        StatusCode::PAYLOAD_TOO_LARGE => Outcome::TooLarge,
        StatusCode::TOO_MANY_REQUESTS => Outcome::Throttled(retry_after(&response)),
        status => Outcome::Other(status.as_u16())
    })
}

// Helper Methods //

/// Seconds a push service asked to wait for. A date reads as `None`.
///
/// # Arguments
///
/// * `response` - What the push service answered.
fn retry_after(response: &Response) -> Option<u64> {
    let header = response.headers().get("Retry-After")?;
    let text = header.to_str().ok()?;

    text.parse().ok()
}
