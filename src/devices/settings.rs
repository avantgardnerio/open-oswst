//! Saved settings: namespace `config` in the dedicated `open-oswst` NVS
//! partition (see partitions.csv), so they survive a reboot and a reflash.

use esp_idf_svc::nvs::{EspCustomNvsPartition, EspNvs, NvsCustom};

pub struct Settings(EspNvs<NvsCustom>);

/// Opened read-write so the menu can save; that also creates the namespace on
/// fresh boards. None (logged) if the partition can't be opened.
pub fn init() -> Option<Settings> {
    let partition = EspCustomNvsPartition::take("open-oswst").ok()?;
    EspNvs::new(partition, "config", true)
        .map(Settings)
        .map_err(|e| log::warn!("NVS config unavailable ({}), settings won't persist", e))
        .ok()
}

impl open_oswst_core::devices::settings::Settings for Settings {
    fn get_u8(&self, key: &str) -> Option<u8> {
        self.0.get_u8(key).ok().flatten()
    }

    fn set_u8(&mut self, key: &str, value: u8) {
        if let Err(e) = self.0.set_u8(key, value) {
            log::warn!("NVS save {}={} failed: {}", key, value, e);
        }
    }
}
