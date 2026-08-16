use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::bytesize::{ByteSize, MEBIBYTE};


/// Static Global (configured once on load from file)
static CONFIG: OnceLock<Config> = OnceLock::new();
const CONFIG_VERSION: u8 = 1;

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub version: u8,
    pub bind: Bind,
    pub storage: Storage,
    pub session: Session,
    pub limits: Limits,
    pub paging: Paging,
    pub socket: Socket,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: CONFIG_VERSION,
            bind: Bind::default(),
            storage: Storage::default(),
            session: Session::default(),
            limits: Limits::default(),
            paging: Paging::default(),
            socket: Socket::default()
        }
    }
}

/// Given a path to a config toml file, attempt to read and set global static config
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

/// Writes the defaults to disk when file does not exist
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

        if self.storage.db_path.is_empty() {
            return Err("storage.db_path must be set".to_string());
        }

        if self.storage.files_path.is_empty() {
            return Err("storage.files_path must be set".to_string());
        }

        if self.session.lifetime_days < 1 {
            return Err("session.lifetime_days must be at least 1".to_string());
        }

        // Equal values leave no renewal window, so sessions never extend
        if self.session.renew_after_days_elapsed >= self.session.lifetime_days {
            return Err("session.renew_after_days_elapsed must be less than session.lifetime_days".to_string());
        }

        range("username", self.limits.username_min, self.limits.username_max)?;
        range("display_name", self.limits.display_name_min, self.limits.display_name_max)?;
        range("password", self.limits.password_min, self.limits.password_max)?;
        range("room_name", 1, self.limits.room_name_max)?;
        range("message_body", 1, self.limits.message_body_max)?;
    
        if ByteSize::to_int(self.limits.file_bytes_max) <= 0 {
            return Err("limits.file_bytes_max must be at least 1 byte".to_string());
        }

        // 32 is the ordinal CHECK in the schema, a larger value fails on insert
        let attachments = self.limits.attachments_per_message_max;
        if attachments < 1 || attachments > 32 {
            return Err("limits.attachments_per_message_max must be between 1 and 32".to_string());
        }

        if self.paging.message_page < 1 {
            return Err("paging.message_page must be at least 1".to_string());
        }

        if self.paging.room_page < 1 {
            return Err("paging.room_page must be at least 1".to_string());
        }

        if self.paging.users_page < 1 {
            return Err("paging.user_page must be at least 1".to_string());
        }

        if self.socket.message_buffer < 1 {
            return Err("socket.message_buffer must be at least 1".to_string());
        }

        Ok(())
    }
}

/// Verify that the field's given value is within the expected range
fn range(name: &str, min: usize, max: usize) -> Result<(), String> {
    if min < 1 {
        return Err(format!("limits.{name}_min must be at least 1"));
    }

    if max < min {
        return Err(format!("limits.{name}_max must be at least limits.{name}_min"));
    }

    Ok(())
}

/// Public accessor for static global config
pub fn get() -> &'static Config {
    CONFIG.get().expect("config not initialized")
}

// Components //

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Bind {
    pub ip: String,
    pub port: u16
}

impl Default for Bind {
    fn default() -> Self {
        Bind {
            ip: "127.0.0.1".to_string(),
            port: 3000
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Storage {
    // SQLite database file
    pub db_path: String,

    // Directory holding uploaded files, sharded by hash
    pub files_path: String
}

impl Default for Storage {
    fn default() -> Self {
        Storage {
            db_path: "chat.db".to_string(),
            files_path: "files".to_string()
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Session {
    // How long a session survives without activity
    pub lifetime_days: i64,

    // Days that must elapse before an active session is extended again.
    // Activity always extends expiry to lifetime_days from now, and this
    // limits how often that write happens
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
    pub room_name_max: usize,
    pub message_body_max: usize,
    pub password_min: usize,
    pub password_max: usize,
    pub file_bytes_max: ByteSize,
    pub attachments_per_message_max: usize
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            username_min: 2,
            username_max: 32,
            display_name_min: 1,
            display_name_max: 64,
            room_name_max: 128,
            message_body_max: 8000,
            password_min: 8,
            password_max: 128,
            file_bytes_max: ByteSize::from_int(25 * MEBIBYTE),
            attachments_per_message_max: 10
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Paging {
    pub message_page: i64,
    pub room_page: i64,
    pub users_page: i64,
}

impl Default for Paging {
    fn default() -> Self {
        Paging {
            message_page: 200,
            room_page: 200,
            users_page: 25,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Socket {
    pub message_buffer: usize
}

impl Default for Socket {
    fn default() -> Self {
        Socket {
            message_buffer: 32
        }
    }
}
