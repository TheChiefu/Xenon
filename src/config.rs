//! The config file, read once at startup and readable from anywhere after.

use std::sync::OnceLock;
use std::net::{IpAddr, SocketAddr};

use serde::{Deserialize, Serialize};

use crate::bytesize::{ByteSize, MEBIBYTE};

/// Static Global (configured once on load from file)
static CONFIG: OnceLock<Config> = OnceLock::new();

/// Config file layout this server reads.
const CONFIG_VERSION: u8 = 1;

/// Every setting the server reads at startup.
#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub version: u8,
    pub info: Info,
    pub bind: Bind,
    pub storage: Storage,
    pub logging: Logging,
    pub session: Session,
    pub limits: Limits,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: CONFIG_VERSION,
            info: Info::default(),
            bind: Bind::default(),
            storage: Storage::default(),
            logging: Logging::default(),
            session: Session::default(),
            limits: Limits::default(),
        }
    }
}

/// Reads the config file into the global static, writing defaults if it is absent.
///
/// Exits the process if the file cannot be parsed or fails validation.
///
/// # Arguments
///
/// * `path` - Path to the config toml file.
pub fn init(path: &str) {

    let config: Config = match std::fs::read_to_string(path) {
        Ok(text) => match toml::from_str(&text) {
            Ok(config) => config,
            Err(e) => {
                eprintln!("{path}: {e}");
                std::process::exit(1);
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => write_default(path),
        Err(e) => panic!("could not read {path}: {e}"),
    };

    if let Err(reason) = config.validate() {
        eprintln!("{path}: {reason}");
        std::process::exit(1);
    }

    // Set global in memory config
    CONFIG.set(config).expect("config already initialized");
}

/// Writes the defaults to disk and returns them.
///
/// # Arguments
///
/// * `path` - Path the config toml file is written to.
fn write_default(path: &str) -> Config {
    let config = Config::default();

    match toml::to_string_pretty(&config) {
        Ok(text) => {
            if let Err(e) = std::fs::write(path, text) {
                eprintln!("could not write {path}: {e}");
            }
        }
        Err(e) => eprintln!("could not serialize default config: {e}"),
    }

    config
}

impl Config {

    /// Ensure loaded config file has real values, reject any invalid ones
    fn validate(&self) -> Result<(), String> {

        if self.version != CONFIG_VERSION {
            return Err(format!(
                "version {} is not supported, server is using version {}",
                self.version,
                CONFIG_VERSION
            ));
        }

        if self.bind.port == 0 {
            return Err("bind.port must be greater than 0".to_string());
        }

        if self.bind.ip.parse::<IpAddr>().is_err() {
            return Err(format!("bind.ip must be an IPv4 or IPv6 address, got \"{}\"", self.bind.ip));
        }

        match (&self.bind.certificate, &self.bind.key) {
            (Some(_), None) => return Err("bind.key must be set alongside bind.certificate".to_string()),
            (None, Some(_)) => return Err("bind.certificate must be set alongside bind.key".to_string()),
            _ => {}
        }

        if self.storage.database.is_empty() {
            return Err("storage.database must be set".to_string());
        }

        if self.storage.files.is_empty() {
            return Err("storage.files must be set".to_string());
        }

        if self.session.lifetime_days < 1 {
            return Err("session.lifetime_days must be at least 1".to_string());
        }

        // Equal values leave no renewal window, so sessions never extend
        if self.session.renew_after_days_elapsed >= self.session.lifetime_days {
            return Err("session.renew_after_days_elapsed must be less than session.lifetime_days".to_string());
        }

        if self.limits.username_min < 1 || self.limits.username_max < self.limits.username_min {
            return Err("limits.username_min must be at least 1 and no greater than limits.username_max".to_string());
        }

        if self.limits.display_name_min < 1 || self.limits.display_name_max < self.limits.display_name_min {
            return Err("limits.display_name_min must be at least 1 and no greater than limits.display_name_max".to_string());
        }

        if self.limits.password_min < 1 || self.limits.password_max < self.limits.password_min {
            return Err("limits.password_min must be at least 1 and no greater than limits.password_max".to_string());
        }

        if self.limits.room_name_max < 1 {
            return Err("limits.room_name_max must be at least 1".to_string());
        }

        if self.limits.message_body_max < 1 {
            return Err("limits.message_body_max must be at least 1".to_string());
        }


        if ByteSize::to_int(self.limits.file_bytes_max) <= 0 {
            return Err("limits.file_bytes_max must be at least 1 byte".to_string());
        }

        // 32 is the ordinal CHECK in the schema, a larger value fails on insert
        let attachments = self.limits.attachments_per_message_max;
        if attachments < 1 || attachments > 32 {
            return Err("limits.attachments_per_message_max must be between 1 and 32".to_string());
        }

        if self.limits.message_page < 1 {
            return Err("limits.message_page must be at least 1".to_string());
        }

        if self.limits.room_page < 1 {
            return Err("limits.room_page must be at least 1".to_string());
        }

        if self.limits.users_page < 1 {
            return Err("limits.users_page must be at least 1".to_string());
        }

        if self.limits.message_buffer < 1 {
            return Err("limits.message_buffer must be at least 1".to_string());
        }

        if !matches!(self.logging.level.as_str(), "trace" | "debug" | "info" | "warn" | "error") {
            return Err(format!(
                "logging.level must be trace, debug, info, warn, or error, got \"{}\"",
                self.logging.level
            ));
        }

        if self.logging.file.is_empty() {
            tracing::warn!("logging.file is not set, tracing calls go to stdout only")
        }

        if self.info.name.is_empty() {
            tracing::warn!("no server name provided")
        }

        if self.info.kind.is_empty() {
            tracing::warn!("no server kind/type provided")
        }

        if self.info.description.is_empty() {
            tracing::warn!("no server description provided")
        }

        Ok(())
    }

    /// The address the listener binds.
    /// Panics if `bind.ip` does not parse.
    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(
            self.bind.ip.parse().expect("bind.ip validated at startup"),
            self.bind.port
        )
    }

    /// Whether the listener binds a loopback address.
    pub fn binds_loopback(&self) -> bool {
        self.socket_addr().ip().is_loopback()
    }

    /// Whether a certificate and key are configured for the listener.
    pub fn tls_configured(&self) -> bool {
        self.bind.certificate.is_some() && self.bind.key.is_some()
    }
}

/// Public accessor for static global config
///
/// # Panics
///
/// Panics if called before `init`.
pub fn get() -> &'static Config {
    CONFIG.get().expect("config not initialized")
}

// Components //

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Bind {
    pub ip: String,
    pub port: u16,
    /// PEM certificate chain served to clients
    pub certificate: Option<String>,

    /// PEM private key for `certificate`
    pub key: Option<String>,
}

impl Default for Bind {
    fn default() -> Self {
        Bind {
            ip: "127.0.0.1".to_string(),
            port: 3000,
            certificate: None,
            key: None
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Storage {
    /// SQLite database file
    pub database: String,

    /// Directory holding uploaded files, sharded by hash
    pub files: String
}

impl Default for Storage {
    fn default() -> Self {
        Storage {
            database: "chat.db".to_string(),
            files: "files".to_string()
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Logging {
    /// File every tracing call is appended to, empty for stdout only
    pub file: String,

    /// Lowest level this server's own tracing calls are written at
    pub level: String
}

impl Default for Logging {
    fn default() -> Self {
        Logging {
            file: "xenon.log".to_string(),
            level: "info".to_string()
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Session {
    /// How long a session survives without activity
    pub lifetime_days: i64,

    /// Days that must elapse before an active session is extended again.
    /// Activity always extends expiry to `lifetime_days` from now, and this
    /// limits how often that write happens
    pub renew_after_days_elapsed: i64
}

impl Default for Session {
    fn default() -> Self {
        Session {
            lifetime_days: 30,
            renew_after_days_elapsed: 1
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Limits {
    pub username_min: usize,
    pub username_max: usize,
    pub display_name_min: usize,
    pub display_name_max: usize,
    pub profile_description_max: usize,
    pub room_name_max: usize,
    pub message_body_max: usize,
    pub password_min: usize,
    pub password_max: usize,
    pub file_bytes_max: ByteSize,
    pub attachments_per_message_max: usize,
    pub message_page: i64,
    pub room_page: i64,
    pub users_page: i64,
    pub message_buffer: usize
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            username_min: 2,
            username_max: 32,
            display_name_min: 1,
            display_name_max: 64,
            profile_description_max: 2000,
            room_name_max: 128,
            message_body_max: 8000,
            password_min: 8,
            password_max: 128,
            file_bytes_max: ByteSize::from_int(25 * MEBIBYTE),
            attachments_per_message_max: 10,
            message_page: 200,
            room_page: 200,
            users_page: 25,
            message_buffer: 32
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Info {
    pub name: String,
    pub kind: String,
    pub description: String,
}

impl Default for Info {
    fn default() -> Self {
        Info {
            name: "Xenon Server".to_string(),
            kind: "Development".to_string(),
            description: "My custom Xenon server".to_string(),
        }
    }
}