//! WebDAV (RFC 4918, class 1) over the radio's storage: everything under
//! /data (config.toml, the logs, anything put there) is a folder at /fs/.
//! Any client works: a file manager at dav://oswst-XXXX.local/fs/, gio,
//! davfs2, rclone, curl.
//!
//!   PROPFIND  list a folder (Depth 0: the folder itself; otherwise one level
//!             down) or describe a file
//!   GET       read a file
//!   PUT       write a file: into a temporary file first, then renamed over
//!             the old one, so a broken upload never replaces anything
//!   DELETE    a file, or a folder and everything in it
//!   MKCOL     make a folder
//!   MOVE      rename (some clients save by uploading under a temporary name)
//!   OPTIONS   says the above
//!
//! Two rules on top: a write that lands on config.toml must pass the settings
//! check, or it's refused and the old file stays; and secrets.toml
//! (devices/secrets.rs) isn't here at all: left out of listings, and no
//! method reaches it (local_path). /api/management (http.rs) is its way in.
//!
//! No LOCK: Linux clients don't need it, but Finder and Windows Explorer may
//! refuse to write. Modification dates are listed only when they're real:
//! a file written before the clock was set (clock.rs) is dated 1970, and
//! gets none. Listings are written as they're read, never held whole.

use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use esp_idf_svc::http::server::{EspHttpConnection, EspHttpServer, Request};
use esp_idf_svc::http::Method;
use esp_idf_svc::io::Write;
use esp_idf_svc::sys::EspError;

use open_oswst_core::utc;

use crate::devices::{secrets, settings, storage};

type Req<'a, 'r> = Request<&'a mut EspHttpConnection<'r>>;
type Result<T = ()> = std::result::Result<T, anyhow::Error>;

/// Where the folder appears in the URL
const PREFIX: &str = "/fs";
/// Files go through this much at a time
const CHUNK: usize = 4096;
/// The biggest config.toml accepted
const MAX_CONFIG: usize = 16 * 1024;
const ALLOW: &str = "OPTIONS, PROPFIND, GET, PUT, DELETE, MKCOL, MOVE";

/// Add the WebDAV handlers to the server
pub fn register(server: &mut EspHttpServer<'static>) -> std::result::Result<(), EspError> {
    for uri in [PREFIX, "/fs/*"] {
        server
            .fn_handler(uri, Method::Options, options)?
            .fn_handler(uri, Method::Propfind, propfind)?
            .fn_handler(uri, Method::Get, get)?
            .fn_handler(uri, Method::Put, put)?
            .fn_handler(uri, Method::Delete, delete)?
            .fn_handler(uri, Method::MkCol, mkcol)?
            .fn_handler(uri, Method::Move, move_to)?;
    }
    Ok(())
}

fn options(req: Req) -> Result {
    req.into_response(200, None, &[("DAV", "1"), ("Allow", ALLOW)])?;
    Ok(())
}

fn propfind(req: Req) -> Result {
    let Some(path) = local_path(req.uri()) else {
        return reply(req, 400, "bad path");
    };
    let Ok(meta) = fs::metadata(&path) else {
        return reply(req, 404, "not found");
    };
    let depth_0 = req.header("Depth") == Some("0");

    let mut response = req.into_response(
        207,
        Some("Multi-Status"),
        &[("Content-Type", "application/xml; charset=utf-8")],
    )?;
    response.write_all(b"<?xml version=\"1.0\" encoding=\"utf-8\"?>\n")?;
    response.write_all(b"<D:multistatus xmlns:D=\"DAV:\">\n")?;
    response.write_all(entry(&path, &meta).as_bytes())?;
    if meta.is_dir() && !depth_0 {
        for item in fs::read_dir(&path)?.flatten() {
            if secrets::is_secret(&item.path()) {
                continue;
            }
            if let Ok(meta) = item.metadata() {
                response.write_all(entry(&item.path(), &meta).as_bytes())?;
            }
        }
    }
    response.write_all(b"</D:multistatus>\n")?;
    Ok(())
}

fn get(req: Req) -> Result {
    let Some(path) = local_path(req.uri()) else {
        return reply(req, 400, "bad path");
    };
    if !path.is_file() {
        return reply(req, 404, "not a file");
    }
    let mut file = fs::File::open(&path)?;
    let mut response = req.into_response(200, None, &[("Content-Type", content_type(&path))])?;
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        response.write_all(&buf[..n])?;
    }
}

fn put(mut req: Req) -> Result {
    let Some(path) = local_path(req.uri()) else {
        return reply(req, 400, "bad path");
    };
    if path.is_dir() {
        return reply(req, 405, "that's a folder");
    }
    let Some(name) = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
    else {
        return reply(req, 400, "bad path");
    };
    if !path.parent().is_some_and(Path::is_dir) {
        return reply(req, 409, "no such folder");
    }
    let existed = path.exists();
    let part = path.with_file_name(format!(".{}.part", name));

    if is_config(&path) {
        let Ok(body) = read_body(&mut req, MAX_CONFIG) else {
            return reply(req, 413, "config.toml too big");
        };
        if let Err(e) = check_config(&body) {
            return reply(req, 400, &format!("rejected: {}", e));
        }
        fs::write(&part, &body)?;
    } else {
        let mut file = fs::File::create(&part)?;
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = match req.read(&mut buf) {
                Ok(n) => n,
                Err(e) => {
                    drop(file);
                    let _ = fs::remove_file(&part);
                    return Err(e.into());
                }
            };
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
        }
        file.sync_all()?;
    }
    fs::rename(&part, &path)?;
    log::info!("WebDAV: wrote {}", path.display());
    reply(req, if existed { 204 } else { 201 }, "")
}

fn delete(req: Req) -> Result {
    let Some(path) = local_path(req.uri()) else {
        return reply(req, 400, "bad path");
    };
    if path == Path::new(storage::ROOT) {
        return reply(req, 403, "not the whole storage");
    }
    let removed = if path.is_dir() {
        fs::remove_dir_all(&path)
    } else {
        fs::remove_file(&path)
    };
    match removed {
        Ok(()) => {
            log::info!("WebDAV: deleted {}", path.display());
            reply(req, 204, "")
        }
        Err(_) => reply(req, 404, "not found"),
    }
}

fn mkcol(req: Req) -> Result {
    let Some(path) = local_path(req.uri()) else {
        return reply(req, 400, "bad path");
    };
    if path.exists() {
        return reply(req, 405, "already exists");
    }
    if !path.parent().is_some_and(Path::is_dir) {
        return reply(req, 409, "no such folder");
    }
    fs::create_dir(&path)?;
    reply(req, 201, "")
}

/// MOVE: rename to the path in the Destination header (a full URL, or just
/// the path). Replaces what's there, unless the header Overwrite: F says not to
fn move_to(req: Req) -> Result {
    let Some(from) = local_path(req.uri()) else {
        return reply(req, 400, "bad path");
    };
    let to = req.header("Destination").map(url_path).and_then(local_path);
    let Some(to) = to else {
        return reply(req, 400, "bad destination");
    };
    if !from.exists() {
        return reply(req, 404, "not found");
    }
    // The secrets would go with it, and be reachable at the new name
    if from == Path::new(storage::ROOT) {
        return reply(req, 403, "not the whole storage");
    }
    let existed = to.exists();
    if existed && req.header("Overwrite") == Some("F") {
        return reply(req, 412, "destination exists");
    }
    if !to.parent().is_some_and(Path::is_dir) {
        return reply(req, 409, "no such folder");
    }
    if is_config(&to) {
        let body = fs::read(&from)?;
        if let Err(e) = check_config(&body) {
            return reply(req, 400, &format!("rejected: {}", e));
        }
    }
    if existed && to.is_dir() {
        fs::remove_dir_all(&to)?;
    }
    fs::rename(&from, &to)?;
    log::info!("WebDAV: moved {} to {}", from.display(), to.display());
    reply(req, if existed { 204 } else { 201 }, "")
}

/// A response with a status and a short text body
fn reply(req: Req, status: u16, text: &str) -> Result {
    let mut response = req.into_status_response(status)?;
    if !text.is_empty() {
        response.write_all(format!("{}\n", text).as_bytes())?;
    }
    Ok(())
}

/// One entry of a PROPFIND reply: a folder, or a file with its size
fn entry(path: &Path, meta: &fs::Metadata) -> String {
    let props = if meta.is_dir() {
        "<D:resourcetype><D:collection/></D:resourcetype>".to_string()
    } else {
        format!(
            "<D:resourcetype/><D:getcontentlength>{}</D:getcontentlength>\
             <D:getcontenttype>{}</D:getcontenttype>{}",
            meta.len(),
            content_type(path),
            last_modified(meta)
        )
    };
    format!(
        "<D:response><D:href>{}</D:href><D:propstat><D:prop>{}</D:prop>\
         <D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>\n",
        href(path, meta.is_dir()),
        props
    )
}

/// A file's modification date, if it's a real one (written after the clock
/// was set): before 2024 means 1970-something
fn last_modified(meta: &fs::Metadata) -> String {
    const REAL_AFTER: i64 = 1_704_067_200; // 2024-01-01
    let seconds = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_secs() as i64);
    if seconds < REAL_AFTER {
        return String::new();
    }
    format!(
        "<D:getlastmodified>{}</D:getlastmodified>",
        utc::http_date(seconds)
    )
}

/// The file or folder a URL path names, under storage::ROOT. None if it isn't
/// under PREFIX, isn't valid once decoded, tries to climb out with `..`, or
/// is a secret: every method finds its path here, a MOVE's destination too
fn local_path(uri: &str) -> Option<PathBuf> {
    let path = uri.split('?').next()?;
    let rest = path.strip_prefix(PREFIX)?;
    if !rest.is_empty() && !rest.starts_with('/') {
        return None;
    }
    let mut local = PathBuf::from(storage::ROOT);
    for segment in rest.split('/').filter(|segment| !segment.is_empty()) {
        let segment = percent_decode(segment)?;
        if segment == "." || segment == ".." || segment.contains('/') {
            return None;
        }
        local.push(segment);
    }
    (!secrets::is_secret(&local)).then_some(local)
}

/// The URL path for a local path: PREFIX, then each part percent-encoded,
/// with a trailing / for a folder
fn href(path: &Path, is_dir: bool) -> String {
    let relative = path.strip_prefix(storage::ROOT).unwrap_or(path);
    let mut href = String::from(PREFIX);
    for part in relative.iter() {
        href.push('/');
        href.push_str(&percent_encode(&part.to_string_lossy()));
    }
    if is_dir || href == PREFIX {
        href.push('/');
    }
    href
}

/// "http://host/fs/a" or "/fs/a" -> "/fs/a"
fn url_path(destination: &str) -> &str {
    match destination.find("://") {
        Some(scheme) => {
            let after = &destination[scheme + 3..];
            after.find('/').map_or("/", |path| &after[path..])
        }
        None => destination,
    }
}

/// Everything but letters, digits and -._~ as %XX
fn percent_encode(text: &str) -> String {
    let mut encoded = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{:02X}", byte));
        }
    }
    encoded
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("txt" | "toml" | "md" | "csv") => "text/plain",
        Some("json") => "application/json",
        // The web app (www/ in the repo): a browser shows a page only as
        // text/html, and runs a module script only with a JavaScript type
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript",
        Some("css") => "text/css",
        _ => "application/octet-stream",
    }
}

fn is_config(path: &Path) -> bool {
    path == Path::new(&settings::path())
}

fn check_config(body: &[u8]) -> std::result::Result<(), String> {
    let text = std::str::from_utf8(body).map_err(|e| e.to_string())?;
    settings::check(text)
}

/// The whole request body, up to `max` bytes
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
