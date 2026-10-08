//! What this radio keeps secret: `secrets.toml` on the storage, beside
//! config.toml but out of WebDAV's reach entirely (webdav.rs). Set and
//! read through /api/management (http.rs), which never shows or takes the
//! private key. For the management server (management.rs):
//!
//! ```toml
//! private_key = "..."        # this radio's: made here, never leaves it
//! server_public_key = "..."  # the server's: `server keygen` prints it
//!
//! [server]
//! host = "..."
//! port = 3101
//! ```
//!
//! Keys are X25519, 64 hex digits. The radio makes its own private key the
//! first time WiFi is up without one (`make_key`: the hardware RNG is only
//! truly random while the WiFi radio runs). Its public key is on
//! /api/management, for the server's radios.toml. provision.py erases the
//! storage, so a re-provisioned radio has a new key and is enrolled again.
//!
//! WARNING: anyone holding the radio can still read the flash over USB.
//! That's what revoking its key on the server is for.
//!
//! Read fresh at each use (they're rare), so a change through
//! /api/management counts at once.

use std::fs;
use std::io::ErrorKind;
use std::sync::Mutex;

use open_oswst_core::management::{key_from_hex, key_to_hex, Key};

use crate::devices::storage;
use crate::noise;

const FILE: &str = "secrets.toml";

/// The management server: where it is and its public key
pub struct Server {
    pub host: String,
    pub port: u16,
    pub public_key: Key,
}

pub fn path() -> String {
    format!("{}/{}", storage::ROOT, FILE)
}

/// The file as a table: empty if it's missing or broken (logged)
fn read() -> toml::Table {
    match fs::read_to_string(path()) {
        Ok(text) => text.parse().unwrap_or_else(|e| {
            log::error!("Secrets: {} is broken: {}", FILE, e);
            toml::Table::new()
        }),
        Err(e) if e.kind() == ErrorKind::NotFound => toml::Table::new(),
        Err(e) => {
            log::error!("Secrets: can't read {}: {}", FILE, e);
            toml::Table::new()
        }
    }
}

fn key(table: &toml::Table, name: &str) -> Option<Key> {
    table.get(name)?.as_str().and_then(key_from_hex)
}

/// This radio's private key, if it has one
pub fn private_key() -> Option<Key> {
    key(&read(), "private_key")
}

/// The management server, or why we can't reach one (a few words, for the
/// screen)
pub fn server() -> Result<Server, &'static str> {
    let table = read();
    let server = table.get("server").ok_or("No server set")?;
    let host = server.get("host").and_then(|v| v.as_str());
    let port = server.get("port").and_then(|v| v.as_integer());
    let (Some(host), Some(port)) = (host, port.and_then(|port| u16::try_from(port).ok())) else {
        return Err("Server: host? port?");
    };
    let public_key = key(&table, "server_public_key").ok_or("No server key")?;
    Ok(Server {
        host: host.into(),
        port,
        public_key,
    })
}

/// Make this radio's private key if it has none. Call with WiFi up (net.rs)
pub fn make_key() {
    let mut table = read();
    if key(&table, "private_key").is_some() {
        return;
    }
    table.insert(
        "private_key".into(),
        key_to_hex(&noise::generate_private_key()).into(),
    );
    let text = toml::to_string(&table).unwrap_or_default();
    match replace(&text) {
        Ok(()) => log::info!("Secrets: made this radio's key"),
        Err(e) => log::error!("Secrets: saving {} failed: {}", FILE, e),
    }
}

/// The public half of this radio's key, for /api/management: hex, or None
/// without one. Worked out once per key (an X25519 multiply, ~110 ms)
pub fn public_key_hex() -> Option<String> {
    static LAST: Mutex<Option<(Key, String)>> = Mutex::new(None);
    let private = private_key()?;
    let mut last = LAST.lock().unwrap();
    match &*last {
        Some((key, public)) if *key == private => Some(public.clone()),
        _ => {
            let public = key_to_hex(&noise::public_key(&private));
            *last = Some((private, public.clone()));
            Some(public)
        }
    }
}

/// Save where the management server is and its public key (PUT
/// /api/management). This radio's own key is left as it is
pub fn set_server(host: &str, port: u16, public_key: &Key) -> Result<(), String> {
    let mut table = read();
    let mut server = toml::Table::new();
    server.insert("host".into(), host.into());
    server.insert("port".into(), i64::from(port).into());
    table.insert("server".into(), server.into());
    table.insert("server_public_key".into(), key_to_hex(public_key).into());
    let text = toml::to_string(&table).map_err(|e| e.to_string())?;
    replace(&text).map_err(|e| e.to_string())
}

/// secrets.toml, or a file on its way to being it ("secrets.toml.new"):
/// what WebDAV must never reach
pub fn is_secret(path: &std::path::Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().contains(FILE))
}

/// Written to a new file first, then renamed over the old one, so a reset
/// mid-write never loses the key
fn replace(text: &str) -> std::io::Result<()> {
    let path = path();
    let new = format!("{}.new", path);
    fs::write(&new, text)?;
    fs::rename(&new, &path)
}
