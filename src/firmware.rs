//! The app image this radio is running, and the rollback that guards new ones.
//!
//! The flash holds two app slots (ota_0, ota_1). A new app goes into the one
//! not running (`install`: POST /api/ota copies /data/firmware.bin there) and
//! the bootloader boots it next.
//! With rollback on (CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE), a new app starts
//! out pending: if the radio reboots for any reason before the app confirms
//! itself, the bootloader goes back to the previous app. The app confirms
//! itself after CONFIRM_AFTER of running, so a build that crashes or
//! boot-loops undoes itself, and a radio updated over WiFi can't be lost to a
//! bad build. (One that runs but misbehaves past a minute is on us.)

use std::io::Read as _;
use std::path::Path;
use std::time::Duration;

use esp_idf_svc::ota::EspOta;
use esp_idf_svc::sys::*;

/// How long a new app must run before it's kept
pub const CONFIRM_AFTER: Duration = Duration::from_secs(60);

/// Wait CONFIRM_AFTER on a thread of its own, then confirm the running app
/// (if it's a new one waiting to be confirmed)
pub fn confirm_later() {
    crate::thread::spawn(c"firmware", 3072, None, None, || {
        std::thread::sleep(CONFIRM_AFTER);
        if state() == "pending" {
            unsafe { esp_ota_mark_app_valid_cancel_rollback() };
            log::info!("Firmware: running {:?}, confirmed", CONFIRM_AFTER);
        }
    });
}

/// Copy an app image file (`espflash save-image`) into the spare app slot.
/// ESP-IDF checks it before marking the slot to boot, so a bad file leaves
/// the running app as it is. Returns the image's size; the caller reboots
pub fn install(image: &Path) -> anyhow::Result<usize> {
    let mut file = std::fs::File::open(image)?;
    let mut ota = EspOta::new()?;
    let mut update = ota.initiate_update()?;
    let mut buf = vec![0u8; 4096];
    let mut total = 0;
    loop {
        let n = match file.read(&mut buf) {
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
    update.complete()?;
    Ok(total)
}

/// The running app's state: "valid" (confirmed, or flashed over USB),
/// "pending" (new, not yet confirmed) or "unknown"
pub fn state() -> &'static str {
    let mut state: esp_ota_img_states_t = 0;
    let found = unsafe { esp_ota_get_state_partition(running(), &mut state) } == ESP_OK;
    if found && state == esp_ota_img_states_t_ESP_OTA_IMG_VALID {
        "valid"
    } else if found && state == esp_ota_img_states_t_ESP_OTA_IMG_PENDING_VERIFY {
        "pending"
    } else {
        "unknown"
    }
}

/// The running app image's length in bytes (less than its slot): what a
/// download of it holds
pub fn image_len() -> Option<u32> {
    let partition = unsafe { &*running() };
    let position = esp_partition_pos_t {
        offset: partition.address,
        size: partition.size,
    };
    let mut metadata: esp_image_metadata_t = Default::default();
    let ok = unsafe { esp_image_get_metadata(&position, &mut metadata) } == ESP_OK;
    ok.then_some(metadata.image_len)
}

/// Read the running app image from `offset` into `buf`
pub fn read(offset: usize, buf: &mut [u8]) -> Result<(), EspError> {
    esp!(unsafe { esp_partition_read(running(), offset, buf.as_mut_ptr().cast(), buf.len()) })
}

fn running() -> *const esp_partition_t {
    unsafe { esp_ota_get_running_partition() }
}
