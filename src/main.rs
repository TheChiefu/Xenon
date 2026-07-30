mod api;
mod bootstrap;
mod db;
mod error;
mod models;
mod utils;
mod validate;

use std::time::Duration;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

use crate::error::Result;


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

    bootstrap::ensure_owner(&pool).await?;

    Ok(())

}
