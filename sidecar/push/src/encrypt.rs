//! Encrypts one push message.
//!
//! Key derivation is RFC 8291 section 3.4. Header and record layout are
//! RFC 8188 section 2. Only sending is implemented, and RFC 8291 section 4
//! allows a single record, so multiple records and padding are not.
//!
//! <https://www.rfc-editor.org/rfc/rfc8291#section-3.4>
//! <https://www.rfc-editor.org/rfc/rfc8188#section-2>

use std::error::Error;

use aes_gcm::aead::Aead;
use aes_gcm::{Aes128Gcm, KeyInit};
use hkdf::Hkdf;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::{PublicKey, SecretKey};
use rand_core::RngCore;
use sha2::Sha256;

/// Bytes of salt at the front of the header
const SALT_LEN: usize = 16;

/// Ends the last record
const DELIMITER: u8 = 0x02;

/// Bytes the authentication tag adds to the ciphertext
const TAG_LEN: usize = 16;

/// Encrypts a message for one subscription, returning the request body.
///
/// # Arguments
///
/// * `p256dh` - Subscription's public key, 65 bytes.
/// * `auth` - Subscription's authentication secret, 16 bytes.
/// * `plaintext` - What the browser receives.
pub fn encrypt(
    p256dh: &[u8],
    auth: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, Box<dyn Error>> {
    let browser = PublicKey::from_sec1_bytes(p256dh)?;

    // A new pair per message, sent as the keyid so the browser can agree the
    // same secret from its own private half
    let server = SecretKey::random(&mut rand_core::OsRng);
    let server_public = server.public_key().to_encoded_point(false);

    // ecdh_secret, which both sides arrive at without sending it
    let shared = p256::ecdh::diffie_hellman(
        server.to_nonzero_scalar(),
        browser.as_affine()
    );

    // key_info, binding the derivation to both public keys in this order
    let mut key_info = Vec::new();
    key_info.extend_from_slice(b"WebPush: info\0");
    key_info.extend_from_slice(p256dh);
    key_info.extend_from_slice(server_public.as_bytes());

    // IKM. The authentication secret salts the shared secret, so knowing both
    // public keys is not enough to derive it
    let (_, prk_key) = Hkdf::<Sha256>::extract(Some(auth), shared.raw_secret_bytes());
    let mut keying_material = [0u8; 32];
    prk_key.expand(&key_info, &mut keying_material)?;

    // Sent in the header, and never reused against the same keying material
    let mut salt = [0u8; SALT_LEN];
    rand_core::OsRng.fill_bytes(&mut salt);

    // CEK and NONCE, which AES-128-GCM is given below
    let derived = Hkdf::<Sha256>::new(Some(&salt), &keying_material);
    let mut key = [0u8; 16];
    derived.expand(b"Content-Encoding: aes128gcm\0", &mut key)?;
    let mut nonce = [0u8; 12];
    derived.expand(b"Content-Encoding: nonce\0", &mut nonce)?;

    // The message and the delimiter, with no padding after it
    let mut record = plaintext.to_vec();
    record.push(DELIMITER);

    // Encrypting appends the authentication tag, so this is TAG_LEN longer
    let cipher = Aes128Gcm::new(&key.into());
    let ciphertext = cipher.encrypt(&nonce.into(), record.as_slice())?;

    // Above the record's own length, which RFC 8291 section 4 requires
    let record_size = (plaintext.len() + 1 + TAG_LEN + 1) as u32;

    // salt, rs, idlen, keyid, then the record
    let mut body = Vec::new();
    body.extend_from_slice(&salt);
    body.extend_from_slice(&record_size.to_be_bytes());
    body.push(server_public.as_bytes().len() as u8);
    body.extend_from_slice(server_public.as_bytes());
    body.extend_from_slice(&ciphertext);

    Ok(body)
}
