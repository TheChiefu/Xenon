//! One client's WebSocket, from the handshake until it closes.

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::response::Response;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::models::Status;
use crate::routes::AuthUser;
use crate::sockets::events::{ClientEvent, ServerEvent};
use crate::sockets::presence::{self, Device};
use crate::sockets::registry::{self, Joined, SocketDevice};
use crate::state::AppState;

// Socketing Methods //

/// Query string the handshake carries, read before the upgrade while the
/// request is still an ordinary GET.
#[derive(Deserialize)]
pub struct ConnectQuery {
    /// What the client is running on, shown beside the user's presence. A
    /// client that names none still connects.
    #[serde(default)]
    pub device: Option<Device>
}

/// Upgrades an HTTP request into a WebSocket.
///
/// # Arguments
///
/// * `user_id` - User the connection belongs to.
/// * `state` - Pool and socket registry.
/// * `query` - What the client is connecting from.
/// * `ws` - The upgrade handshake.
pub async fn ws_handler(
    AuthUser(user_id, ..): AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ConnectQuery>,
    ws: WebSocketUpgrade,
) -> Response {

    let handshake = ws.protocols(["Bearer"]);
    let start_socket = move |socket| handle_socket(socket, user_id, query.device, state);
    handshake.on_upgrade(start_socket)
}

// Helper Methods //

/// Pushes server events to one connection until it closes.
///
/// # Arguments
///
/// * `socket` - The client's connection.
/// * `user_id` - User the connection belongs to.
/// * `device` - What the client is running on.
/// * `state` - Pool and socket registry.
async fn handle_socket(
    socket: WebSocket,
    user_id: Uuid,
    device: Option<Device>,
    state: AppState,
) {
    // Where the connection starts, before the client declares anything
    let status = match presence::preferred_status(&state, user_id).await {
        Ok(status) => status,
        Err(e) => {
            tracing::error!("could not read the status for {user_id}: {e}");
            Status::Online
        }
    };

    // Identifies this socket's entry in the user's device list
    let socket_id = registry::next_socket_id();
    let device = match device {
        Some(device) => Some(SocketDevice { socket_id, device }),
        None => None
    };

    let (mut receiver, joined) = registry::subscribe(&state, user_id, status, device);

    // A later socket joins a user who was already connected
    if joined == Joined::First {
        presence::on_change(&state, user_id, None, Some(status)).await;
    }

    // Tells the client to re-read over HTTP, and is sent whenever this connection falls behind its channel.
    let resync = match serde_json::to_string(&ServerEvent::Resync) {
        Ok(payload) => payload,
        Err(e) => {
            tracing::error!("failed to serialize resync event: {e}");
            registry::unsubscribe(&state, user_id, socket_id, receiver);
            return;
        }
    };

    // A single connection cannot be read and written at once, so it is split
    // into two parts that can be
    let (mut outgoing, mut incoming) = socket.split();

    send_snapshot(&state, user_id, &mut outgoing).await;

    // Either one finishing means the connection is over
    tokio::select! {
        _ = send_events(&mut outgoing, &mut receiver, &resync, user_id, &state) => {}
        _ = read_events(&mut incoming, user_id, &state) => {}
    }

    // A status comes back only when this was the user's last connection
    if let Some(last) = registry::unsubscribe(&state, user_id, socket_id, receiver) {
        presence::on_change(&state, user_id, Some(last), None).await;
    }
}


/// Writes the presence of everyone visible to a socket, before any update can
/// reach it.
///
/// A failure is logged and the connection carries on: the client is left
/// without an initial picture rather than dropped.
///
/// # Arguments
///
/// * `state` - Pool and socket registry.
/// * `viewer_id` - User the connection belongs to.
/// * `socket` - Write half of the client's socket.
async fn send_snapshot(
    state: &AppState,
    viewer_id: Uuid,
    socket: &mut SplitSink<WebSocket, Message>,
) {
    let users = match presence::snapshot(state, viewer_id).await {
        Ok(users) => users,
        Err(e) => {
            tracing::error!("could not build a presence snapshot for {viewer_id}: {e}");
            return;
        }
    };

    let payload = match serde_json::to_string(&ServerEvent::PresenceSnapshot { users }) {
        Ok(payload) => payload,
        Err(e) => {
            tracing::error!("failed to serialize the presence snapshot: {e}");
            return;
        }
    };

    let _ = socket.send(Message::Text(payload.into())).await;
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
/// * `app_state` - State of application connections
async fn send_events(
    socket: &mut SplitSink<WebSocket, Message>,
    receiver: &mut broadcast::Receiver<String>,
    resync: &str,
    user_id: Uuid,
    app_state: &AppState,
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
                send_snapshot(app_state, user_id, socket).await;
                resync.to_string()
            }

            Err(broadcast::error::RecvError::Closed) => break,
        };

        if socket.send(Message::Text(text.into())).await.is_err() {
            break;
        }
    }
}

/// Applies what a client sends until it closes the connection.
///
/// A frame that does not parse is logged and skipped, leaving the connection
/// open.
///
/// # Arguments
///
/// * `socket` - Stream of messages arriving from the client.
/// * `user_id` - User the connection belongs to.
/// * `state` - Pool and socket registry.
async fn read_events(
    socket: &mut SplitStream<WebSocket>,
    user_id: Uuid,
    state: &AppState,
) {
    while let Some(Ok(message)) = socket.next().await {

        // Only text frames carry an event
        let Message::Text(text) = message else {
            continue;
        };

        let event = match serde_json::from_str::<ClientEvent>(&text) {
            Ok(event) => event,
            Err(e) => {
                tracing::debug!("socket for {user_id} sent an unreadable event: {e}");
                continue;
            }
        };

        match event {
            ClientEvent::Status { status } => {
                if let Some(previous) = registry::set_status(state, user_id, status) {
                    presence::on_change(state, user_id, Some(previous), Some(status)).await;
                }
            }
        }
    }
}


