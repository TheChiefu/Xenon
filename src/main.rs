mod api;
mod bytesize;
mod config;
mod db;
mod error;
mod models;
mod routes;
mod utils;
mod validate;

use std::path::Path;
use std::time::Duration;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;
use uuid::Uuid;
use tracing;

use crate::error::{AppError, Result};
use crate::models::GlobalRole;
use crate::routes::AppState;

#[tokio::main]
async fn main() -> Result<()> {

    // Read configuration file (create defaults if not existing)
    let config_path = std::env::var("XENON_CONFIG").unwrap_or_else(|_| "config.toml".into());
    config::init(&config_path);
    println!("{}", config::get().limits.file_bytes_max.to_int());

    // Setup tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "xenon=info,sqlx=warn".into())
        )
        .init();
    tracing::info!("loaded configuration file: {config_path}");
    tracing::info!("starting server");

    // Check if file directory is setup
    ensure_files();

    // Set DB options and properties
    let db_path = &config::get().storage.db_path;
    let options = SqliteConnectOptions::new()
        .filename(db_path)
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
    tracing::info!("connected to database: {db_path}");

    // Ensure owner bootstrap account is created for owner access
    ensure_owner(&pool).await?;

    let app_state = AppState::new(pool);
    let app: axum::Router = routes::router(app_state);
    let bind_addr = format!(
        "{}:{}",
        config::get().bind.ip,
        config::get().bind.port
    );

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
            println!("+-------- Owner account created --------+");
            println!("           Username:\t{username}         ");
            println!("           Password:\t{password}         ");
            println!("  Save credentials (this is NOT logged)  ");
            println!("+---------------------------------------+");
            Ok(())
        }
        Err(AppError::OwnerExists) => Ok(()),
        Err(e) => Err(e),
    }
}

fn ensure_files() {

    // Create or check if files folder path exists
    let files_path = &config::get().storage.files_path;
    if let Err(e) = std::fs::create_dir_all(files_path) {
        eprintln!("{files_path}: {e}");
        std::process::exit(1);
    }

    // Create/check "tmp" folder where streaming uploads sit
    let subdir = Path::new(files_path).join("tmp");
    if let Err(e) = std::fs::create_dir_all(&subdir) {
        eprintln!("{}: {e}", subdir.display());
        std::process::exit(1);
    }

    // Test if tmp directory is writable (create probe file)
    let test_file = subdir.join("test");
    if let Err(e) = std::fs::write(&test_file, "") {
        eprintln!("{} is not writable: {e}", subdir.display());
        std::process::exit(1);
    }

    // Remove probe file now that the write succeeded
    if let Err(e) = std::fs::remove_file(&test_file) {
        eprintln!("{}: {e}", test_file.display());
        std::process::exit(1);
    }

    tracing::info!("file storage ready at: {files_path}");

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
    tracing::info!("Received \"SIGTERM\", terminating program...");
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
