//! Rows as the rest of the server sees them.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// Roles //

/// A user's server-wide role, stored as an integer.
///
/// !!! Permanent !!!
/// Never reuse or renumber a retired variant's number once rows exist
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(i8)]
pub enum GlobalRole {
    Owner = 0,
    Admin = 1,
    Member = 2,
    Visitor = 3
}

/// How a room is discovered and entered, stored as an integer.
///
/// !!! Permanent !!!
/// Never reuse or renumber a retired variant's number once rows exist
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(i8)]
pub enum Visibility {
    /// Self service.
    Public = 0,
    /// Invite only.
    Locked = 1,
    /// Invite only.
    Hidden = 2
}

// Permissions //

/// One bit position in a [`Permissions`] mask.
///
/// !!! Permanent !!!
/// Never reuse or renumber a retired variant's number once rows exist
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum Permission {
    /// Send messages.
    Post = 0,
    /// Attach files to a message.
    Attach = 1,
    /// Delete other users' messages.
    DeleteMsg = 2,
    /// Delete the room.
    DeleteRoom = 3,
    /// Create invites to the room.
    Invite = 4,
    /// Set other users' permissions.
    Manage = 5,
    /// Edit the room name.
    Rename = 6,
    /// Remove a user from the room, with or without an expiry.
    Ban = 7,
    /// Use slash commands (unimplemented).
    Commands = 8,
    /// Join voice chat (unimplemented).
    Connect = 9,
    /// Speak in voice chat (unimplemented).
    Speak = 10,
    /// Mute others in voice chat (unimplemented).
    Mute = 11,
    /// Show webcam video (unimplemented).
    Video = 12,
    /// Share screen (unimplemented).
    Screenshare = 13,
}
const _: () = assert!((Permission::Screenshare as u8) < 63);

/// A set of [`Permission`] bits, stored as an integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, Serialize, Deserialize)]
#[sqlx(transparent)]
#[serde(transparent)]
pub struct Permissions(i64);

impl Permissions {
    pub const NONE: Self = Self(0);
    pub const ALL: Self = Self(-1);

    /// Reports whether the mask holds the given permission.
    ///
    /// # Arguments
    ///
    /// * `p` - Permission to test for.
    #[must_use]
    pub fn has(self, p: Permission) -> bool {
        let perm = p as u8;
        let bit = 1i64 << perm; // Shift by 'perm' bits left
        self.0 & bit != 0            // AND | Check if 'perm' bit is set (0 - No, 1 -Yes)
    }

    /// Returns the mask with the given permission added.
    ///
    /// # Arguments
    ///
    /// * `p` - Permission to add.
    #[must_use]
    pub fn grant(self, p: Permission) -> Self {
        let perm = p as u8;
        let bit = 1i64 << perm;
        Self(self.0 | bit) // Turn 'perm' bit ON
    }

    /// Returns the mask with the given permission removed.
    ///
    /// # Arguments
    ///
    /// * `p` - Permission to remove.
    #[must_use]
    pub fn revoke(self, p: Permission) -> Self {
        let perm = p as u8;
        let bit = 1i64 << perm;
        Self(self.0 & !bit) // Turn 'perm' bit OFF
    }

    /// Reports whether every permission in the given set is also in this one.
    ///
    /// # Arguments
    ///
    /// * `p` - Permission set that must be covered.
    pub fn contains(self, p: Permissions) -> bool {
        p.0 & !self.0 == 0 // No bit of 'p' is absent from self
    }

    /// Builds a mask from a list of permissions.
    ///
    /// # Arguments
    ///
    /// * `perms` - Permissions the mask holds.
    pub fn from_list(perms: &[Permission]) -> Permissions {
        let mut output = Self::NONE;
        for perm in perms {
            output = output.grant(*perm);
        }
        output
    }
}

impl Default for Permissions {
    fn default() -> Self { Self::NONE }
}

// Users //

/// Subset of a user, safe to hand to authenticated callers.
#[derive(sqlx::FromRow, Serialize)]
pub struct UserSummary {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub global_role: GlobalRole,
}

/// A `users` row.
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

/// A `rooms` row.
#[derive(sqlx::FromRow, Serialize)]
pub struct Room {
    pub id: Uuid,
    pub name: String,
    pub visibility: Visibility,
    pub default_permissions: Permissions,
    pub created_at: i64,
    pub mutation_seq: i64
}

// Messages //

/// A `messages` row.
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

/// A `files` row.
#[derive(sqlx::FromRow)]
pub struct File {
    pub id: Uuid,
    pub sha256: Vec<u8>,
    pub filename: String,
    pub mime: String,
    pub byte_size: i64,
    pub created_at: i64
}
