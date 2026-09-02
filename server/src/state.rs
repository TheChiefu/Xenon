//! What every handler is given:
//! - the database
//! - who is connected
//! - any sidecars

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use axum::extract::FromRef;
use sqlx::SqlitePool;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::config;
use crate::sockets::events::GameActivity;
use crate::sockets::registry::Registry;

/// Cloned per request.
///
/// `FromRef` generates a per-field accessor, letting handlers keep asking for
/// `State<SqlitePool>`.
#[derive(Clone, FromRef)]
pub struct AppState {
    pub pool: SqlitePool,
    pub registry: Registry,

    /// Jobs for the sidecar: read by the sidecar's connection
    pub to_sidecar: broadcast::Sender<String>,

    /// Users whose game account link stopped renewing
    pub needs_reauth: Arc<RwLock<HashSet<Uuid>>>,

    /// What the sidecar last reported for each user's linked account
    pub game_presence: Arc<RwLock<HashMap<Uuid, GameActivity>>>
}

impl AppState {

    /// Builds the state a router is handed.
    ///
    /// # Arguments
    ///
    /// * `pool` - Pool of SQL connections.
    pub fn new(pool: SqlitePool) -> Self {

        // Create the broadcast channel the sidecar's connection reads
        let (to_sidecar, _) = broadcast::channel(config::get().limits.message_buffer);

        AppState {
            pool,
            registry: Registry::default(),
            to_sidecar,
            needs_reauth: Arc::new(RwLock::new(HashSet::new())),
            game_presence: Arc::new(RwLock::new(HashMap::new()))
        }
    }
}
