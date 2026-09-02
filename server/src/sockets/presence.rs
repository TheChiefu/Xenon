//! What is sent to a viewer about a user, and who it is sent to.

use uuid::Uuid;

use serde::{Deserialize, Serialize};

use crate::db;
use crate::error::Result;
use crate::models::Status;
use crate::sockets::events::{ServerEvent, UserPresence};
use crate::sockets::game_presence;
use crate::sockets::registry;
use crate::state::AppState;

/// How a user appears to a viewer, from their [`Status`] and whether
/// they have a connection (Invisible appears as Offline).
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
    Ios,
    Xbox
}

/// Reads the status a user's connections start at.
pub async fn preferred_status(
    state: &AppState,
    user_id: Uuid,
) -> Result<Status> {

    let mut conn = state.pool.acquire().await?;
    db::preferred_status(&mut conn, user_id).await
}

/// Builds the presence list a connecting user is sent. It contains everyone
/// they share a room with, apart from anyone reported as Offline.
pub async fn snapshot(
    state: &AppState,
    user_id: Uuid,
) -> Result<Vec<UserPresence>> {

    // The members whose presence this user may see
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

/// The presence a viewer sees for a given status.
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

/// Sends the new member's presence to the room
/// and a snapshot of everyone visible to the new member.
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

    // A failed snapshot is logged, and the member joins with no presence list
    match snapshot(state, user_id).await {
        Ok(users) => {
            let event = ServerEvent::PresenceSnapshot { users };
            registry::inform_user(state, user_id, event);
        }
        Err(e) => tracing::error!("could not build a presence snapshot for {user_id}: {e}")
    }
}

/// Sends a user's new presence to everyone sharing a room with them, when it
/// differs from the presence they had before.
///
/// # Arguments
///
/// * `before` - Status they held, or `None` before they connected.
/// * `after` - Status they hold now, or `None` once disconnected.
pub async fn on_change(
    state: &AppState,
    user_id: Uuid,
    before: Option<Status>,
    after: Option<Status>,
) {
    // Invisible users have no game presence
    if after == Some(Status::Invisible) && before != Some(Status::Invisible) {
        game_presence::clear(state, user_id).await;
    }

    // An Invisible user connecting or leaving is not a change to a viewer
    let presence = from_status(after);
    if presence == from_status(before) {
        return;
    }

    // A user with no connection has no device
    let declared = registry::statuses_of(state, &[user_id]);
    let device = match declared.first() {
        Some(entry) => entry.device,
        None => None
    };

    let event = ServerEvent::PresenceUpdated { user_id, presence, device };
    registry::inform_shared_members(state, user_id, event).await;
}
