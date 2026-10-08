//! Saved settings that survive a reboot and a reflash: a config file on the
//! board's storage. Values are text (`mode = "repeater"`), so the file reads
//! and edits by hand. A key `flags.x` is `x` in the file's [flags] table.
//! What the settings are is config.rs.

pub trait Settings {
    fn get(&self, key: &str) -> Option<String>;

    /// Best effort: a failed save is logged, not returned.
    fn set(&mut self, key: &str, value: &str);

    /// Every setting in the file, as `get` takes them
    fn keys(&self) -> Vec<String>;

    /// The saved WiFi networks' names, in the order they're tried
    fn wifi_ssids(&self) -> Vec<String>;

    /// Save a WiFi network, last in the order; one saved already keeps its
    /// place and gets the new password. Best effort, like `set`
    fn add_wifi(&mut self, ssid: &str, password: &str);

    /// Drop a saved WiFi network. Best effort, like `set`
    fn forget_wifi(&mut self, ssid: &str);
}
