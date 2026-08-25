//! The map of connected users, and sending events to them.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use tokio::sync::broadcast;
use uuid::Uuid;

use crate::config;
use crate::db;
use crate::models::Status;
use crate::sockets::events::ServerEvent;
use crate::sockets::presence::ClientStatus;
use crate::state::AppState;

// Data Structs //

/// One connected user
pub struct Connected {
    /// The writing end of a channel every socket the user holds reads
    pub events: broadcast::Sender<String>,

    /// What the connection that last declared won, starting at
    /// `users.preferred_status` with no device
    pub client_status: ClientStatus
}

/// One entry per connected user, keyed by user id.
///
/// Room membership stays in `room_access` and is read at broadcast time.
///
/// - Arc: Counted pointer, so every clone of AppState reads and writes to the same map
/// - RwLock: (Many readers/one writer mutex) Connects and disconnects write, broadcasts read
/// - broadcast: a single send reaches every subscriber. Sockets subscribe on connect and their
///   subscription ends when they drop. The map keeps the channel after its
///   last subscriber leaves.
pub type Registry = Arc<RwLock<HashMap<Uuid, Connected>>>;

/// Whether a socket is the only one its user holds.
#[derive(PartialEq, Eq)]
pub enum Joined {
    First,
    Additional
}

// Registry Methods //

/// Returns a receiver on a user's channel, creating the entry if this is their
/// first connection.
///
/// # Arguments
///
/// * `state` - Pool and socket registry.
/// * `user_id` - User the channel belongs to.
/// * `client_status` - What a first connection starts at. A later one joins
///   the entry as it stands, keeping whatever the user has since declared.
pub fn subscribe(
    state: &AppState,
    user_id: Uuid,
    client_status: ClientStatus,
) -> (broadcast::Receiver<String>, Joined) {
    let mut reg = write_lock(state);

    let mut joined = Joined::Additional;
    let connected = match reg.entry(user_id) {

        // The user already has a channel
        Entry::Occupied(entry) => entry.into_mut(),

        // The user has no channel, create one and initial prefered status of user
        Entry::Vacant(entry) => {
            joined = Joined::First;
            let buffer = config::get().limits.message_buffer;
            let (sender, _) = broadcast::channel(buffer);
            entry.insert(Connected { events: sender, client_status })
        }
    };

    (connected.events.subscribe(), joined)
}

/// Drops a receiver, and the user's entry with it once no other receiver
/// remains, returning the status that entry held.
///
/// # Arguments
///
/// * `state` - Pool and socket registry.
/// * `user_id` - User the channel belongs to.
/// * `receiver` - Receiver to drop.
pub fn unsubscribe(
    state: &AppState,
    user_id: Uuid,
    receiver: broadcast::Receiver<String>,
) -> Option<Status> {
    drop(receiver);

    let mut reg = write_lock(state);

    // No subscribe can run while this lock is held, the count cannot change before removal
    let receivers = match reg.get(&user_id) {
        Some(connected) => connected.events.receiver_count(),
        None => return None,
    };

    if receivers > 0 {
        return None;
    }

    reg.remove(&user_id).map(|connected| connected.client_status.status)
}

/// Reads what the given users are declaring, leaving out any holding no
/// connection.
///
/// # Arguments
///
/// * `state` - Pool and socket registry.
/// * `users` - Users to look up.
pub fn statuses_of(
    state: &AppState,
    users: &[Uuid],
) -> Vec<(Uuid, ClientStatus)> {
    let reg = read_lock(state);

    let mut found = Vec::new();
    for user_id in users {
        if let Some(connected) = reg.get(user_id) {
            found.push((*user_id, connected.client_status));
        }
    }

    found
}

/// Writes the status a user's connections declare, returning the previous one,
/// or `None` when the user holds no connection.
///
/// # Arguments
///
/// * `state` - Pool and socket registry.
/// * `user_id` - User to write.
/// * `status` - Status the connections now declare.
pub fn set_status(
    state: &AppState,
    user_id: Uuid,
    status: Status,
) -> Option<Status> {
    let mut reg = write_lock(state);
    let connected = reg.get_mut(&user_id)?;

    let previous = connected.client_status.status;
    connected.client_status.status = status;

    Some(previous)
}

// Sending Methods //

/// Sends an event to every member of a room.
///
/// # Arguments
///
/// * `state` - Pool and socket registry.
/// * `room_id` - Room whose members receive the event.
/// * `event` - What to send.
pub async fn broadcast(
    state: &AppState,
    room_id: Uuid,
    event: ServerEvent,
) {
    let mut conn = match state.pool.acquire().await {
        Ok(conn) => conn,
        Err(e) => {
            tracing::error!("broadcast could not acquire a connection: {e}");
            return;
        }
    };

    let members = match db::room_member_ids(&mut conn, room_id).await {
        Ok(members) => members,
        Err(e) => {
            tracing::error!("could not acquire room members: {e}");
            return;
        }
    };

    notify_users(state, &members, event);
}

/// Sends an event to one user.
///
/// # Arguments
///
/// * `state` - Pool and socket registry.
/// * `user_id` - User who receives the event.
/// * `event` - What to send.
pub fn notify_user(
    state: &AppState,
    user_id: Uuid,
    event: ServerEvent,
) {
    notify_users(state, &[user_id], event);
}

/// Sends an event to a list of users.
///
/// # Arguments
///
/// * `state` - Pool and socket registry.
/// * `users` - Users who receive the event.
/// * `event` - What to send.
pub fn notify_users(
    state: &AppState,
    users: &[Uuid],
    event: ServerEvent,
) {
    let payload = match serde_json::to_string(&event) {
        Ok(payload) => payload,
        Err(e) => {
            tracing::error!("failed to serialize server event: {e}");
            return;
        }
    };

    let reg = read_lock(state);

    for user_id in users {
        if let Some(connected) = reg.get(user_id) {
            let _ = connected.events.send(payload.clone());
        }
    }
}

// Helper Methods //

/// Takes the read lock, recovering it if a writer panicked while holding it.
fn read_lock(state: &AppState) -> std::sync::RwLockReadGuard<'_, HashMap<Uuid, Connected>> {
    match state.registry.read() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("registry lock was poisoned, recovering");
            poisoned.into_inner()
        }
    }
}

/// Takes the write lock, recovering it if a writer panicked while holding it.
fn write_lock(state: &AppState) -> std::sync::RwLockWriteGuard<'_, HashMap<Uuid, Connected>> {
    match state.registry.write() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("registry lock was poisoned, recovering");
            poisoned.into_inner()
        }
    }
}
