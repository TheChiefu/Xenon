//! What the sidecar reads at startup.

use std::error::Error;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// File the config is read from, relative to the working directory
pub const CONFIG_FILE: &str = "config.toml";

#[derive(Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    /// Xenon's socket address, `ws://` or `wss://`
    pub xenon: String,

    /// The same string as Xenon's `[push] secret`
    pub secret: String,

    /// Contact URI a push service can reach you at, `mailto:` or `https:`
    pub subject: String,

    /// Seconds a push service keeps a message for a browser that is not connected
    pub ttl: u32
}

impl Default for Config {
    fn default() -> Self {
        Config {
            xenon: "ws://127.0.0.1:3000/push/ws".to_string(),
            secret: String::new(),
            subject: String::new(),
            ttl: 86400
        }
    }
}

/// Reads the config file, writing defaults when it is absent.
///
/// Returns an error when the file was just written, or when `secret` is unset.
///
/// # Arguments
///
/// * `path` - File the config is read from.
pub fn read(path: &str) -> Result<Config, Box<dyn Error>> {
    if !Path::new(path).exists() {
        fs::write(path, toml::to_string_pretty(&Config::default())?)?;
        return Err(format!("wrote {path}, fill it in").into());
    }

    let config: Config = toml::from_str(&fs::read_to_string(path)?)?;

    if config.secret.is_empty() {
        return Err("secret is unset, and must match Xenon's [push] secret".into());
    }

    Ok(config)
}
