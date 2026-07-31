use uuid::Uuid;


/// !!! Permanent !!!
/// Never reuse or renumber a retired variant's number once rows exist
/// Encodes as integer in DB
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[repr(i16)]
pub enum GlobalRole {
    Owner = 0,
    Admin = 1,
    Member = 2
}


#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[repr(i16)]
pub enum Visibility {
    Public = 0, // Self service
    Locked = 1, // Invite only
    Hidden = 2  // Invite only
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Permission {
    Expunge = 0,        // Delete room
    Rename = 1,         // Edit room name
    Manage = 2,         // Permissions
    Invitations = 3,    // Create invites to room
    Post = 4,           // Allows user to send messages
    Delete = 5,         // Delete other's messages
    Attach = 6,         // File attachments
    Pin = 7,            // Pin specific message to pin tab
    Suspend = 8,        // Suspend users from room
    Ban = 9,            // Ban users from room permanently
    Commands = 10,      // Use slash commands (unimplemented)
    Mute = 11,          // Mute others in voice chat (unimplemented)
    Priority = 12,      // Lower others voice to boost priortized user (unimplemented)
    Video = 13,         // Show webcam video (unimplemented)
    Screenshare = 14,   // Share screen (unimplemented)
    Nickname = 15,      // Can change display name in room
    Connect = 16,       // Join voice chat (unimplemented)
    Speak = 17,         // Can speak in voice chat (unimplemented)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(transparent)]
pub struct Permissions(i64);
