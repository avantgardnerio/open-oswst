//! THE firmware radios converge on: DIR/firmware.bin, an app image as
//! `espflash save-image` writes it (scripts/publish-firmware.sh puts it
//! there). Re-read whenever the file changes, so publishing needs no
//! restart. Publish by renaming a finished file over it: a radio part way
//! through a download is then told IMAGE_CHANGED, never sent half of each.

use open_oswst_core::management::Sha256;
use sha2::Digest;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

pub const FIRMWARE_FILE: &str = "firmware.bin";

pub struct Image {
    pub bytes: Vec<u8>,
    /// The image's own SHA-256: its last 32 bytes, checked when read
    pub sha256: Sha256,
    /// The build's version string (git hash), from its app description
    pub version: String,
}

pub struct Firmware {
    path: PathBuf,
    /// The image as last read, and the file's modification time then
    cached: Mutex<Option<(SystemTime, Arc<Image>)>>,
}

impl Firmware {
    pub fn new(path: PathBuf) -> Firmware {
        Firmware {
            path,
            cached: Mutex::new(None),
        }
    }

    /// The image now: read again if the file changed. Err in a few words,
    /// for a radio's screen
    pub fn image(&self) -> Result<Arc<Image>, String> {
        let modified = fs::metadata(&self.path)
            .and_then(|meta| meta.modified())
            .map_err(|_| "No firmware on server")?;
        let mut cached = self.cached.lock().unwrap();
        if let Some((when, image)) = &*cached {
            if *when == modified {
                return Ok(image.clone());
            }
        }
        let bytes = fs::read(&self.path).map_err(|_| "No firmware on server")?;
        let image = Arc::new(parse(bytes).map_err(|e| {
            eprintln!("{}: {}", self.path.display(), e);
            "Server firmware broken"
        })?);
        println!(
            "Firmware: {} ({} B, sha256 {})",
            image.version,
            image.bytes.len(),
            short_hex(&image.sha256)
        );
        *cached = Some((modified, image.clone()));
        Ok(image)
    }
}

/// An ESP-IDF app image, checked: its header's magic, the SHA-256 it ends
/// in matching the rest, and its app description's version
fn parse(bytes: Vec<u8>) -> Result<Image, String> {
    // The image header: magic 0xE9, and at 23 whether a SHA-256 is appended
    const HASH_APPENDED: usize = 23;
    // The app description: after the image header (24 B) and the first
    // segment's header (8 B), 256 B long. Then the custom one, which holds
    // our version (the firmware's firmware.rs VERSION): ESP-IDF's own goes
    // stale
    const APP_DESC: usize = 32;
    const APP_DESC_MAGIC: u32 = 0xABCD_5432;
    const VERSION: usize = APP_DESC + 256;

    if bytes.len() < VERSION + 32 + 32 || bytes[0] != 0xE9 {
        return Err("not an ESP app image".into());
    }
    if bytes[HASH_APPENDED] != 1 {
        return Err("no SHA-256 appended".into());
    }
    let (body, end) = bytes.split_at(bytes.len() - 32);
    let sha256: Sha256 = end.try_into().unwrap();
    if sha2::Sha256::digest(body)[..] != sha256 {
        return Err("its SHA-256 doesn't match".into());
    }
    let magic = u32::from_le_bytes(bytes[APP_DESC..APP_DESC + 4].try_into().unwrap());
    if magic != APP_DESC_MAGIC {
        return Err("no app description".into());
    }
    let version = &bytes[VERSION..VERSION + 32];
    let version = version.split(|byte| *byte == 0).next().unwrap_or_default();
    Ok(Image {
        version: String::from_utf8_lossy(version).into_owned(),
        sha256,
        bytes,
    })
}

/// The first 8 hex digits: enough to tell images apart in a log
pub fn short_hex(sha: &Sha256) -> String {
    sha[..4]
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect()
}
