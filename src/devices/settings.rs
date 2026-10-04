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
//! The settings and flags (`get` / `set`) are declared in core's config.rs.
//! The WiFi networks are for the firmware (`wifi_networks`), tried in order.

use std::fs;
use std::io::ErrorKind;

use open_oswst_core::config;

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
    let path = path();
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

/// Where the config file is
pub fn path() -> String {
    format!("{}/{}", storage::ROOT, FILE)
}

/// Replace the whole config file: written to a new file first, then renamed
/// over the old one, so a reset mid-write never leaves half a config.
pub fn replace(text: &str) -> std::io::Result<()> {
    let path = path();
    let new = format!("{}.new", path);
    fs::write(&new, text)?;
    fs::rename(&new, &path)
}

/// Can this firmware use `text` as its config? It must parse as TOML, and
/// every setting in it must have a value the setting can take. A file that
/// fails is refused (webdav.rs), and the old one stays
pub fn check(text: &str) -> Result<(), String> {
    let table = text.parse::<toml::Table>().map_err(|e| e.to_string())?;
    config::check(&Settings { table })
}

/// The [flags] table's name in the file, and in a key (`flags.x`)
const FLAGS: &str = "flags";
/// The WiFi networks: the firmware's, not a setting
const WIFI: &str = "wifi";

impl Settings {
    /// The WiFi networks to try, in the order listed. Malformed entries are
    /// skipped and logged.
    pub fn wifi_networks(&self) -> Vec<WifiNetwork> {
        let Some(entries) = self.table.get(WIFI).and_then(|wifi| wifi.as_array()) else {
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

    fn save(&self) {
        let text = toml::to_string(&self.table).unwrap_or_default();
        if let Err(e) = replace(&text) {
            log::warn!("Config: saving {} failed: {}", path(), e);
        }
    }
}

impl open_oswst_core::devices::settings::Settings for Settings {
    fn get(&self, key: &str) -> Option<String> {
        let value = match key.split_once('.') {
            Some((FLAGS, flag)) => self.table.get(FLAGS)?.get(flag)?,
            _ => self.table.get(key)?,
        };
        match value {
            toml::Value::String(text) => Some(text.clone()),
            other => Some(other.to_string()),
        }
    }

    /// Saved as TOML's own type: `true`, `42`, else a string
    fn set(&mut self, key: &str, value: &str) {
        let value = if let Ok(on) = value.parse::<bool>() {
            toml::Value::Boolean(on)
        } else if let Ok(n) = value.parse::<i64>() {
            toml::Value::Integer(n)
        } else {
            toml::Value::String(value.into())
        };
        match key.split_once('.') {
            Some((FLAGS, flag)) => {
                let flags = self
                    .table
                    .entry(FLAGS)
                    .or_insert_with(|| toml::Value::Table(toml::Table::new()));
                if let Some(flags) = flags.as_table_mut() {
                    flags.insert(flag.into(), value);
                }
            }
            _ => {
                self.table.insert(key.into(), value);
            }
        }
        self.save();
    }

    fn keys(&self) -> Vec<String> {
        let mut keys = Vec::new();
        for (key, value) in &self.table {
            match (key.as_str(), value) {
                (WIFI, _) => {}
                (FLAGS, toml::Value::Table(flags)) => {
                    keys.extend(flags.keys().map(|flag| format!("{}.{}", FLAGS, flag)))
                }
                _ => keys.push(key.clone()),
            }
        }
        keys
    }
}
