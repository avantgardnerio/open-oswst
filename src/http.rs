//! The HTTP API, on port 80 once WiFi is up (net.rs). No authentication:
//! knowing the WiFi password is the security.
//!
//!   GET  /api/status      name, MAC, firmware (and whether it's confirmed),
//!                         OTA slot, mode, settings profile, uptime, heap,
//!                         the management server we dial and this radio's
//!                         public key for it (JSON)
//!   POST /api/ota         install /data/firmware.bin (`espflash save-image`,
//!                         put there over /fs/): into the spare app slot,
//!                         checked, then reboot. It must run a minute or the
//!                         old one comes back (firmware.rs)
//!   GET  /api/firmware    download the app image running
//!   GET  /api/screenshot  the screen as it is now, menus and all (1-bit BMP)
//!   POST /api/reboot
//!   POST /api/wifi/off    WiFi off until the next reboot (net.rs)
//!   PUT  /api/management  set the management server: {"host": "...",
//!                         "port": 3101, "server_public_key": "..."} (JSON),
//!                         into secrets.toml (devices/secrets.rs). The
//!                         radio's own key is its own: never set from here
//!   POST /api/management/update
//!                         ask the management server for firmware, and if
//!                         it has other than ours, install it and reboot
//!                         (the net thread does it: 202 at once, then
//!                         /api/status says how it's going). The menu's
//!                         Update does the same, with a question first
//!   POST /api/management/echo?bytes=N
//!                         N bytes (default 16, at most 4096) to the
//!                         management server and back, timed (JSON):
//!                         does this radio reach it (management.rs)
//!   /fs/...               the storage (/data) as a WebDAV folder, read and
//!                         write: config.toml, log/, anything (webdav.rs)
//!
//! Handlers run on the server's own task, never the app's or the radio's.

use std::path::Path;
use std::sync::mpsc::Sender;

use esp_idf_svc::hal::cpu::Core;
use esp_idf_svc::http::server::{Configuration, EspHttpConnection, EspHttpServer, Request};
use esp_idf_svc::http::Method;
use esp_idf_svc::io::Write;
use open_oswst_core::devices::management::{Job, CANCEL, JOBS};
use open_oswst_core::devices::screen;
use open_oswst_core::management::{self as protocol, key_from_hex, key_to_hex, Response};
use open_oswst_core::{config, mode};
use std::sync::atomic::Ordering;

use crate::devices::{secrets, storage};
use crate::{firmware, management, webdav};

/// Files go through this much at a time
const CHUNK: usize = 4096;

type Req<'a, 'r> = Request<&'a mut EspHttpConnection<'r>>;
type Result<T = ()> = std::result::Result<T, anyhow::Error>;

/// Start the server. It runs until the returned server is dropped.
/// `wifi_off` tells the net thread to switch WiFi off.
pub fn start(name: &str, mac: &str, wifi_off: Sender<()>) -> Option<EspHttpServer<'static>> {
    let config = Configuration {
        stack_size: 10240,
        core: Some(Core::Core0),
        uri_match_wildcard: true,
        ..Default::default()
    };
    let mut server = EspHttpServer::new(&config)
        .map_err(|e| log::error!("HTTP server failed to start: {}", e))
        .ok()?;
    let (name, mac) = (name.to_string(), mac.to_string());
    let routes = server
        .fn_handler("/api/status", Method::Get, move |req| {
            status(req, &name, &mac)
        })
        .and_then(|s| s.fn_handler("/api/ota", Method::Post, ota))
        .and_then(|s| s.fn_handler("/api/firmware", Method::Get, download_firmware))
        .and_then(|s| s.fn_handler("/api/screenshot", Method::Get, screenshot))
        .and_then(|s| s.fn_handler("/api/reboot", Method::Post, reboot))
        .and_then(|s| s.fn_handler("/api/management", Method::Put, put_management))
        .and_then(|s| s.fn_handler("/api/management/update", Method::Post, management_update))
        .and_then(|s| s.fn_handler("/api/management/echo", Method::Post, management_echo))
        .and_then(|s| {
            s.fn_handler("/api/wifi/off", Method::Post, move |req| {
                switch_wifi_off(req, &wifi_off)
            })
        })
        .map(|_| ())
        .and_then(|()| webdav::register(&mut server));
    if let Err(e) = routes {
        log::error!("HTTP routes failed: {}", e);
        return None;
    }
    log::info!("HTTP API on port 80");
    Some(server)
}

fn status(req: Req, name: &str, mac: &str) -> Result {
    let (firmware, slot, uptime_s, heap_free, heap_min) = unsafe {
        use esp_idf_svc::sys::*;
        // Which app partition is running: ota_0 or ota_1
        let running = &*esp_ota_get_running_partition();
        let slot = core::ffi::CStr::from_ptr(running.label.as_ptr()).to_string_lossy();
        (
            firmware::version(),
            slot.into_owned(),
            esp_timer_get_time() / 1_000_000,
            esp_get_free_heap_size(),
            esp_get_minimum_free_heap_size(),
        )
    };
    let server = secrets::server().ok();
    let status = Status {
        name,
        mac,
        firmware: &firmware,
        firmware_state: firmware::state(),
        slot: &slot,
        mode: mode::get().name(),
        profile: config::profile(),
        uptime_s,
        heap_free,
        heap_min,
        management: Management {
            host: server.as_ref().map(|server| server.host.clone()),
            port: server.as_ref().map(|server| server.port),
            server_public_key: server.as_ref().map(|server| key_to_hex(&server.public_key)),
            radio_public_key: secrets::public_key_hex(),
            last_install: management::install_state(),
        },
        image_sha256: firmware::running_sha256().map(|sha| key_to_hex(&sha)),
    };
    let mut body = serde_json::to_vec(&status)?;
    body.push(b'\n');
    req.into_response(200, None, &[("Content-Type", "application/json")])?
        .write_all(&body)?;
    Ok(())
}

/// What GET /status answers, as JSON
#[derive(serde::Serialize)]
struct Status<'a> {
    name: &'a str,
    mac: &'a str,
    /// The build's git hash (-dirty if built with uncommitted changes)
    firmware: &'a str,
    /// "valid", or "pending" for a new one not yet confirmed (firmware.rs)
    firmware_state: &'static str,
    /// The app partition running: ota_0 or ota_1
    slot: &'a str,
    mode: &'static str,
    /// Radios must have the same to hear each other (config.rs)
    profile: String,
    uptime_s: i64,
    heap_free: u32,
    heap_min: u32,
    management: Management,
    /// The running image's own SHA-256 (its last 32 bytes): what the
    /// management server tells images apart by
    image_sha256: Option<String>,
}

/// The management server we dial (as PUT /api/management set it), and this
/// radio's public key, for the server's radios.toml (devices/secrets.rs).
/// Never the private key
#[derive(serde::Serialize)]
struct Management {
    host: Option<String>,
    port: Option<u16>,
    server_public_key: Option<String>,
    /// None until WiFi has first come up: the radio makes its key then
    radio_public_key: Option<String>,
    /// How the last firmware install went, if there's been one since boot
    last_install: Option<String>,
}

/// Install /data/firmware.bin (put it there over WebDAV first), then reboot
/// into it. It boots pending: if it doesn't run a minute, the bootloader goes
/// back to the app running now (firmware.rs). The file stays
fn ota(req: Req) -> Result {
    let image = Path::new(storage::ROOT).join("firmware.bin");
    log::info!("OTA: installing {}", image.display());
    match firmware::install(&image) {
        Ok(size) => {
            log::info!("OTA: {} bytes installed, rebooting into them", size);
            req.into_ok_response()?
                .write_all(format!("ok, {} bytes, rebooting\n", size).as_bytes())?;
            firmware::reboot_soon();
        }
        Err(e) => {
            log::warn!("OTA: failed: {}", e);
            req.into_status_response(400)?
                .write_all(format!("not installed: {}\n", e).as_bytes())?;
        }
    }
    Ok(())
}

/// The running app image, read straight from its slot: a backup, or the
/// same build for another radio (put it there as /fs/firmware.bin)
fn download_firmware(req: Req) -> Result {
    let Some(len) = firmware::image_len() else {
        req.into_status_response(500)?
            .write_all(b"can't read the running image\n")?;
        return Ok(());
    };
    let mut response =
        req.into_response(200, None, &[("Content-Type", "application/octet-stream")])?;
    let mut buf = vec![0u8; CHUNK];
    let mut offset = 0;
    while offset < len as usize {
        let n = CHUNK.min(len as usize - offset);
        firmware::read(offset, &mut buf[..n])?;
        response.write_all(&buf[..n])?;
        offset += n;
    }
    Ok(())
}

/// The frame on the panel now, as a BMP. Unlike the menu's Screenshot (which
/// saves the radio screen under the menu), this catches the menus too
fn screenshot(req: Req) -> Result {
    let bmp = screen::on_panel().bmp();
    req.into_response(200, None, &[("Content-Type", "image/bmp")])?
        .write_all(&bmp)?;
    Ok(())
}

fn reboot(req: Req) -> Result {
    req.into_ok_response()?.write_all(b"rebooting\n")?;
    firmware::reboot_soon();
    Ok(())
}

/// Check with the management server, then hand an install to the net thread
fn management_update(req: Req) -> Result {
    match management::check() {
        Ok(Response::UpToDate) => text_reply(req, 200, "up to date"),
        Ok(Response::Offer(offer)) => {
            let reply = format!(
                "installing {} ({} B): see /api/status",
                offer.version, offer.size
            );
            CANCEL.store(false, Ordering::Relaxed);
            match JOBS.try_send(Job::Install(offer)) {
                Ok(()) => text_reply(req, 202, &reply),
                Err(_) => text_reply(req, 409, "busy with another job"),
            }
        }
        Ok(Response::Error(e)) | Err(e) => text_reply(req, 502, &e),
        Ok(other) => text_reply(req, 502, &format!("odd answer: {:?}", other)),
    }
}

/// What PUT /api/management takes
#[derive(serde::Deserialize)]
struct SetManagement {
    host: String,
    port: u16,
    server_public_key: String,
}

/// The biggest body PUT /api/management takes
const MANAGEMENT_MAX: usize = 1024;

fn put_management(mut req: Req) -> Result {
    let mut body = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        let n = req.read(&mut buf)?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buf[..n]);
        if body.len() > MANAGEMENT_MAX {
            return text_reply(req, 413, "too big");
        }
    }
    let wanted: SetManagement = match serde_json::from_slice(&body) {
        Ok(wanted) => wanted,
        Err(e) => return text_reply(req, 400, &format!("bad JSON: {}", e)),
    };
    let Some(key) = key_from_hex(&wanted.server_public_key) else {
        return text_reply(req, 400, "server_public_key must be 64 hex digits");
    };
    if wanted.host.is_empty() || wanted.port == 0 {
        return text_reply(req, 400, "needs a host and a port (1-65535)");
    }
    match secrets::set_server(&wanted.host, wanted.port, &key) {
        Ok(()) => {
            log::info!("Management server set (port {})", wanted.port);
            text_reply(req, 200, "ok")
        }
        Err(e) => text_reply(req, 500, &e),
    }
}

fn text_reply(req: Req, code: u16, text: &str) -> Result {
    req.into_response(code, None, &[("Content-Type", "text/plain")])?
        .write_all(format!("{}\n", text).as_bytes())?;
    Ok(())
}

/// The most /api/management/echo sends: its copy and the answer are both on
/// the heap at once
const ECHO_MAX: usize = 4096;

/// Echo through the management server: ?bytes=N there and back, timed.
/// Blocks this handler (the server's own task) for up to its timeouts
fn management_echo(req: Req) -> Result {
    let size = req
        .uri()
        .split_once("?bytes=")
        .and_then(|(_, n)| n.parse::<usize>().ok())
        .unwrap_or(16)
        .min(ECHO_MAX);
    let sent: Vec<u8> = (0..size).map(|i| i as u8).collect();
    let started = std::time::Instant::now();
    let answer = management::call(&protocol::Request::Echo(sent.clone()));
    let round_trip_ms = started.elapsed().as_millis() as u64;
    let (code, reply) = match answer {
        Ok(Response::Echo(got)) if got == sent => (200, EchoReply::ok(size, round_trip_ms)),
        Ok(Response::Echo(_)) => (502, EchoReply::failed("came back different", round_trip_ms)),
        Ok(Response::Error(e)) | Err(e) => (502, EchoReply::failed(&e, round_trip_ms)),
        Ok(_) => (502, EchoReply::failed("odd answer", round_trip_ms)),
    };
    log::info!("Management echo, {} B: {:?}", size, reply);
    let mut body = serde_json::to_vec(&reply)?;
    body.push(b'\n');
    req.into_response(code, None, &[("Content-Type", "application/json")])?
        .write_all(&body)?;
    Ok(())
}

/// What POST /api/management/echo answers, as JSON
#[derive(serde::Serialize, Debug)]
struct EchoReply {
    ok: bool,
    bytes: usize,
    round_trip_ms: u64,
    error: Option<String>,
}

impl EchoReply {
    fn ok(bytes: usize, round_trip_ms: u64) -> EchoReply {
        EchoReply {
            ok: true,
            bytes,
            round_trip_ms,
            error: None,
        }
    }

    fn failed(error: &str, round_trip_ms: u64) -> EchoReply {
        EchoReply {
            ok: false,
            bytes: 0,
            round_trip_ms,
            error: Some(error.into()),
        }
    }
}

/// Ask the net thread to switch WiFi off. It waits for this reply to go out
/// first. Not saved: the next boot has WiFi on again
fn switch_wifi_off(req: Req, wifi_off: &Sender<()>) -> Result {
    log::info!("WiFi: switching off until the next reboot");
    req.into_ok_response()?
        .write_all(b"ok, WiFi off until the next reboot\n")?;
    let _ = wifi_off.send(());
    Ok(())
}
