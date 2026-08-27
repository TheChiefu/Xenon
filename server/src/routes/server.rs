//! HTTP handlers reporting what this server is.

use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;

use crate::config;
use crate::error::Result;

// Data Structs //

/// Name, version, kind, and description of this server.
#[derive(Serialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    pub kind: String,
    pub description: String,
}

// Routing Methods //

/// Gets the server's name, version, kind, and description.
pub async fn info() -> Result<(StatusCode, Json<ServerInfo>)> {

    let info = ServerInfo {
        name: config::get().info.name.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        kind: config::get().info.kind.clone(),
        description: config::get().info.description.clone(),
    };

    Ok((StatusCode::OK, Json(info)))
}

/// Gets the server version.
pub async fn version() -> Result<Json<String>> {

    let version = env!("CARGO_PKG_VERSION").to_string();

    Ok(Json(version))
}

/// Gets what kind of server is deployed, such as "Development" or "Release".
pub async fn kind() -> Result<Json<String>> {

    let kind = config::get().info.kind.clone();

    Ok(Json(kind))
}
