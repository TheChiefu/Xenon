//! What a viewer is told about a user, and who is told it.

use uuid::Uuid;

use serde::{Deserialize, Serialize};

use crate::db;
use crate::error::Result;
use crate::models::Status;
use crate::sockets::events::{ServerEvent, UserPresence};
use crate::sockets::registry;
use crate::state::AppState;

/// What a viewer is told about someone, from their [`Status`] and whether they
/// hold a connection. Invisible reads as Offline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Presence {
    Offline,
    Online,
    Busy,
    Away
}

/// What a user is connected from, for a client to show beside their presence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Device {
    Windows,
    Macos,
    Linux,
    Android,
    Ios
}

/// Reads the status a user's connections start at.
///
/// # Arguments
///
/// * `state` - Pool and socket registry.
/// * `user_id` - User to read.
pub async fn preferred_status(
    state: &AppState,
    user_id: Uuid,
) -> Result<Status> {

    let mut conn = state.pool.acquire().await?;
    db::preferred_status(&mut conn, user_id).await
}

/// Builds what a connecting user is told about everyone they share a room
/// with, leaving out any who read as Offline.
///
/// # Arguments
///
/// * `state` - Pool and socket registry.
/// * `user_id` - User the snapshot is built for.
pub async fn snapshot(
    state: &AppState,
    user_id: Uuid,
) -> Result<Vec<UserPresence>> {

    // Who the user may be told about
    let mut conn = state.pool.acquire().await?;
    let members = db::shared_room_member_ids(&mut conn, user_id).await?;

    let mut users = Vec::new();
    for declared in registry::statuses_of(state, &members) {
        let presence = from_status(Some(declared.status));

        // An Invisible member is left out, the same as one holding no connection
        if presence == Presence::Offline {
            continue;
        }

        users.push(UserPresence {
            user_id: declared.user_id,
            presence,
            device: declared.device
        });
    }

    Ok(users)
}

/// What a viewer is told about a user.
///
/// # Arguments
///
/// * `status` - Status the user's connection declares, or `None` when they
///   hold no connection.
pub fn from_status(status: Option<Status>) -> Presence {
    match status {
        Some(Status::Online) => Presence::Online,
        Some(Status::Busy) => Presence::Busy,
        Some(Status::Away) => Presence::Away,
        Some(Status::Invisible) | None => Presence::Offline
    }

}

/// Sends the presence a join changes on both sides: the room is told the new
/// member's presence, and the new member is sent a snapshot covering everyone
/// now visible.
///
/// # Arguments
///
/// * `state` - Pool and socket registry.
/// * `user_id` - User who joined.
pub async fn on_join(
    state: &AppState,
    user_id: Uuid,
) {
    // Nothing to send for a user holding no connection
    let declared = registry::statuses_of(state, &[user_id]);
    let Some(entry) = declared.first() else {
        return;
    };

    // No previous status: the room could not see the new member until now
    on_change(state, user_id, None, Some(entry.status)).await;

    // Covers everyone the new member now shares a room with
    match snapshot(state, user_id).await {
        Ok(users) => {
            let event = ServerEvent::PresenceSnapshot { users };
            registry::inform_user(state, user_id, event);
        }
        Err(e) => tracing::error!("could not build a presence snapshot for {user_id}: {e}")
    }
}

/// Tells everyone sharing a room with a user what to show for that user, when
/// it differs from what was shown before.
///
/// # Arguments
///
/// * `state` - Pool and socket registry.
/// * `user_id` - User whose status changed.
/// * `before` - Status they held, or `None` before they connected.
/// * `after` - Status they hold now, or `None` once disconnected.
pub async fn on_change(
    state: &AppState,
    user_id: Uuid,
    before: Option<Status>,
    after: Option<Status>,
) {
    // An Invisible user connecting or leaving is not a change to a viewer
    let presence = from_status(after);
    if presence == from_status(before) {
        return;
    }

    let mut conn = match state.pool.acquire().await {
        Ok(conn) => conn,
        Err(e) => {
            tracing::error!("presence could not acquire a connection: {e}");
            return;
        }
    };

    let members = match db::shared_room_member_ids(&mut conn, user_id).await {
        Ok(members) => members,
        Err(e) => {
            tracing::error!("could not read who shares a room with {user_id}: {e}");
            return;
        }
    };

    // Empty once they hold no connection, which is what going offline is
    let declared = registry::statuses_of(state, &[user_id]);
    let device = match declared.first() {
        Some(entry) => entry.device,
        None => None
    };

    let event = ServerEvent::PresenceUpdated { user_id, presence, device };
    registry::inform_users(state, &members, event);
}
