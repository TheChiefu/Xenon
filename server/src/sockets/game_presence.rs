//! What each linked account last reported, and who is told it.

use std::collections::HashMap;

use uuid::Uuid;

use crate::db;
use crate::error::Result;
use crate::models::Status;
use crate::sockets::events::{GameActivity, GamePresence, ServerEvent, UserGamePresence};
use crate::sockets::presence;
use crate::sockets::registry;
use crate::state::AppState;

/// Stores what a linked account reported and sends it to everyone sharing a
/// room with its owner.
///
/// # Arguments
///
/// * `state` - Pool, socket registry, and the channel to the sidecar.
/// * `user_id` - User the linked account belongs to.
/// * `game` - What the account reported.
pub async fn on_report(
    state: &AppState,
    user_id: Uuid,
    game: GameActivity,
) {
    // Their stored preferred status or "Online" if not set
    let stored = match presence::preferred_status(state, user_id).await {
        Ok(status) => status,
        Err(e) => {
            tracing::error!("could not read the status for {user_id}: {e}");
            Status::Online
        }
    };

    // Invisible users have no game presence, remove entry
    if stored == Status::Invisible {
        write_map(state).remove(&user_id);
        return;
    }

    match game.status {
        GamePresence::Offline => write_map(state).remove(&user_id),
        _ => write_map(state).insert(user_id, game.clone())
    };

    let event = ServerEvent::GamePresenceUpdated { user_id, game };
    registry::inform_shared_members(state, user_id, event).await;
}

/// Builds what a connecting user is told about the linked accounts of everyone
/// they share a room with. Only members on one are named.
///
/// # Arguments
///
/// * `state` - Pool, socket registry, and the channel to the sidecar.
/// * `user_id` - User the snapshot is built for.
pub async fn snapshot(
    state: &AppState,
    user_id: Uuid,
) -> Result<Vec<UserGamePresence>> {

    // Who the user may be told about
    let mut conn = state.pool.acquire().await?;
    let members = db::shared_room_member_ids(&mut conn, user_id).await?;

    let held = read_map(state);

    let mut users = Vec::new();
    for member_id in members {
        let Some(game) = held.get(&member_id) else {
            continue;
        };

        users.push(UserGamePresence { user_id: member_id, game: game.clone() });
    }

    Ok(users)
}

/// Removes a user's game presence and tells everyone sharing a room with them
/// that their linked account is on nothing.
///
/// # Arguments
///
/// * `state` - Pool, socket registry, and the channel to the sidecar.
/// * `user_id` - User whose status became Invisible.
pub async fn clear(state: &AppState, user_id: Uuid) {
    let removed = write_map(state).remove(&user_id);
    let Some(game) = removed else {
        return;
    };

    let event = ServerEvent::GamePresenceUpdated {
        user_id,
        game: GameActivity {
            platform: game.platform,
            status: GamePresence::Offline,
            title: None,
            activity: None
        }
    };
    registry::inform_shared_members(state, user_id, event).await;
}

/// Empties what every linked account last reported.
pub fn clear_all(state: &AppState) {
    write_map(state).clear();
}

// Helper Methods //

/// Read what each linked account last reported (RwLock)
fn read_map(
    state: &AppState,
) -> std::sync::RwLockReadGuard<'_, HashMap<Uuid, GameActivity>> {
    match state.game_presence.read() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("game_presence lock was poisoned, recovering");
            poisoned.into_inner()
        }
    }
}

/// Write what each linked account last reported (RwLock)
fn write_map(
    state: &AppState,
) -> std::sync::RwLockWriteGuard<'_, HashMap<Uuid, GameActivity>> {
    match state.game_presence.write() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("game_presence lock was poisoned, recovering");
            poisoned.into_inner()
        }
    }
}
