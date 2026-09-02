//! What the server pushes to a connected client, and what one may send back.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::{Platform, Status};
use crate::routes::messages::MessageResponse;
use crate::sockets::presence::{Device, Presence};

/// What a client sends over its socket.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientEvent {
    /// The status this connection declares
    Status { status: Status }
}

/// How a linked game account appears to someone sharing a room with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum GamePresence {
    Online,
    Offline,
    Away
}

/// What a linked account last reported about the user who owns it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameActivity {
    pub platform: Platform,
    pub status: GamePresence,

    /// Name of the game
    pub title: Option<String>,

    /// What the game says they are doing
    pub activity: Option<String>
}

/// How one link attempt ended.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LinkOutcome {
    Linked {
        user_id: Uuid,
        platform: Platform,
        handle: String
    },
    Error {
        user_id: Uuid,
        platform: Platform,
        message: String
    }
}

/// What the sidecar sends over its socket.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SidecarEvent {
    /// The key browsers subscribe against, 65 bytes
    VapidKey { public_key: Vec<u8> },

    /// Where to send someone to sign in with the platform
    LinkUrl {
        user_id: Uuid,
        platform: Platform,
        authorize_url: String
    },

    /// How a link attempt ended
    LinkResult { outcome: LinkOutcome },

    /// This user's link stopped renewing and has to be made again
    NeedsReauth {
        user_id: Uuid,
        platform: Platform
    },

    /// Sent when a linked account's presence changes
    Presence {
        user_id: Uuid,

        #[serde(flatten)]
        game: GameActivity
    },

    /// Asks for every user Xenon lists as linked. The sidecar drops the
    /// credentials of anyone missing from the answer
    GetLinkedAccounts
}

/// A user and the linked account they are on.
#[derive(Serialize, Clone)]
pub struct UserGamePresence {
    pub user_id: Uuid,

    #[serde(flatten)]
    pub game: GameActivity
}

/// A user and what the reader is told to show for them.
#[derive(Serialize, Clone)]
pub struct UserPresence {
    pub user_id: Uuid,
    pub presence: Presence,

    /// Unset until the user's client declares one
    pub device: Option<Device>
}

/// One browser a push message is sent to.
#[derive(Clone, Deserialize, Serialize)]
pub struct Subscription {
    pub endpoint: String,
    pub p256dh: Vec<u8>,
    pub auth: Vec<u8>
}

#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    Message {
        room_id: Uuid,
        message: MessageResponse
    },
    MessageDeleted {
        room_id: Uuid,
        message_id: Uuid
    },
    MessageEdited {
        room_id: Uuid,
        message_id: Uuid,
        body: Option<String>,
        edited_at: i64 
    },
    Invited {
        room_id: Uuid,
        invited_by: Uuid
    },
    InviteRevoked {
        room_id: Uuid
    },
    Banned {
        room_id: Uuid
    },
    MemberJoined {
        room_id: Uuid,
        user_id: Uuid
    },
    MemberLeft {
        room_id: Uuid,
        user_id: Uuid
    },
    RoomDeleted {
        room_id: Uuid
    },
    RoomUpdated {
        room_id: Uuid
    },
    PresenceUpdated {
        user_id: Uuid,
        presence: Presence,
        device: Option<Device>
    },
    PresenceSnapshot {
        users: Vec<UserPresence>
    },
    GamePresenceSnapshot {
        users: Vec<UserGamePresence>
    },
    ProfileUpdated {
        user_id: Uuid,
        display_name: String,
        description: String,
        avatar_file_id: Option<Uuid>,
        banner_file_id: Option<Uuid>
    },
    Resync,
    Notification {
        room_id: Uuid,
        room_name: String,
        author: String,
        body: String
    },
    Push {
        room_id: Uuid,
        room_name: String,
        author: String,
        body: String,
        renotify: bool,
        user_ids: Vec<Uuid>
    },
    Subscribe {
        user_id: Uuid,
        subscription: Subscription
    },
    Unsubscribe {
        user_id: Uuid,
        endpoint: String
    },
    LinkRequested {
        user_id: Uuid,
        platform: Platform
    },

    /// The callback's query string, passed on unread
    LinkCallback {
        params: HashMap<String, String>
    },
    LinkedAccounts {
        user_ids: Vec<Uuid>
    },
    LinkUrl {
        platform: Platform,
        authorize_url: String
    },
    AccountLinked {
        user_id: Uuid,
        platform: Platform,
        handle: String
    },
    AccountUnlinked {
        user_id: Uuid,
        platform: Platform
    },
    LinkFailed {
        user_id: Uuid,
        platform: Platform,
        message: String
    },
    LinkNeedsReauth {
        user_id: Uuid,
        platform: Platform
    },
    GamePresenceUpdated {
        user_id: Uuid,

        #[serde(flatten)]
        game: GameActivity
    }
}
