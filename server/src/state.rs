//! What every handler is given:
//! - the database
//! - who is connected
//! - any sidecars

use axum::extract::FromRef;
use sqlx::SqlitePool;
use tokio::sync::broadcast;

use crate::config;
use crate::sockets::registry::Registry;

/// Cloned per request.
///
/// `FromRef` generates a per-field accessor, letting handlers keep asking for
/// `State<SqlitePool>`.
#[derive(Clone, FromRef)]
pub struct AppState {
    pub pool: SqlitePool,
    pub registry: Registry,

    /// Push events: read by the push sidecar's connection
    pub push_channel: broadcast::Sender<String>
}

impl AppState {

    /// Builds the state a router is handed.
    ///
    /// # Arguments
    ///
    /// * `pool` - Pool of SQL connections.
    pub fn new(pool: SqlitePool) -> Self {

        // Create the broadcast channel push events are sent on
        let (push_channel, _) = broadcast::channel(config::get().limits.message_buffer);

        AppState {
            pool,
            registry: Registry::default(),
            push_channel
        }
    }
}
