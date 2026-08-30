//! The map of connected users, and sending events to them.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use tokio::sync::broadcast;
use uuid::Uuid;

use crate::config;
use crate::db;
use crate::models::Status;
use crate::sockets::events::ServerEvent;
use crate::sockets::presence::Device;
use crate::state::AppState;

// Data Structs //

/// One connected user
pub struct Connected {
    /// The writing end of a channel every socket the user holds reads
    pub events: broadcast::Sender<String>,

    /// Starts at `users.preferred_status`, then whatever the user declares
    pub status: Status,

    /// One entry per live socket that named a device, in connect order
    pub devices: Vec<SocketDevice>
}

/// What one socket named on connect, kept until that socket closes
pub struct SocketDevice {
    pub socket_id: u64,
    pub device: Device
}

/// What one user declares, before a viewer is told anything of it
#[derive(Debug)]
pub struct UserStatus {
    pub user_id: Uuid,
    pub status: Status,
    pub device: Option<Device>
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

/// Handed out once per socket, so a disconnect can find the entry it added.
static NEXT_SOCKET_ID: AtomicU64 = AtomicU64::new(0);

// Registry Methods //

/// Takes the next socket id. A socket holds its own for as long as it is open.
pub fn next_socket_id() -> u64 {
    NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed)
}

/// Returns a receiver on a user's channel, creating the entry if this is their
/// first connection.
///
/// # Arguments
///
/// * `state` - Pool and socket registry.
/// * `user_id` - User the channel belongs to.
/// * `status` - What a first connection starts at. A later one joins the entry
///   as it stands, keeping whatever the user has since declared.
/// * `device` - What this socket named on connect, if anything.
pub fn subscribe(
    state: &AppState,
    user_id: Uuid,
    status: Status,
    device: Option<SocketDevice>,
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
            entry.insert(Connected { events: sender, status, devices: Vec::new() })
        }
    };

    if let Some(device) = device {
        connected.devices.push(device);
    }

    (connected.events.subscribe(), joined)
}

/// Drops a receiver, and the user's entry with it once no other receiver
/// remains, returning the status that entry held.
///
/// # Arguments
///
/// * `state` - Pool and socket registry.
/// * `user_id` - User the channel belongs to.
/// * `socket_id` - Identifies the entry this socket added to the device list.
/// * `receiver` - Receiver to drop.
pub fn unsubscribe(
    state: &AppState,
    user_id: Uuid,
    socket_id: u64,
    receiver: broadcast::Receiver<String>,
) -> Option<Status> {
    drop(receiver);

    let mut reg = write_lock(state);

    let connected = match reg.get_mut(&user_id) {
        Some(connected) => connected,
        None => return None,
    };

    // Remove the entry this socket added, if it added one
    for i in 0..connected.devices.len() {
        if connected.devices[i].socket_id == socket_id {
            connected.devices.remove(i);
            break;
        }
    }

    // This socket's receiver is dropped above, so the count is the write sockets left
    if connected.events.receiver_count() > 0 {
        return None;
    }

    // Remove connection with no more sockets are left
    reg.remove(&user_id).map(|connected| connected.status)
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
) -> Vec<UserStatus> {
    let reg = read_lock(state);

    let mut found = Vec::new();
    for user_id in users {
        if let Some(connected) = reg.get(user_id) {

            // The device most recently connected from
            let device = match connected.devices.last() {
                Some(last) => Some(last.device),
                None => None
            };

            found.push(UserStatus {
                user_id: *user_id,
                status: connected.status,
                device
            });
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

    let previous = connected.status;
    connected.status = status;

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

    inform_users(state, &members, event);
}

/// Sends an event to one user.
///
/// # Arguments
///
/// * `state` - Pool and socket registry.
/// * `user_id` - User who receives the event.
/// * `event` - What to send.
pub fn inform_user(
    state: &AppState,
    user_id: Uuid,
    event: ServerEvent,
) {
    inform_users(state, &[user_id], event);
}

/// Sends an event to a list of users.
///
/// # Arguments
///
/// * `state` - Pool and socket registry.
/// * `users` - Users who receive the event.
/// * `event` - What to send.
pub fn inform_users(
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

/// Reads which of the given users are not connected.
pub fn offline_users(
    state: &AppState,
    users: &[Uuid],
) -> Vec<Uuid> {
    let reg = read_lock(state);

    let mut found = Vec::new();
    for user_id in users {
        if !reg.contains_key(user_id) {
            found.push(*user_id);
        }
    }

    found
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
