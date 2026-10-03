//! The HTTP API, on port 80 once WiFi is up (net.rs). No authentication:
//! knowing the WiFi password is the security.
//!
//!   GET  /status          name, MAC, firmware, OTA slot, mode, uptime, heap (JSON)
//!   /logs/                the log files, as a read-only WebDAV folder:
//!                         PROPFIND lists them, GET /logs/NNNN.txt fetches one
//!   GET  /config          config.toml
//!   PUT  /config          replace config.toml (checked first); applies on reboot
//!   POST /ota             a new app image (`espflash save-image`): written to
//!                         the spare OTA slot, then the board reboots into it
//!   POST /reboot
//!   POST /wifi/off        WiFi off until the next reboot (net.rs)
//!
//! Handlers run on the server's own task, never the app's or the radio's.

use std::fs;
use std::io::Read as _;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::Duration;

use esp_idf_svc::hal::cpu::Core;
use esp_idf_svc::http::server::{Configuration, EspHttpConnection, EspHttpServer, Request};
use esp_idf_svc::http::Method;
use esp_idf_svc::io::Write;
use esp_idf_svc::ota::EspOta;
use open_oswst_core::logger;
use open_oswst_core::mode::{self, Mode};

use crate::devices::{settings, storage};

/// Request bodies and files go through this much at a time
const CHUNK: usize = 4096;
/// The biggest config.toml accepted
const MAX_CONFIG: usize = 16 * 1024;

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
        .fn_handler("/status", Method::Get, move |req| status(req, &name, &mac))
        .and_then(|s| s.fn_handler("/logs", Method::Options, dav_options))
        .and_then(|s| s.fn_handler("/logs/*", Method::Options, dav_options))
        .and_then(|s| s.fn_handler("/logs", Method::Propfind, dav_propfind))
        .and_then(|s| s.fn_handler("/logs/*", Method::Propfind, dav_propfind))
        .and_then(|s| s.fn_handler("/logs/*", Method::Get, get_log))
        .and_then(|s| s.fn_handler("/config", Method::Get, get_config))
        .and_then(|s| s.fn_handler("/config", Method::Put, put_config))
        .and_then(|s| s.fn_handler("/ota", Method::Post, ota))
        .and_then(|s| s.fn_handler("/reboot", Method::Post, reboot))
        .and_then(|s| {
            s.fn_handler("/wifi/off", Method::Post, move |req| {
                switch_wifi_off(req, &wifi_off)
            })
        });
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
    let body = format!(
        concat!(
            "{{\"name\":\"{}\",\"mac\":\"{}\",\"firmware\":\"{}\",\"slot\":\"{}\",\"mode\":\"{}\",",
            "\"uptime_s\":{},\"heap_free\":{},\"heap_min\":{}}}\n"
        ),
        name,
        mac,
        firmware,
        slot,
        mode::get().name(),
        uptime_s,
        heap_free,
        heap_min
    );
    req.into_response(200, None, &[("Content-Type", "application/json")])?
        .write_all(body.as_bytes())?;
    Ok(())
}

fn log_dir() -> String {
    format!("{}/log", storage::ROOT)
}

// --- The logs, as a read-only WebDAV folder (RFC 4918) ---
//
// Any WebDAV client lists and copies them like a folder: a file manager at
// dav://oswst-XXXX.local/logs/, davfs2, rclone. PROPFIND lists (sizes only:
// the radio's clock isn't set to real time, so no dates), GET fetches. The
// listing is written one entry at a time, never held whole.

fn dav_options(req: Req) -> Result {
    req.into_response(
        200,
        None,
        &[("DAV", "1"), ("Allow", "OPTIONS, GET, PROPFIND")],
    )?;
    Ok(())
}

/// PROPFIND /logs/ (Depth 0: the folder; otherwise the folder and its files)
/// or PROPFIND /logs/NNNN.txt (that file)
fn dav_propfind(req: Req) -> Result {
    let name = req
        .uri()
        .trim_start_matches("/logs")
        .trim_start_matches('/')
        .to_string();
    let depth_0 = req.header("Depth") == Some("0");
    let dir = PathBuf::from(log_dir());

    if !name.is_empty() {
        let size = match plain_log_name(&name).then(|| fs::metadata(dir.join(&name))) {
            Some(Ok(meta)) => meta.len(),
            _ => {
                req.into_status_response(404)?.write_all(b"no such log\n")?;
                return Ok(());
            }
        };
        let mut response = req.into_response(207, Some("Multi-Status"), DAV_XML)?;
        response.write_all(DAV_START.as_bytes())?;
        response.write_all(dav_file(&name, size).as_bytes())?;
        response.write_all(DAV_END.as_bytes())?;
        return Ok(());
    }

    let mut response = req.into_response(207, Some("Multi-Status"), DAV_XML)?;
    response.write_all(DAV_START.as_bytes())?;
    response.write_all(DAV_FOLDER.as_bytes())?;
    if !depth_0 {
        let mut numbers = logger::log_file_numbers(&dir);
        numbers.sort_unstable();
        for number in numbers {
            let path = logger::log_path(&dir, number);
            let size = fs::metadata(&path).map_or(0, |meta| meta.len());
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            response.write_all(dav_file(&name, size).as_bytes())?;
        }
    }
    response.write_all(DAV_END.as_bytes())?;
    Ok(())
}

const DAV_XML: &[(&str, &str)] = &[("Content-Type", "application/xml; charset=utf-8")];
const DAV_START: &str =
    "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:multistatus xmlns:D=\"DAV:\">\n";
const DAV_END: &str = "</D:multistatus>\n";
const DAV_FOLDER: &str = "<D:response><D:href>/logs/</D:href><D:propstat><D:prop>\
    <D:resourcetype><D:collection/></D:resourcetype></D:prop>\
    <D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>\n";

/// One file's entry in a PROPFIND reply
fn dav_file(name: &str, size: u64) -> String {
    format!(
        "<D:response><D:href>/logs/{}</D:href><D:propstat><D:prop><D:resourcetype/>\
         <D:getcontentlength>{}</D:getcontentlength>\
         <D:getcontenttype>text/plain</D:getcontenttype></D:prop>\
         <D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>\n",
        name, size
    )
}

/// Only plain file names: nothing outside the log directory
fn plain_log_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && !name.contains("..")
}

fn get_log(req: Req) -> Result {
    let name = req.uri().trim_start_matches("/logs/").to_string();
    if !plain_log_name(&name) {
        req.into_status_response(400)?
            .write_all(b"bad file name\n")?;
        return Ok(());
    }
    let Ok(mut file) = fs::File::open(format!("{}/{}", log_dir(), name)) else {
        req.into_status_response(404)?.write_all(b"no such log\n")?;
        return Ok(());
    };
    let mut resp = req.into_response(200, None, &[("Content-Type", "text/plain")])?;
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        resp.write_all(&buf[..n])?;
    }
}

fn get_config(req: Req) -> Result {
    match fs::read_to_string(settings::path()) {
        Ok(text) => req
            .into_response(200, None, &[("Content-Type", "text/plain")])?
            .write_all(text.as_bytes())?,
        Err(_) => req
            .into_status_response(404)?
            .write_all(b"no config.toml\n")?,
    }
    Ok(())
}

/// Replace config.toml, but only with one that parses and has a known mode
fn put_config(mut req: Req) -> Result {
    let body = read_body(&mut req, MAX_CONFIG)?;
    let checked = std::str::from_utf8(&body)
        .map_err(|e| e.to_string())
        .and_then(|text| {
            let table = text.parse::<toml::Table>().map_err(|e| e.to_string())?;
            match table
                .get("mode")
                .map(|mode| mode.as_str().and_then(Mode::from_name))
            {
                Some(None) => Err("mode must be normal, repeater or echo".to_string()),
                _ => Ok(text),
            }
        });
    match checked {
        Ok(text) => {
            settings::replace(text)?;
            log::info!("HTTP: config.toml replaced ({} bytes)", text.len());
            req.into_ok_response()?
                .write_all(b"saved, applies on reboot (POST /reboot)\n")?;
        }
        Err(e) => {
            req.into_status_response(400)?
                .write_all(format!("rejected: {}\n", e).as_bytes())?;
        }
    }
    Ok(())
}

/// Stream a new app image into the spare OTA slot, then reboot into it
fn ota(mut req: Req) -> Result {
    log::info!("OTA: receiving an image");
    let mut ota = EspOta::new()?;
    let mut update = ota.initiate_update()?;
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0usize;
    loop {
        let n = match req.read(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                update.abort()?;
                return Err(e.into());
            }
        };
        if n == 0 {
            break;
        }
        update.write(&buf[..n])?;
        total += n;
    }
    // complete() checks the image before marking the slot to boot
    update.complete()?;
    log::info!("OTA: {} bytes written, rebooting into them", total);
    req.into_ok_response()?
        .write_all(format!("ok, {} bytes, rebooting\n", total).as_bytes())?;
    reboot_soon();
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

fn read_body(req: &mut Req, max: usize) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let n = req.read(&mut buf)?;
        if n == 0 {
            return Ok(body);
        }
        if body.len() + n > max {
            anyhow::bail!("body over {} bytes", max);
        }
        body.extend_from_slice(&buf[..n]);
    }
}
