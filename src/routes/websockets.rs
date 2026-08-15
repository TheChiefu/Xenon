use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::Response;
use tokio::sync::broadcast;
use futures_util::{SinkExt, StreamExt};
use uuid::Uuid;
use serde::{Serialize};
use tracing;

use crate::config;
use crate::db;
use crate::routes::messages::MessageResponse;
use crate::routes::{AuthUser, AppState};

// Data Structs //

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    Message { room_id: Uuid, message: MessageResponse},
    MessageDeleted {room_id: Uuid, message_id: Uuid},
    MessageEdited {room_id: Uuid, message_id: Uuid, body: Option<String>, edited_at: i64}
}

// Socketing Methods //

/// Upgrades an HTTP request into a WebSocket.
///
/// The request arrives as a GET carrying upgrade headers, so AuthUser runs on
/// it like any other route and the connection is authenticated for its whole
/// lifetime. Returns once the upgrade is agreed, leaving handle_socket running
/// in its own task.
/// - AuthUser: Who the socket belongs to
/// - state: Pool and socket registry
/// - ws: The upgrade handshake
pub async fn ws_handler(
    AuthUser(user_id): AuthUser,
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> Response {
    // A handshake offering subprotocols only succeeds if the server names one
    // of them back. Echo "Bearer", never the token that follows it.
    ws.protocols(["Bearer"])
        .on_upgrade(move |socket| handle_socket(socket, user_id, state))
}

/// Pushes server notifications to one connected client until they disconnect.
///
/// Everything the client sends is discarded. Joining rooms and posting messages
/// happen over the HTTP endpoints, and this socket carries only what the server
/// has to tell the client.
async fn handle_socket(
    socket: WebSocket,
    user_id: Uuid,
    state: AppState
) {

    // Each connected user has one broadcast channel in the registry.
    // Take a receiver on theirs, creating a channel on their first connection.
    let mut rx = {
        let mut reg = match state.registry.write() {
            Ok(guard) => guard,
            Err(poisoned) => {
                tracing::warn!("registry lock was poisoned, recovering");
                poisoned.into_inner()
            }
        };
        
        let tx = reg
            .entry(user_id)
            .or_insert_with(|| broadcast::channel(config::get().socket.message_buffer).0);
        tx.subscribe()
    }; // Write lock released here, before the long-lived loop below

    // Split socket to handle read/write separately
    let (mut sender, mut receiver) = socket.split();

    // Take messages off of broadcast channel and write them to the socket
    let mut send_task = tokio::spawn(async move {
        while let Ok(text) = rx.recv().await {
            if sender.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    // Void what client sends / discard it (socket only sends data not retrieves)
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(_)) = receiver.next().await {}
    });

    // Either half finishing means the connection is done (close it)
    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }

}

/// Sends an event to every member of a room with an open socket.
/// Failures are logged rather than returend, so a broadcast doesn't fail on a request.
/// - state: Pool and socket registry
/// - room_id: Room whose members recevive the event
/// - event: What to broadcast
pub async fn broadcast(
    state: &AppState,
    room_id: Uuid,
    event: ServerEvent
) {

    // Seralize event for broadcast
    match serde_json::to_string(&event) {
        Ok(payload) => {

            // Attempt to acquire connection
            let mut conn = match state.pool.acquire().await {
                Ok(conn) => conn,
                Err(e) => {
                    tracing::error!("broadcast could not acquire a connection: {e}");
                    return;
                }
            };

            // Attempt to retrieve all members of a given room
            let members = match db::room_member_ids(&mut conn, room_id).await {
                Ok(mem) => mem,
                Err(e) => {
                    tracing::error!("could not acquire room members: {e}");
                    return;
                }
            };

            // Get the registry lock
            let reg = match state.registry.read() {
                Ok(guard) => guard,
                Err(poisoned) => {
                    tracing::warn!("registry lock was poisoned, recovering");
                    poisoned.into_inner()
                }
            };

            // Iterate over each member and send them the message
            for member in members {
                if let Some(tx) = reg.get(&member) {
                    let _ = tx.send(payload.clone());
                }
            }
        },

        // Could not serialize
        Err(e) => tracing::error!("failed to serialize server event: {e}"),
    };
}