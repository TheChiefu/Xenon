use serde::{Deserialize, Serialize};
use uuid::Uuid;


/// !!! Permanent !!!
/// Never reuse or renumber a retired variant's number once rows exist
/// Encodes as integer in DB
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(i8)]
pub enum GlobalRole {
    Owner = 0,
    Admin = 1,
    Member = 2,
    Visitor = 3
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(i8)]
pub enum Visibility {
    Public = 0, // Self service
    Locked = 1, // Invite only
    Hidden = 2  // Invite only
}

// Permissions //

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum Permission {
    DeleteRoom = 0,     // Delete room
    Rename = 1,         // Edit room name
    Manage = 2,         // Permissions
    Invite = 3,         // Create invites to room
    Nickname = 4,       // Per room display name (deferred)
    Post = 5,           // Allows user to send messages
    DeleteMsg = 6,      // Delete other's messages
    Attach = 7,         // File attachments
    Pin = 8,            // Pin specific message to pin tab
    Suspend = 9,        // Suspend users from room (time-limited; no permanent ban)
    Commands = 10,      // Use slash commands (unimplemented)
    Connect = 11,       // Join voice chat (unimplemented)
    Speak = 12,         // Can speak in voice chat (unimplemented)
    Mute = 13,          // Mute others in voice chat (unimplemented)
    Video = 14,         // Show webcam video (unimplemented)
    Screenshare = 15,   // Share screen (unimplemented)
}
const _: () = assert!((Permission::Screenshare as u8) < 63);

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, Serialize, Deserialize)]
#[sqlx(transparent)]
#[serde(transparent)]
pub struct Permissions(i64);

impl Permissions {
    pub const NONE: Self = Self(0);
    pub const ALL: Self = Self(-1);

    pub fn has(self, p: Permission) -> bool {
        let perm = p as u8;
        let bit = 1i64 << perm; // Shift by 'perm' bits left
        self.0 & bit != 0            // AND | Check if 'perm' bit is set (0 - No, 1 -Yes)
    }

    pub fn grant(self, p: Permission) -> Self {
        let perm = p as u8;
        let bit = 1i64 << perm;
        Self(self.0 | bit) // Turn 'perm' bit ON
    }

    pub fn revoke(self, p: Permission) -> Self {
        let perm = p as u8;
        let bit = 1i64 << perm;
        Self(self.0 & !bit) // Turn 'perm' bit OFF
    }
}

impl Default for Permissions {
    fn default() -> Self { Self::NONE }
}

// Users

/// Public subset of a user row: safe to hand to any authenticated caller,
/// unlike the full row (password_hash, email, global_role, ...)
#[derive(sqlx::FromRow, Serialize)]
pub struct UserSummary {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
}

// Room

#[derive(sqlx::FromRow, Serialize)]
pub struct Room {
    pub id: Uuid,
    pub name: Option<String>,
    pub visibility: Visibility,
    pub default_permissions: Permissions,
    pub created_at: i64,
    pub mutation_seq: i64
}

#[derive(sqlx::FromRow)]
pub struct Message {
    pub seq: i64,
    pub id: Uuid,
    pub room_id: Uuid,
    pub author_id: Uuid,
    pub body: Option<String>,
    pub created_at: i64,
    pub edited_at: Option<i64>,
    pub deleted_at: Option<i64>
}

/// Check if a user can delete a message:
/// 
/// - perm: An actor's permission bitmask (allowed permissions)
/// - actor_id: Who is performing the action
/// - target_id: Who is affected by the action
pub fn can_delete_message(perm: Permissions, actor_id: Uuid, target_id: Uuid) -> bool {
    if actor_id == target_id {
        return true;
    }

    if perm.has(Permission::DeleteMsg) {
        return true;
    }

    return false
}

#[derive(sqlx::FromRow)]
pub struct File {
    pub id: Uuid,
    pub sha256: Vec<u8>,
    pub filename: String,
    pub mime: String,
    pub byte_size: i64,
    pub created_at: i64
}