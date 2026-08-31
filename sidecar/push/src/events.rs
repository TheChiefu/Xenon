//! What crosses the socket between Xenon and this sidecar.
//!
//! Each type here has a counterpart in `server/src/sockets/events.rs`, and the
//! two have to agree or the JSON does not parse.

use serde::{Deserialize, Serialize};

/// One browser a push message is sent to.
#[derive(Deserialize)]
pub struct Subscription {
    pub endpoint: String,
    pub p256dh: Vec<u8>,
    pub auth: Vec<u8>
}

/// What Xenon sends.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    Push {
        room_id: String,
        room_name: String,
        author: String,
        body: String,
        renotify: bool,
        subscriptions: Vec<Subscription>
    }
}

/// What this sidecar sends.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SidecarEvent {
    /// The key browsers subscribe against, 65 bytes
    Key { public_key: Vec<u8> }
}

/// What one browser's service worker reads out of a push message.
#[derive(Serialize)]
pub struct Payload {
    pub room_id: String,
    pub room: String,
    pub author: String,
    pub body: String,
    pub renotify: bool
}
