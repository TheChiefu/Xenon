//! What every handler is given: the database, and who is connected.

use axum::extract::FromRef;
use sqlx::SqlitePool;

use crate::sockets::registry::Registry;

/// Cloned per request.
///
/// `FromRef` generates a per-field accessor, letting handlers keep asking for
/// `State<SqlitePool>`.
#[derive(Clone, FromRef)]
pub struct AppState {
    pub pool: SqlitePool,
    pub registry: Registry
}

impl AppState {

    /// Builds the state a router is handed.
    ///
    /// # Arguments
    ///
    /// * `pool` - Pool of SQL connections.
    pub fn new(pool: SqlitePool) -> Self {
        AppState {
            pool,
            registry: Registry::default()
        }
    }
}
