//! Saved settings that survive a reboot: NVS on the board, a file on the desktop.

pub trait Settings {
    fn get_u8(&self, key: &str) -> Option<u8>;

    /// Best effort: a failed save is logged, not returned.
    fn set_u8(&mut self, key: &str, value: u8);
}
