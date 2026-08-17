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
    Post = 0,           // Allows user to send messages
    Attach = 1,         // File attachments
    DeleteMsg = 2,      // Delete other's messages
    DeleteRoom = 3,     // Delete room
    Invite = 4,         // Create invites to room
    Manage = 5,         // Permissions
    Rename = 6,         // Edit room name
    Suspend = 7,        // Remove a user from a room, with or without an expiry
    Commands = 8,       // Use slash commands (unimplemented)
    Connect = 9,        // Join voice chat (unimplemented)
    Speak = 10,         // Can speak in voice chat (unimplemented)
    Mute = 11,          // Mute others in voice chat (unimplemented)
    Video = 12,         // Show webcam video (unimplemented)
    Screenshare = 13,   // Share screen (unimplemented)
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

// Users //

/// Subset of a user, safe to hand to authenticated callers
#[derive(sqlx::FromRow, Serialize)]
pub struct UserSummary {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub global_role: GlobalRole,
}

// Public subset of user without private identifying information
#[derive(sqlx::FromRow, Serialize)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub description: String,
    pub avatar_file_id: Uuid,
    pub banner_file_id: Uuid,
    pub global_role: i8,
    pub status_pref: i8,
    pub created_at: i64,
    pub deleted_at: i64
}

// Room //

#[derive(sqlx::FromRow, Serialize)]
pub struct Room {
    pub id: Uuid,
    pub name: Option<String>,
    pub visibility: Visibility,
    pub default_permissions: Permissions,
    pub created_at: i64,
    pub mutation_seq: i64
}

/// A `room_invites` row plus `rooms.name`, since
/// the invitee has no `room_access` row on their own
#[derive(sqlx::FromRow, Serialize)]
pub struct RoomInvite {
    pub room_id: Uuid,
    pub room_name: Option<String>,
    pub invited_by: Uuid,
    pub created_at: i64,
    pub expires_at: Option<i64>,
}

// Messages //

#[derive(sqlx::FromRow)]
pub struct Message {
    pub seq: i64,
    pub id: Uuid,
    pub room_id: Uuid,
    pub author_id: Uuid,
    pub body: Option<String>,
    pub created_at: i64,
    pub edited_at: Option<i64>,
    pub deleted_at: Option<i64>,
    pub spoiler: bool
}

// Files //

#[derive(sqlx::FromRow)]
pub struct File {
    pub id: Uuid,
    pub sha256: Vec<u8>,
    pub filename: String,
    pub mime: String,
    pub byte_size: i64,
    pub created_at: i64
}