//! The HTTP API, on port 80 once WiFi is up (net.rs). No authentication:
//! knowing the WiFi password is the security.
//!
//!   GET  /api/status      name, MAC, firmware (and whether it's confirmed),
//!                         OTA slot, mode, uptime, heap (JSON)
//!   POST /api/ota         install /data/firmware.bin (`espflash save-image`,
//!                         put there over /fs/): into the spare app slot,
//!                         checked, then reboot. It must run a minute or the
//!                         old one comes back (firmware.rs)
//!   GET  /api/firmware    download the app image running
//!   POST /api/reboot
//!   POST /api/wifi/off    WiFi off until the next reboot (net.rs)
//!   /fs/...               the storage (/data) as a WebDAV folder, read and
//!                         write: config.toml, log/, anything (webdav.rs)
//!
//! Handlers run on the server's own task, never the app's or the radio's.

use std::path::Path;
use std::sync::mpsc::Sender;
use std::time::Duration;

use esp_idf_svc::hal::cpu::Core;
use esp_idf_svc::http::server::{Configuration, EspHttpConnection, EspHttpServer, Request};
use esp_idf_svc::http::Method;
use esp_idf_svc::io::Write;
use open_oswst_core::mode;

use crate::devices::storage;
use crate::{firmware, webdav};

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
        .and_then(|s| s.fn_handler("/api/reboot", Method::Post, reboot))
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
        let app = &*esp_app_get_description();
        let version = core::ffi::CStr::from_ptr(app.version.as_ptr()).to_string_lossy();
        // Which app partition is running: ota_0 or ota_1
        let running = &*esp_ota_get_running_partition();
        let slot = core::ffi::CStr::from_ptr(running.label.as_ptr()).to_string_lossy();
        (
            version.into_owned(),
            slot.into_owned(),
            esp_timer_get_time() / 1_000_000,
            esp_get_free_heap_size(),
            esp_get_minimum_free_heap_size(),
        )
    };
    let status = Status {
        name,
        mac,
        firmware: &firmware,
        firmware_state: firmware::state(),
        slot: &slot,
        mode: mode::get().name(),
        uptime_s,
        heap_free,
        heap_min,
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
    uptime_s: i64,
    heap_free: u32,
    heap_min: u32,
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
            reboot_soon();
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

fn reboot(req: Req) -> Result {
    req.into_ok_response()?.write_all(b"rebooting\n")?;
    reboot_soon();
    Ok(())
}

/// Reboot after a moment, so the response gets out first
fn reboot_soon() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(500));
        unsafe { esp_idf_svc::sys::esp_restart() };
    });
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
