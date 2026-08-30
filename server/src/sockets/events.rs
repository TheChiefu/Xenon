//! What the server pushes to a connected client, and what one may send back.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::Status;
use crate::routes::messages::MessageResponse;
use crate::sockets::presence::{Device, Presence};

/// What a client sends over its socket.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientEvent {
    /// The status this connection declares
    Status { status: Status }
}

/// A user and what the reader is told to show for them.
#[derive(Serialize, Clone)]
pub struct UserPresence {
    pub user_id: Uuid,
    pub presence: Presence,

    /// Unset until the user's client declares one
    pub device: Option<Device>
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
        user_id: Uuid,
        room_id: Uuid,
        room_name: String,
        author: String,
        body: String,
        renotify: bool
    }
}
