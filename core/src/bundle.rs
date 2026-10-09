//! An update bundle: what the management server publishes, and radios
//! install (issue #34). One .tar.gz (scripts/publish-firmware.sh) holding
//! the firmware and files for the storage:
//!
//! ```text
//! firmware.bin     the app image -> the spare app slot
//! www/app.js       any other file -> the storage, at that path (/data/www/app.js)
//! ```
//!
//! A radio installs one in three steps: download it to the storage, check
//! its SHA-256, then extract it. Files overwrite what's there, except the
//! radio's own: its settings and its secrets are never touched.

/// The app image's name in a bundle
pub const FIRMWARE: &str = "firmware.bin";

/// The bundle's name on the management server, in its folder
pub const FILE_NAME: &str = "bundle.tar.gz";

/// The radio's own files, never overwritten by a bundle: its settings
/// (devices/settings.rs) and its keys (devices/secrets.rs)
pub const NEVER_EXTRACTED: &[&str] = &["config.toml", "secrets.toml"];

/// Where a bundle's file goes, relative to the storage: None for one to
/// leave alone. Its own settings and secrets, and any path that would climb
/// out of the storage ("..", or from the root)
pub fn extract_to(path: &str) -> Option<&str> {
    let path = path.strip_prefix("./").unwrap_or(path);
    let escapes = path.starts_with('/') || path.split('/').any(|part| part == "..");
    if path.is_empty() || escapes || NEVER_EXTRACTED.contains(&path) {
        return None;
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_land_at_their_path() {
        assert_eq!(extract_to("www/app.js"), Some("www/app.js"));
        assert_eq!(extract_to("./www/index.html"), Some("www/index.html"));
    }

    #[test]
    fn the_radios_own_files_are_left_alone() {
        assert_eq!(extract_to("config.toml"), None);
        assert_eq!(extract_to("./secrets.toml"), None);
        // Only the real ones: the same name elsewhere is just a file
        assert_eq!(extract_to("www/config.toml"), Some("www/config.toml"));
    }

    #[test]
    fn nothing_climbs_out_of_the_storage() {
        assert_eq!(extract_to("../sdcard/x"), None);
        assert_eq!(extract_to("www/../../x"), None);
        assert_eq!(extract_to("/data/x"), None);
        assert_eq!(extract_to(""), None);
    }
}
