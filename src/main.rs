mod api;
mod bootstrap;
mod db;
mod error;
mod models;
mod routes;
mod utils;
mod validate;

use std::time::Duration;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

use crate::error::Result;

const BIND_IP: &str = "127.0.0.1";
const BIND_PORT: u16 = 3000;

#[tokio::main]
async fn main() -> Result<()> {

    // Set DB options and properties
    let options = SqliteConnectOptions::new()
        .filename("chat.db")
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal) // Write-Ahead Logging
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5))
        .pragma("secure_delete", "ON");

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;

    // Create initial DB if it does not already exists
    sqlx::migrate!().run(&pool).await?;

    // Ensure owner bootstrap account is created for owner access
    bootstrap::ensure_owner(&pool).await?;

    let app: axum::Router = routes::router(pool);
    let bind_addr = format!("{BIND_IP}:{BIND_PORT}");
    let listener = tokio::net::TcpListener::bind(bind_addr).await.expect("failed to bind port to listener");
    println!("listening on {}", listener.local_addr().expect("failed to find bound address"));
    axum::serve(listener, app).await.expect("failed serve axum application");

    Ok(())

}
