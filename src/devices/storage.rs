//! Flash storage: the `storage` partition (see partitions.csv) as a LittleFS
//! filesystem mounted at /data. After `init`, it's plain `std::fs`.
//!
//! LittleFS rather than FAT because it survives a power cut mid-write, and a
//! battery radio gets switched off whenever its owner feels like it.

use core::sync::atomic::{AtomicBool, Ordering};
use esp_idf_svc::sys::{
    esp, esp_littlefs_info, esp_vfs_littlefs_conf_t, esp_vfs_littlefs_register, EspError,
};

/// Where the filesystem appears in `std::fs` paths
pub const ROOT: &str = "/data";

const LABEL: &core::ffi::CStr = c"storage";

/// Set once the filesystem is mounted
static MOUNTED: AtomicBool = AtomicBool::new(false);

pub fn mounted() -> bool {
    MOUNTED.load(Ordering::Relaxed)
}

pub fn init() -> Result<(), EspError> {
    let mut conf = esp_vfs_littlefs_conf_t {
        base_path: c"/data".as_ptr(),
        partition_label: LABEL.as_ptr(),
        ..Default::default()
    };
    conf.set_format_if_mount_failed(1); // a fresh board's partition is blank
    esp!(unsafe { esp_vfs_littlefs_register(&conf) })?;
    MOUNTED.store(true, Ordering::Relaxed);

    let (used, total) = usage();
    log::info!(
        "Storage mounted at {}: {} of {} KB used",
        ROOT,
        used / 1024,
        total / 1024
    );
    Ok(())
}

/// (used, total) bytes
pub fn usage() -> (usize, usize) {
    let (mut total, mut used) = (0usize, 0usize);
    unsafe { esp_littlefs_info(LABEL.as_ptr(), &mut total, &mut used) };
    (used, total)
}
