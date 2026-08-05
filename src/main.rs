mod api;
mod db;
mod error;
mod models;
mod routes;
mod utils;
mod validate;

use std::time::Duration;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;
use uuid::Uuid;
use tracing;

use crate::error::{AppError, Result};
use crate::models::GlobalRole;
use crate::routes::AppState;

const BIND_IP: &str = "127.0.0.1";
const BIND_PORT: u16 = 3000;

#[tokio::main]
async fn main() -> Result<()> {

    // Setup tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "xenon=info,sqlx=warn".into())
        )
        .init();

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
    ensure_owner(&pool).await?;

    let app_state = AppState::new(pool);
    let app: axum::Router = routes::router(app_state);
    let bind_addr = format!("{BIND_IP}:{BIND_PORT}");
    let listener = tokio::net::TcpListener::bind(bind_addr).await.expect("failed to bind port to listener");
    tracing::info!("listening on {}", listener.local_addr().expect("failed to find bound address"));

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("failed serve axum application");

    Ok(())

}

/// Generates a default owner for the server for inital setup
async fn ensure_owner(pool: &SqlitePool) -> Result<()> {
    let password = utils::generate_invite_code();
    let hash = utils::hash_password(&password)?;
    let username = "owner";
    let display_name = "Owner";
    let id = Uuid::now_v7();

    // Attempt to create owner account
    let mut conn = pool.acquire().await?;
    let result = db::insert_user(
        &mut *conn,
        id,
        username,
        display_name,
        &hash,
        GlobalRole::Owner
    ).await;

    match result {
        Ok(()) => {
            // Printed, never logged. This is the only time the password is shown
            println!("Owner account created: ");
            println!("Username: {username}");
            println!("Password: [{password}]");
            println!("Note: This is not logged save somewhere safe");
            Ok(())
        }
        Err(AppError::OwnerExists) => Ok(()),
        Err(e) => Err(e),
    }
}

// Shutdown //

/// Handle "SIGTERM" for unix platforms
#[cfg(unix)]
async fn terminate() {
    use tokio::signal::unix::{signal, SignalKind};
    signal(SignalKind::terminate())
        .expect("failed to install SIGTERM handler")
        .recv()
        .await;
    tracing::info!("Received \"SIGTERM\", termininating program...");
}

#[cfg(not(unix))]
async fn terminate() {
    std::future::pending::<()>().await;
}

/// Resolves on the first shutdown signal
/// Axum polls this alongside serving
async fn shutdown_signal() {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = terminate() => {},
    }
    tracing::info!("Shutting down server");
}
