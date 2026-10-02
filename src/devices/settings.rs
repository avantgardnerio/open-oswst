//! Saved settings: `config.toml` on the storage partition, so they survive a
//! reboot and a reflash, and read and edit by hand. scripts/provision.py
//! writes it before a board ships. For example:
//!
//! ```toml
//! mode = "repeater"
//!
//! [[wifi]]
//! ssid = "Starlink"
//! password = "..."
//! ```
//!
//! The app's settings are top-level text values (`get` / `set`). The WiFi
//! networks are for the firmware (`wifi_networks`), tried in order.

use std::fs;
use std::io::ErrorKind;

use crate::devices::storage;

const FILE: &str = "config.toml";

pub struct Settings {
    table: toml::Table,
}

/// One network the board may join
pub struct WifiNetwork {
    pub ssid: String,
    pub password: String,
}

/// Read the config file. Missing means defaults; broken is logged and also
/// means defaults (the file is left as it is until a setting is saved).
pub fn init() -> Settings {
    let path = format!("{}/{}", storage::ROOT, FILE);
    let table = match fs::read_to_string(&path) {
        Ok(text) => text.parse::<toml::Table>().unwrap_or_else(|e| {
            log::error!("Config: {} is broken, using defaults: {}", path, e);
            toml::Table::new()
        }),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            log::warn!("Config: no {}, using defaults", path);
            toml::Table::new()
        }
        Err(e) => {
            log::error!("Config: can't read {}, using defaults: {}", path, e);
            toml::Table::new()
        }
    };
    Settings { table }
}

impl Settings {
    /// The WiFi networks to try, in the order listed. Malformed entries are
    /// skipped and logged.
    pub fn wifi_networks(&self) -> Vec<WifiNetwork> {
        let Some(entries) = self.table.get("wifi").and_then(|wifi| wifi.as_array()) else {
            return Vec::new();
        };
        entries
            .iter()
            .filter_map(|entry| {
                let text = |key| entry.get(key).and_then(|v| v.as_str()).map(String::from);
                match (text("ssid"), text("password")) {
                    (Some(ssid), Some(password)) => Some(WifiNetwork { ssid, password }),
                    _ => {
                        log::warn!("Config: a [[wifi]] entry needs ssid and password");
                        None
                    }
                }
            })
            .collect()
    }

    /// Write the whole file again: to a new file first, then renamed over the
    /// old one, so a reset mid-write never leaves half a config.
    fn save(&self) {
        let path = format!("{}/{}", storage::ROOT, FILE);
        let new = format!("{}.new", path);
        let text = toml::to_string(&self.table).unwrap_or_default();
        if let Err(e) = fs::write(&new, text).and_then(|()| fs::rename(&new, &path)) {
            log::warn!("Config: saving {} failed: {}", path, e);
        }
    }
}

impl open_oswst_core::devices::settings::Settings for Settings {
    fn get(&self, key: &str) -> Option<String> {
        match self.table.get(key)? {
            toml::Value::String(text) => Some(text.clone()),
            other => Some(other.to_string()),
        }
    }

    fn set(&mut self, key: &str, value: &str) {
        self.table.insert(key.into(), value.into());
        self.save();
    }
}
