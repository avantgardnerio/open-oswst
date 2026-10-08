//! The server's key pair (server.key) and the radios it lets in
//! (radios.toml), both in the server's directory. Keys are 64 hex digits.
//!
//! server.key, made by `server keygen DIR`:
//!   private_key = "..."
//!   public_key = "..."
//!
//! radios.toml, written by hand: a radio's public key is on its
//! /api/status (management_key)
//!   [[radio]]
//!   name = "handheld-1"
//!   mac = "AA:BB:CC:DD:EE:FF"
//!   public_key = "..."
//!   revoked = true      # optional: a lost or stolen radio

use open_oswst_core::management::{key_from_hex, key_to_hex, Key, PATTERN};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

pub const KEY_FILE: &str = "server.key";
pub const RADIOS_FILE: &str = "radios.toml";

#[derive(Serialize, Deserialize)]
struct KeyFile {
    private_key: String,
    public_key: String,
}

pub struct ServerKey {
    pub private: Key,
    pub public: Key,
}

impl ServerKey {
    /// Make a new key pair in `dir`. Refuses to replace one: every radio
    /// holds the public key, so a new one means changing them all
    pub fn generate(dir: &Path) -> Result<ServerKey, String> {
        let path = dir.join(KEY_FILE);
        let keys = snow::Builder::new(PATTERN.parse().unwrap())
            .generate_keypair()
            .map_err(|e| e.to_string())?;
        let key = ServerKey {
            private: keys.private.try_into().unwrap(),
            public: keys.public.try_into().unwrap(),
        };
        let text = toml::to_string(&KeyFile {
            private_key: key_to_hex(&key.private),
            public_key: key.public_hex(),
        })
        .map_err(|e| e.to_string())?;
        // Readable by its owner only, from the moment it exists
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .and_then(|mut file| file.write_all(text.as_bytes()))
            .map_err(|e| format!("{}: {}", path.display(), e))?;
        Ok(key)
    }

    pub fn load(dir: &Path) -> Result<ServerKey, String> {
        let path = dir.join(KEY_FILE);
        let text = fs::read_to_string(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
        let file: KeyFile =
            toml::from_str(&text).map_err(|e| format!("{}: {}", path.display(), e))?;
        match (
            key_from_hex(&file.private_key),
            key_from_hex(&file.public_key),
        ) {
            (Some(private), Some(public)) => Ok(ServerKey { private, public }),
            _ => Err(format!("{}: keys must be 64 hex digits", path.display())),
        }
    }

    pub fn public_hex(&self) -> String {
        key_to_hex(&self.public)
    }
}

/// One radio in radios.toml
#[derive(Deserialize, Debug, Clone)]
pub struct Radio {
    pub name: String,
    pub mac: String,
    pub public_key: String,
    #[serde(default)]
    pub revoked: bool,
}

impl std::fmt::Display for Radio {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{} ({})", self.name, self.mac)
    }
}

#[derive(Deserialize)]
struct RadiosFile {
    #[serde(default)]
    radio: Vec<Radio>,
}

/// The radio with this public key, if radios.toml lets it in. Read fresh
/// each time: a radio added or revoked counts from its next connection
pub fn find_radio(path: &Path, key: &Key) -> Result<Radio, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {}", path.display(), e))?;
    let file: RadiosFile =
        toml::from_str(&text).map_err(|e| format!("{}: {}", path.display(), e))?;
    let radio = file
        .radio
        .into_iter()
        .find(|radio| key_from_hex(&radio.public_key) == Some(*key))
        .ok_or("not in the list")?;
    if radio.revoked {
        return Err(format!("{} is revoked", radio.name));
    }
    Ok(radio)
}
