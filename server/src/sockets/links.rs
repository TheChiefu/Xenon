//! Who is sent what when the sidecar reports something about a linked
//! account.

use std::collections::HashSet;

use uuid::Uuid;

use crate::api;
use crate::models::Platform;
use crate::sockets::events::{LinkOutcome, ServerEvent};
use crate::sockets::registry;
use crate::sockets::sidecar;
use crate::state::AppState;

/// Sends the sidecar the id of every user with a link
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
/// * `user_id` - User who asked to link.
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

/// Stores a finished link attempt and sends the outcome to the user it belongs to
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

/// Records that a link stopped renewing, and sends a LinkNeedsReauth to its owner
pub fn on_needs_reauth(state: &AppState, user_id: Uuid, platform: Platform) {
    write_reauth(state).insert(user_id);
    registry::inform_user(state, user_id, ServerEvent::LinkNeedsReauth { user_id, platform });
}

/// Whether this user has a link that has to be made again
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

/// Write which users have a link that stopped renewing (RwLock)
fn write_reauth(state: &AppState) -> std::sync::RwLockWriteGuard<'_, HashSet<Uuid>> {
    match state.needs_reauth.write() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("needs_reauth lock was poisoned, recovering");
            poisoned.into_inner()
        }
    }
}
