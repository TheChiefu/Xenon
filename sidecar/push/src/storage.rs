//! Browser subscriptions, held in memory and mirrored to a JSON file.
//!
//! One process reads and writes this file, one call at a time, so a
//! whole-file rewrite on every mutation is safe — there is no concurrent
//! writer to race against, only a crash to guard, which the atomic rename
//! below handles.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::events::Subscription;

/// One stored subscription, keyed by endpoint.
#[derive(Deserialize, Serialize)]
struct Entry {
    user_id: String,
    p256dh: Vec<u8>,
    auth: Vec<u8>
}

/// Every browser subscribed.
pub struct Store {
    path: PathBuf,
    by_endpoint: HashMap<String, Entry>,

    /// Rebuilt from `by_endpoint` on load, never itself persisted
    by_user: HashMap<String, HashSet<String>>
}

impl Store {

    /// Loads the store from `path`, starting empty if it does not exist yet.
    ///
    /// # Arguments
    ///
    /// * `path` - File subscriptions are persisted to.
    pub fn load(path: &str) -> Result<Store, Box<dyn Error>> {
        let by_endpoint: HashMap<String, Entry> = match fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => return Err(e.into())
        };

        let mut by_user: HashMap<String, HashSet<String>> = HashMap::new();
        for (endpoint, entry) in &by_endpoint {
            by_user.entry(entry.user_id.clone()).or_default().insert(endpoint.clone());
        }

        Ok(Store { path: PathBuf::from(path), by_endpoint, by_user })
    }

    /// Stores a browser's subscription, replacing any earlier owner of the
    /// same endpoint.
    ///
    /// # Arguments
    ///
    /// * `user_id` - Account the browser is signed in as.
    /// * `subscription` - What the browser produced when it subscribed.
    pub fn upsert(&mut self, user_id: &str, subscription: &Subscription) {
        if let Some(previous) = self.by_endpoint.get(&subscription.endpoint) {
            if previous.user_id != user_id {
                if let Some(endpoints) = self.by_user.get_mut(&previous.user_id) {
                    endpoints.remove(&subscription.endpoint);
                }
            }
        }

        self.by_endpoint.insert(subscription.endpoint.clone(), Entry {
            user_id: user_id.to_string(),
            p256dh: subscription.p256dh.clone(),
            auth: subscription.auth.clone()
        });

        self.by_user.entry(user_id.to_string()).or_default().insert(subscription.endpoint.clone());

        self.save();
    }

    /// Removes one browser's subscription, if it belongs to `user_id`.
    ///
    /// # Arguments
    ///
    /// * `user_id` - Account the row must belong to.
    /// * `endpoint` - Push resource the row is keyed by.
    pub fn remove(&mut self, user_id: &str, endpoint: &str) {
        if self.by_endpoint.get(endpoint).is_some_and(|entry| entry.user_id == user_id) {
            self.remove_by_endpoint(endpoint);
        }
    }

    /// Removes one browser's subscription regardless of owner. Used to prune
    /// endpoints a push service reports as gone.
    ///
    /// # Arguments
    ///
    /// * `endpoint` - Push resource the row is keyed by.
    pub fn remove_by_endpoint(&mut self, endpoint: &str) {
        let Some(entry) = self.by_endpoint.remove(endpoint) else {
            return;
        };

        if let Some(endpoints) = self.by_user.get_mut(&entry.user_id) {
            endpoints.remove(endpoint);
        }

        self.save();
    }

    /// Reads every subscription belonging to any of `user_ids`.
    ///
    /// # Arguments
    ///
    /// * `user_ids` - Accounts to read.
    pub fn subscriptions_for(&self, user_ids: &[String]) -> Vec<Subscription> {
        let mut subscriptions = Vec::new();

        for user_id in user_ids {
            let Some(endpoints) = self.by_user.get(user_id) else {
                continue;
            };

            for endpoint in endpoints {
                if let Some(entry) = self.by_endpoint.get(endpoint) {
                    subscriptions.push(Subscription {
                        endpoint: endpoint.clone(),
                        p256dh: entry.p256dh.clone(),
                        auth: entry.auth.clone()
                    });
                }
            }
        }

        subscriptions
    }

    /// Writes `by_endpoint` to `path`, replacing it atomically so a crash
    /// mid-write cannot corrupt it. Written at mode `0600`, same as
    /// `vapid.key`, so only its owner can read it.
    fn save(&self) {
        let text = match serde_json::to_string(&self.by_endpoint) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("could not serialize subscriptions: {e}");
                return;
            }
        };

        let tmp = self.path.with_extension("tmp");

        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        let result = options.open(&tmp).and_then(|mut file| file.write_all(text.as_bytes()));

        if let Err(e) = result {
            eprintln!("could not write {}: {e}", tmp.display());
            return;
        }

        if let Err(e) = fs::rename(&tmp, &self.path) {
            eprintln!("could not replace {}: {e}", self.path.display());
        }
    }
}
