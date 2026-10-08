//! Talking to the management server (issue #34; server/ in this repo):
//! connect, the Noise handshake (core's management.rs, on mbedTLS:
//! noise.rs), then requests and their responses. Blocking, so it runs on a
//! thread of its own: the net thread for the menu's jobs and the installs
//! (net.rs), the HTTP API's for its endpoints (http.rs). Where the server is
//! and the keys: secrets.toml (devices/secrets.rs).
//!
//! Nothing here runs unless someone asked: the menu, or the HTTP API.

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use esp_idf_svc::ota::EspOta;
use open_oswst_core::management::{NoiseLink, Offer, Request, Response, CHUNK, IMAGE_CHANGED};

use crate::devices::secrets;
use crate::firmware;
use crate::noise::MbedtlsResolver;

/// For the connection, and then for each answer (the handshake's too). The
/// menu waits longer (app.rs UPDATE_TIMEOUT), so it hears which one ran out
const TIMEOUT: Duration = Duration::from_secs(10);

type Link = NoiseLink<TcpStream>;

/// Errors are a few words, for the screen: "timed out", "Wrong key or
/// version". The server's address stays out of them, and so out of the log
fn io(e: std::io::Error) -> String {
    e.kind().to_string()
}

fn connect() -> Result<Link, String> {
    let server = secrets::server()?;
    let ours = secrets::private_key().ok_or("No radio key yet")?;
    let address = (server.host.as_str(), server.port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut addresses| addresses.next())
        .ok_or("Server not found")?;
    let stream = TcpStream::connect_timeout(&address, TIMEOUT).map_err(io)?;
    stream.set_read_timeout(Some(TIMEOUT)).map_err(io)?;
    stream.set_write_timeout(Some(TIMEOUT)).map_err(io)?;
    stream.set_nodelay(true).map_err(io)?;
    NoiseLink::connect(stream, Box::new(MbedtlsResolver), &ours, &server.public_key)
        .map_err(|e| e.to_string())
}

fn ask(link: &mut Link, request: &Request) -> Result<Response, String> {
    link.send(request).map_err(io)?;
    link.receive().map_err(io)
}

/// Send `request` on a connection of its own, and wait for its response
pub fn call(request: &Request) -> Result<Response, String> {
    ask(&mut connect()?, request)
}

/// Is there other firmware for us? UpToDate, Offer, or the server's Error
pub fn check() -> Result<Response, String> {
    let hello = Request::Hello {
        mac: our_mac(),
        running_sha256: firmware::running_sha256().ok_or("Can't read our image")?,
        version: firmware::version(),
    };
    call(&hello)
}

/// The board's base MAC address, from eFuse
fn our_mac() -> [u8; 6] {
    let mut mac = [0u8; 6];
    unsafe {
        esp_idf_svc::sys::esp_read_mac(
            mac.as_mut_ptr(),
            esp_idf_svc::sys::esp_mac_type_t_ESP_MAC_WIFI_STA,
        );
    }
    mac
}

/// How the last install went, for /api/status: None if there's been none
/// since boot
static LAST_INSTALL: Mutex<Option<String>> = Mutex::new(None);

pub fn install_state() -> Option<String> {
    LAST_INSTALL.lock().unwrap().clone()
}

/// Download `offer` into the spare app slot, a chunk at a time, telling
/// `progress` each new percent; check it's the image offered, and mark it
/// to boot. Ok: the caller reboots. Err: the running firmware stays as it
/// is (the spare slot's half an image is never booted). `cancel` raised
/// gives up at the next chunk
pub fn install(
    offer: &Offer,
    mut progress: impl FnMut(u8),
    cancel: &AtomicBool,
) -> Result<(), String> {
    let set_state = |state: String| *LAST_INSTALL.lock().unwrap() = Some(state);
    set_state(format!("installing {}: 0%", offer.version));
    let result = download(
        offer,
        &mut |percent| {
            set_state(format!("installing {}: {}%", offer.version, percent));
            progress(percent);
        },
        cancel,
    );
    set_state(match &result {
        Ok(()) => format!("installed {}, rebooting", offer.version),
        Err(e) => format!("{} not installed: {}", offer.version, e),
    });
    result
}

fn download(
    offer: &Offer,
    progress: &mut dyn FnMut(u8),
    cancel: &AtomicBool,
) -> Result<(), String> {
    let mut link = connect()?;
    let mut ota = EspOta::new().map_err(|e| e.to_string())?;
    let mut update = ota.initiate_update().map_err(|e| e.to_string())?;
    let mut offset = 0u32;
    let mut last_bytes = Vec::new(); // the image's end: its own SHA-256
    let mut shown = 0u8;
    while offset < offer.size {
        if cancel.load(Ordering::Relaxed) {
            let _ = update.abort();
            return Err("Cancelled".into());
        }
        let len = CHUNK.min((offer.size - offset).min(u16::MAX as u32) as u16);
        let request = Request::Chunk {
            sha256: offer.sha256,
            offset,
            len,
        };
        let bytes = match ask(&mut link, &request) {
            Ok(Response::Chunk(bytes)) if bytes.len() == len as usize => bytes,
            Ok(Response::Error(e)) => {
                let _ = update.abort();
                return Err(if e == IMAGE_CHANGED {
                    e
                } else {
                    format!("Server: {}", e)
                });
            }
            Ok(_) => {
                let _ = update.abort();
                return Err("Server: odd answer".into());
            }
            Err(e) => {
                let _ = update.abort();
                return Err(e);
            }
        };
        if let Err(e) = update.write(&bytes) {
            let _ = update.abort();
            return Err(format!("Flash: {}", e));
        }
        last_bytes.extend_from_slice(&bytes);
        let keep = last_bytes.len().saturating_sub(32);
        last_bytes.drain(..keep);
        offset += len as u32;
        let percent = (offset as u64 * 100 / offer.size as u64) as u8;
        if percent != shown {
            shown = percent;
            progress(percent);
        }
    }
    // The image ends in its own SHA-256, which ESP-IDF checks against the
    // whole image in complete(): so the bytes are whole, and are the
    // image offered
    if last_bytes != offer.sha256 {
        let _ = update.abort();
        return Err("Not the image offered".into());
    }
    update
        .complete()
        .map_err(|e| format!("Image refused: {}", e))
}
