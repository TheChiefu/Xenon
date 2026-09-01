//! Who is told what when the sidecar reports something about a linked
//! account.

use std::collections::HashSet;

use uuid::Uuid;

use crate::api;
use crate::models::Platform;
use crate::sockets::events::{GamePresence, LinkOutcome, ServerEvent};
use crate::sockets::registry;
use crate::sockets::sidecar;
use crate::state::AppState;

/// Sends the sidecar the id of every user with a link.
///
/// # Arguments
///
/// * `state` - Pool, socket registry, and the channel to the sidecar.
pub async fn send_linked_users(state: &AppState) {
    match api::linked_accounts::list_users(&state.pool, Platform::Xbox).await {
        Ok(user_ids) => sidecar::send(state, ServerEvent::LinkedAccounts { user_ids }),
        Err(e) => tracing::error!("could not read who has a link: {e}")
    }
}

/// Passes a sign-in address to the clients of the user who asked for it.
///
/// # Arguments
///
/// * `state` - Pool, socket registry, and the channel to the sidecar.
/// * `user_id` - User who asked to link.
/// * `platform` - Service being linked.
/// * `authorize_url` - Address to open to sign in.
pub fn on_link_url(
    state: &AppState,
    user_id: Uuid,
    platform: Platform,
    authorize_url: String,
) {
    let event = ServerEvent::LinkUrl { platform, authorize_url };
    registry::inform_user(state, user_id, event);
}

/// Stores a finished link attempt and tells the account it belongs to.
///
/// # Arguments
///
/// * `state` - Pool, socket registry, and the channel to the sidecar.
/// * `outcome` - How the attempt ended.
pub async fn on_link_result(state: &AppState, outcome: LinkOutcome) {
    let (user_id, platform, handle) = match outcome {
        LinkOutcome::Linked { user_id, platform, handle } => (user_id, platform, handle),
        LinkOutcome::Error { user_id, platform, message } => {
            tracing::info!("linking {platform:?} for {user_id} failed: {message}");

            let event = ServerEvent::LinkFailed { user_id, platform, message };
            registry::inform_user(state, user_id, event);
            return;
        }
    };

    if let Err(e) = api::linked_accounts::set(&state.pool, user_id, platform, &handle).await {
        tracing::error!("could not store the {platform:?} link for {user_id}: {e}");
        return;
    }

    write_reauth(state).remove(&user_id);

    let event = ServerEvent::AccountLinked { user_id, platform, handle };
    registry::inform_user(state, user_id, event);
}

/// Records that a link stopped renewing, and sends that to its owner.
///
/// # Arguments
///
/// * `state` - Pool, socket registry, and the channel to the sidecar.
/// * `user_id` - Owner of the link.
/// * `platform` - Service that stopped renewing.
pub fn on_needs_reauth(state: &AppState, user_id: Uuid, platform: Platform) {
    write_reauth(state).insert(user_id);
    registry::inform_user(state, user_id, ServerEvent::LinkNeedsReauth { user_id, platform });
}

/// Sends a presence change to everyone sharing a room with that user, which
/// is what shows under their name.
///
/// # Arguments
///
/// * `state` - Pool, socket registry, and the channel to the sidecar.
/// * `user_id` - User the presence belongs to.
/// * `platform` - Service the presence came from.
/// * `status` - How they now appear.
/// * `title` - What they are playing, unset when that is unknown.
pub async fn on_presence(
    state: &AppState,
    user_id: Uuid,
    platform: Platform,
    status: GamePresence,
    title: Option<String>,
) {
    let event = ServerEvent::GamePresenceUpdated { user_id, platform, status, title };
    registry::inform_shared_members(state, user_id, event).await;
}

/// Whether this user has a link that has to be made again.
///
/// # Arguments
///
/// * `state` - Pool, socket registry, and the channel to the sidecar.
/// * `user_id` - Owner of the link.
pub fn needs_reauth(state: &AppState, user_id: Uuid) -> bool {
    match state.needs_reauth.read() {
        Ok(guard) => guard.contains(&user_id),
        Err(poisoned) => {
            tracing::warn!("needs_reauth lock was poisoned, recovering");
            poisoned.into_inner().contains(&user_id)
        }
    }
}

// Helper Methods //

/// Takes the write lock over the users whose link stopped renewing.
///
/// # Arguments
///
/// * `state` - Pool, socket registry, and the channel to the sidecar.
fn write_reauth(state: &AppState) -> std::sync::RwLockWriteGuard<'_, HashSet<Uuid>> {
    match state.needs_reauth.write() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("needs_reauth lock was poisoned, recovering");
            poisoned.into_inner()
        }
    }
}
