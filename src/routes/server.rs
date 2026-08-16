
use axum::{Json, http::StatusCode};
use serde::Serialize;
use crate::{config, error::Result};

// Data Structs //
#[derive(Serialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    pub kind: String,
    pub description: String,
}

// Routing Methods //

/// Get server version
pub async fn version() -> Result<Json<String>> {

    let version = env!("CARGO_PKG_VERSION").to_string();
    Ok(Json(version))
}

/// Get what kind of server is deployed (ie "Development", "Release")
pub async fn kind() -> Result<Json<String>> {
    let kind = config::get().info.kind.clone();
    Ok(Json(kind))
}

pub async fn info(
) -> Result<(StatusCode, Json<ServerInfo>)> {

    let version = env!("CARGO_PKG_VERSION").to_string();
    let description = config::get().info.description.clone();
    let kind = config::get().info.kind.clone();
    let name = config::get().info.name.clone();
    let info = ServerInfo {
        name,
        version,
        kind,
        description,
    };

    Ok((StatusCode::OK, Json(info)))
    
}