//! Talking to the management server (issue #34; server/ in this repo):
//! connect, the Noise handshake (core's management.rs, on mbedTLS:
//! noise.rs), then requests and their responses. Blocking, so it runs on a
//! thread of its own: the net thread for the menu's jobs and the installs
//! (net.rs), the HTTP API's for its endpoints (http.rs). Where the server is
//! and the keys: secrets.toml (devices/secrets.rs).
//!
//! Nothing here runs unless someone asked: the menu, or the HTTP API.

use std::cell::Cell;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use esp_idf_svc::ota::EspOta;
use open_oswst_core::bundle;
use open_oswst_core::devices::management::Installing;
use open_oswst_core::gzip::GzipReader;
use open_oswst_core::management::{
    key_from_hex, key_to_hex, NoiseLink, Offer, Request, Response, Sha256, CHUNK, IMAGE_CHANGED,
};
use open_oswst_core::tar::TarReader;
use snow::types::Hash as _;

use crate::devices::{secrets, storage};
use crate::firmware;
use crate::inflate::RomInflate;
use crate::noise::{self, MbedtlsResolver};

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

/// Is there another update bundle for us? UpToDate, BundleOffer, or the
/// server's Error
pub fn check() -> Result<Response, String> {
    let request = Request::CheckBundle {
        mac: our_mac(),
        running_sha256: firmware::running_sha256().ok_or("Can't read our image")?,
        installed_sha256: installed_sha256(),
        version: firmware::version(),
    };
    call(&request)
}

/// The SHA-256 of the bundle installed last (INSTALLED), if any
fn installed_sha256() -> Option<Sha256> {
    // Hex, like the keys: the same 32 bytes
    fs::read_to_string(INSTALLED)
        .ok()
        .and_then(|text| key_from_hex(&text))
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

/// Where a bundle is downloaded to, and extracted from (deleted after)
const DOWNLOAD: &str = "/data/update.tar.gz";
/// The SHA-256 of the bundle installed last, as hex: CheckBundle tells it
/// to the server
const INSTALLED: &str = "/data/bundle.sha256";
/// Free storage left over after a download, for the logs meanwhile
const ROOM_TO_SPARE: usize = 64 * 1024;
/// Files are read and written this much at a time
const COPY: usize = 4096;

/// Install the bundle `offer` (core's bundle.rs) in three steps, telling
/// `progress` each new percent of each:
/// 1. download it to the storage, from the management server
/// 2. verify its SHA-256 against the offer's
/// 3. extract it: the firmware into the spare app slot, marked to boot
///    only once every file is written; the files onto the storage
///
/// Ok: the caller reboots. Err: the running firmware stays (the spare
/// slot's image is never marked to boot), but files already extracted stay
/// extracted. `cancel` raised gives up at the next chunk or file
pub fn install(
    offer: &Offer,
    mut progress: impl FnMut(Installing),
    cancel: &AtomicBool,
) -> Result<(), String> {
    let mut steps = Steps {
        version: &offer.version,
        progress: &mut progress,
        shown: None,
    };
    let result = download(offer, &mut steps, cancel)
        .and_then(|()| verify(offer, &mut steps))
        .and_then(|()| extract(offer, &mut steps, cancel));
    let _ = fs::remove_file(DOWNLOAD);
    if result.is_ok() {
        let _ = fs::write(INSTALLED, key_to_hex(&offer.sha256));
    }
    set_state(match &result {
        Ok(()) => format!("installed {}, rebooting", offer.version),
        Err(e) => format!("{} not installed: {}", offer.version, e),
    });
    result
}

fn set_state(state: String) {
    *LAST_INSTALL.lock().unwrap() = Some(state);
}

/// Tells the menu (and /api/status) which step, and each new percent
struct Steps<'a> {
    version: &'a str,
    progress: &'a mut dyn FnMut(Installing),
    shown: Option<Installing>,
}

impl Steps<'_> {
    fn update(&mut self, step: fn(u8) -> Installing, done: u64, total: u64) {
        let percent = (done * 100 / total.max(1)).min(100) as u8;
        let news = step(percent);
        if self.shown.as_ref() != Some(&news) {
            let (step, percent) = match news {
                Installing::Downloading(percent) => ("downloading", percent),
                Installing::Verifying(percent) => ("verifying", percent),
                Installing::Extracting(percent) => ("extracting", percent),
                _ => ("installing", 0),
            };
            set_state(format!("{} {}: {}%", step, self.version, percent));
            (self.progress)(news.clone());
            self.shown = Some(news);
        }
    }
}

/// Step 1: the bundle, a chunk at a time, into DOWNLOAD
fn download(offer: &Offer, steps: &mut Steps, cancel: &AtomicBool) -> Result<(), String> {
    let (used, total) = storage::usage();
    if total.saturating_sub(used) < offer.size as usize + ROOM_TO_SPARE {
        return Err("No room: delete logs".into());
    }
    steps.update(Installing::Downloading, 0, offer.size as u64);
    let mut link = connect()?;
    let mut file = File::create(DOWNLOAD).map_err(|e| format!("Storage: {}", e.kind()))?;
    let mut offset = 0u32;
    while offset < offer.size {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        let len = CHUNK.min((offer.size - offset).min(u16::MAX as u32) as u16);
        let request = Request::BundleChunk {
            sha256: offer.sha256,
            offset,
            len,
        };
        let bytes = match ask(&mut link, &request)? {
            Response::Chunk(bytes) if bytes.len() == len as usize => bytes,
            Response::Error(e) if e == IMAGE_CHANGED => return Err(e),
            Response::Error(e) => return Err(format!("Server: {}", e)),
            _ => return Err("Server: odd answer".into()),
        };
        file.write_all(&bytes)
            .map_err(|e| format!("Storage: {}", e.kind()))?;
        offset += len as u32;
        steps.update(Installing::Downloading, offset as u64, offer.size as u64);
    }
    file.sync_all()
        .map_err(|e| format!("Storage: {}", e.kind()))
}

/// Step 2: DOWNLOAD's SHA-256 (on the SHA hardware) is the offer's: every
/// byte came, and is the bundle offered
fn verify(offer: &Offer, steps: &mut Steps) -> Result<(), String> {
    let mut file = File::open(DOWNLOAD).map_err(|e| format!("Storage: {}", e.kind()))?;
    let mut sha = noise::Sha256::new();
    let mut buf = vec![0u8; COPY];
    let mut done = 0u64;
    loop {
        let read = file
            .read(&mut buf)
            .map_err(|e| format!("Storage: {}", e.kind()))?;
        if read == 0 {
            break;
        }
        sha.input(&buf[..read]);
        done += read as u64;
        steps.update(Installing::Verifying, done, offer.size as u64);
    }
    let mut sha256 = [0u8; 32];
    sha.result(&mut sha256);
    if sha256 != offer.sha256 {
        return Err("Not the update offered".into());
    }
    Ok(())
}

/// Step 3: inflate DOWNLOAD and walk its tar: the firmware into the spare
/// app slot, each other file onto the storage at its path
/// (bundle::extract_to says which, and leaves our settings and secrets
/// alone)
fn extract(offer: &Offer, steps: &mut Steps, cancel: &AtomicBool) -> Result<(), String> {
    let file = File::open(DOWNLOAD).map_err(|e| format!("Storage: {}", e.kind()))?;
    // Progress: how much of the .tar.gz has been read
    let read = Rc::new(Cell::new(0u64));
    let counted = Counted {
        inner: file,
        read: read.clone(),
    };
    let (inflate, mut window) = RomInflate::new().ok_or("No PSRAM")?;
    let gzip = GzipReader::new(counted, inflate, &mut window).map_err(|e| e.to_string())?;
    let mut tar = TarReader::new(gzip);
    let mut ota = EspOta::new().map_err(|e| e.to_string())?;
    // Taken by the bundle's firmware: one per bundle
    let mut spare_slot = Some(&mut ota);
    // Written but not marked to boot until every file is out. Dropped
    // unfinished (any error), it's abandoned
    let mut firmware = None;

    while let Some(entry) = tar.next_entry().map_err(|e| format!("Bundle: {}", e))? {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        if !entry.is_file {
            continue;
        }
        if entry.path == bundle::FIRMWARE {
            let slot = spare_slot.take().ok_or("Bundle: two firmwares")?;
            let mut update = slot.initiate_update().map_err(|e| e.to_string())?;
            // Most of the bundle: tell how far along as it goes
            copy(&mut tar, |bytes| {
                update.write(bytes).map_err(|e| format!("Flash: {}", e))?;
                steps.update(Installing::Extracting, read.get(), offer.size as u64);
                Ok(())
            })?;
            firmware = Some(update);
        } else if let Some(path) = bundle::extract_to(&entry.path) {
            write_file(&Path::new(storage::ROOT).join(path), &mut tar)?;
            log::info!("Update: wrote {} ({} B)", path, entry.size);
        } else {
            log::warn!("Update: left {} alone", entry.path);
        }
        steps.update(Installing::Extracting, read.get(), offer.size as u64);
    }
    if let Some(update) = firmware {
        // ESP-IDF checks the whole image (its SHA-256) before marking it
        update
            .complete()
            .map_err(|e| format!("Image refused: {}", e))?;
    }
    steps.update(Installing::Extracting, 1, 1);
    Ok(())
}

/// A file from the bundle: written beside its place, then renamed over it,
/// so a failure part way leaves the old one whole
fn write_file(path: &Path, from: &mut impl Read) -> Result<(), String> {
    let storage = |e: std::io::Error| format!("Storage: {}", e.kind());
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).map_err(storage)?;
    }
    let mut new_name = path.as_os_str().to_owned();
    new_name.push(".new");
    let new_path = Path::new(&new_name);
    let mut file = File::create(new_path).map_err(storage)?;
    copy(from, |bytes| file.write_all(bytes).map_err(storage))?;
    file.sync_all().map_err(storage)?;
    drop(file);
    // LittleFS's rename replaces an existing file
    fs::rename(new_path, path).map_err(storage)
}

/// Everything `from` has, COPY bytes at a time, to `to`
fn copy(
    from: &mut impl Read,
    mut to: impl FnMut(&[u8]) -> Result<(), String>,
) -> Result<(), String> {
    let mut buf = vec![0u8; COPY];
    loop {
        let read = from.read(&mut buf).map_err(|e| format!("Bundle: {}", e))?;
        if read == 0 {
            return Ok(());
        }
        to(&buf[..read])?;
    }
}

/// A reader that counts what's read through it
struct Counted<R> {
    inner: R,
    read: Rc<Cell<u64>>,
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.read.set(self.read.get() + read as u64);
        Ok(read)
    }
}
