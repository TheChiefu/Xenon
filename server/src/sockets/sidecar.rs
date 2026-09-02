//! The sidecar's WebSocket, from the handshake until it closes.

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast;

use crate::api;
use crate::config;
use crate::sockets::events::{ServerEvent, SidecarEvent};
use crate::sockets::game_presence;
use crate::sockets::links;
use crate::state::AppState;

// Socketing Methods //

/// Upgrades the sidecar's request into a WebSocket.
///
/// # Arguments
///
/// * `state` - Pool, socket registry, and the channel to the sidecar.
/// * `headers` - Head of the request, carrying the shared secret.
/// * `ws` - The upgrade handshake.
pub async fn handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {

    let expected = &config::get().sidecar.secret;

    // An unset secret refuses every connection
    if expected.is_empty() {
        return StatusCode::FORBIDDEN.into_response();
    }

    let sent = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));

    match sent {
        Some(sent) if matches(sent, expected) => {
            ws.on_upgrade(move |socket| handle_socket(socket, state))
        }
        _ => StatusCode::FORBIDDEN.into_response()
    }
}

// Helper Methods //

/// Runs the connection until either direction ends.
///
/// # Arguments
///
/// * `socket` - The sidecar's connection.
/// * `state` - Pool, socket registry, and the channel to the sidecar.
async fn handle_socket(socket: WebSocket, state: AppState) {
    tracing::info!("the sidecar connected");

    let mut events = state.to_sidecar.subscribe();
    let (mut outgoing, mut incoming) = socket.split();

    tokio::select! {
        _ = send_events(&mut outgoing, &mut events) => {}
        _ = read_events(&mut incoming, &state) => {}
    }

    // Nothing reports game presence when the sidecar gone
    game_presence::clear_all(&state);
}

/// Writes each event to the socket, returning when it cannot.
///
/// # Arguments
///
/// * `socket` - Write half of the sidecar's connection.
/// * `events` - Events to write.
async fn send_events(
    socket: &mut SplitSink<WebSocket, Message>,
    events: &mut broadcast::Receiver<String>,
) {
    loop {
        let payload = match events.recv().await {
            Ok(payload) => payload,

            // The channel keeps a fixed number of recent events and overwrites
            // the earliest to make room. These were overwritten before the
            // sidecar read them, and nothing stores them, so they are lost.
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                tracing::warn!("the sidecar skipped {skipped} events");
                continue;
            }

            Err(broadcast::error::RecvError::Closed) => break
        };

        if socket.send(Message::Text(payload.into())).await.is_err() {
            break;
        }
    }
}

/// Reads the sidecar's events until the connection closes.
///
/// # Arguments
///
/// * `socket` - Read half of the sidecar's connection.
/// * `state` - Pool, socket registry, and the channel to the sidecar.
async fn read_events(socket: &mut SplitStream<WebSocket>, state: &AppState) {
    while let Some(Ok(message)) = socket.next().await {

        let Message::Text(text) = message else {
            continue;
        };

        let event: SidecarEvent = match serde_json::from_str(&text) {
            Ok(event) => event,
            Err(e) => {
                tracing::warn!("the sidecar sent an event that did not parse: {e}");
                continue;
            }
        };

        match event {
            SidecarEvent::VapidKey { public_key } => {
                if let Err(e) = api::push::set_key(&state.pool, &public_key).await {
                    tracing::error!("could not store the push public key: {e}");
                }
            }
            SidecarEvent::GetLinkedAccounts => links::send_linked_users(state).await,
            SidecarEvent::LinkUrl { user_id, platform, authorize_url } => {
                links::on_link_url(state, user_id, platform, authorize_url)
            }
            SidecarEvent::LinkResult { outcome } => links::on_link_result(state, outcome).await,
            SidecarEvent::NeedsReauth { user_id, platform } => {
                links::on_needs_reauth(state, user_id, platform)
            }
            SidecarEvent::Presence { user_id, game } => {
                game_presence::on_report(state, user_id, game).await
            }
        }
    }
}

/// Serializes an event and offers it to the sidecar's connection. Dropped
/// when no sidecar is connected.
///
/// # Arguments
///
/// * `state` - Pool, socket registry, and the channel to the sidecar.
/// * `event` - Event to send.
pub fn send(state: &AppState, event: ServerEvent) {
    match serde_json::to_string(&event) {
        Ok(payload) => { let _ = state.to_sidecar.send(payload); }
        Err(e) => tracing::error!("failed to serialize a sidecar event: {e}")
    }
}

/// Whether two secrets are the same, comparing every byte so the time taken
/// does not reveal how many matched.
///
/// # Arguments
///
/// * `sent` - What the request presented.
/// * `expected` - What the config holds.
fn matches(sent: &str, expected: &str) -> bool {
    if sent.len() != expected.len() {
        return false;
    }

    let mut differences = 0u8;
    for i in 0..expected.len() {
        differences |= sent.as_bytes()[i] ^ expected.as_bytes()[i];
    }

    differences == 0
}
