//! Saved settings that survive a reboot and a reflash: a config file on the
//! board's storage. Values are text (`mode = "repeater"`), so the file reads
//! and edits by hand.

pub trait Settings {
    fn get(&self, key: &str) -> Option<String>;

    /// Best effort: a failed save is logged, not returned.
    fn set(&mut self, key: &str, value: &str);
}
