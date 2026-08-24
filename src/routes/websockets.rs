use std::collections::hash_map::Entry;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::Response;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::config;
use crate::db;
use crate::routes::messages::MessageResponse;
use crate::routes::{AppState, AuthUser};

#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    Message { room_id: Uuid, message: MessageResponse },
    MessageDeleted { room_id: Uuid, message_id: Uuid },
    MessageEdited { room_id: Uuid, message_id: Uuid, body: Option<String>, edited_at: i64 },
    Invited { room_id: Uuid, invited_by: Uuid },
    Banned { room_id: Uuid },
    RoomDeleted { room_id: Uuid },
    RoomUpdated { room_id: Uuid },
    Resync,
}

/// Upgrades an HTTP request into a WebSocket.
///
/// # Arguments
///
/// * `user_id` - User the connection belongs to.
/// * `state` - Pool and socket registry.
/// * `ws` - The upgrade handshake.
pub async fn ws_handler(
    AuthUser(user_id): AuthUser,
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> Response {

    let handshake = ws.protocols(["Bearer"]);
    let start_socket = move |socket| handle_socket(socket, user_id, state);
    handshake.on_upgrade(start_socket)
}

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

    let reg = match state.registry.read() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("registry lock was poisoned, recovering");
            poisoned.into_inner()
        }
    };

    for user_id in users {
        if let Some(channel) = reg.get(user_id) {
            let _ = channel.send(payload.clone());
        }
    }
}

/// Pushes server events to one connection until it closes.
///
/// # Arguments
///
/// * `socket` - The client's connection.
/// * `user_id` - User the connection belongs to.
/// * `state` - Pool and socket registry.
async fn handle_socket(
    socket: WebSocket,
    user_id: Uuid,
    state: AppState,
) {
    let mut receiver = subscribe(&state, user_id);

    // Tells the client to re-read over HTTP, and is sent whenever this connection falls behind its channel.
    let resync = match serde_json::to_string(&ServerEvent::Resync) {
        Ok(payload) => payload,
        Err(e) => {
            tracing::error!("failed to serialize resync event: {e}");
            unsubscribe(&state, user_id, receiver);
            return;
        }
    };

    // A single connection cannot be read and written at once, so it is split
    // into two parts that can be
    let (mut outgoing, mut incoming) = socket.split();

    // Either one finishing means the connection is over
    tokio::select! {
        _ = send_events(&mut outgoing, &mut receiver, &resync, user_id) => {}
        _ = await_close(&mut incoming) => {}
    }

    unsubscribe(&state, user_id, receiver);
}

/// Returns a receiver on a user's channel, creating the channel if this is
/// their first connection.
///
/// # Arguments
///
/// * `state` - Pool and socket registry.
/// * `user_id` - User the channel belongs to.
fn subscribe(
    state: &AppState,
    user_id: Uuid,
) -> broadcast::Receiver<String> {
    let mut reg = match state.registry.write() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("registry lock was poisoned, recovering");
            poisoned.into_inner()
        }
    };

    let channel = match reg.entry(user_id) {
        Entry::Occupied(entry) => entry.into_mut(),
        Entry::Vacant(entry) => {
            let buffer = config::get().limits.message_buffer;
            let (sender, _) = broadcast::channel(buffer);
            entry.insert(sender)
        }
    };

    channel.subscribe()
}

/// Drops a receiver, and the user's channel with it once no other receiver
/// remains.
///
/// # Arguments
///
/// * `state` - Pool and socket registry.
/// * `user_id` - User the channel belongs to.
/// * `receiver` - Receiver to drop.
fn unsubscribe(
    state: &AppState,
    user_id: Uuid,
    receiver: broadcast::Receiver<String>,
) {
    drop(receiver);

    let mut reg = match state.registry.write() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("registry lock was poisoned, recovering");
            poisoned.into_inner()
        }
    };

    // No subscribe can run while this lock is held, the count cannot change before removal
    let receivers = match reg.get(&user_id) {
        Some(channel) => channel.receiver_count(),
        None => return,
    };

    if receivers == 0 {
        reg.remove(&user_id);
    }
}

/// Reads events from the user's channel and writes them through the socket.
///
/// Ends when the channel closes or a write fails.
///
/// # Arguments
///
/// * `socket` - Write half of the client's socket.
/// * `receiver` - Receiver on the user's channel.
/// * `resync` - Payload written when the channel reports overwritten entries.
/// * `user_id` - User the channel belongs to.
async fn send_events(
    socket: &mut SplitSink<WebSocket, Message>,
    receiver: &mut broadcast::Receiver<String>,
    resync: &str,
    user_id: Uuid,
) {
    loop {
        let text = match receiver.recv().await {
            Ok(text) => text,

            // The channel keeps a fixed number of recent events and overwrites
            // the earliest to make room. This connection fell far enough behind
            // that events it had not read were overwritten, so the client is
            // told to re-read over HTTP. Its place in the channel moves forward
            // to the earliest event still kept.
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                tracing::warn!("socket for {user_id} skipped {skipped} events");
                resync.to_string()
            }

            Err(broadcast::error::RecvError::Closed) => break,
        };

        if socket.send(Message::Text(text.into())).await.is_err() {
            break;
        }
    }
}

/// Ends when the client closes the connection.
///
/// Clients are expected to send nothing but a close frame, so anything else
/// that arrives is dropped.
///
/// # Arguments
///
/// * `socket` - Stream of messages arriving from the client.
async fn await_close(socket: &mut SplitStream<WebSocket>) {
    while let Some(Ok(_)) = socket.next().await {}
}

