//! The SD card (#44): LittleFS mounted at /data/sd, holding everything the
//! radio writes while it runs: the logs, the web app, update downloads,
//! screenshots. After `init`, it's plain `std::fs`.
//!
//! Why a card: writing the chip's own flash turns its caches off, which
//! stalls everything, the radio included, for up to ~18 ms per write. The SD
//! card is a separate peripheral, so writing it stalls nothing. Only the
//! settings (config.toml, secrets.toml) stay on the flash (storage.rs):
//! read at boot, and written only when someone changes one.
//!
//! Mounted inside /data so WebDAV's /fs/ shows both: ESP-IDF hands a path to
//! the filesystem with the longest matching mount point, so /data/sd/... is
//! the card and the rest of /data is the flash.
//!
//! No card, no card storage: nothing is mounted, and whatever lives here
//! (logs, web app, updates) is simply missing. LittleFS rather than FAT, as
//! on the flash, because it survives a power cut mid-write. A card that
//! isn't LittleFS (a new one comes FAT) is formatted at the first boot.
//!
//! WARNING: CLK is GPIO45, a strapping pin. The card's pull-ups must not
//! hold it high at reset (that selects 1.8 V flash): they're on Ve, which
//! is off until board::take() switches it on.

use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};
use core::time::Duration;
use esp_idf_svc::hal::gpio::AnyIOPin;
use esp_idf_svc::hal::sd::mmc::{SdMmcHostConfiguration, SdMmcHostDriver, SDMMC1};
use esp_idf_svc::hal::sd::{SdCardConfiguration, SdCardDriver};
use esp_idf_svc::sys::{
    esp, esp_littlefs_sdmmc_info, esp_vfs_littlefs_conf_t, esp_vfs_littlefs_register, sdmmc_card_t,
    EspError,
};

/// Where the card appears in `std::fs` paths
pub const ROOT: &str = "/data/sd";

/// SD bus clock: 40 MHz, SD high speed. On the rev 9 board sd_test passed
/// every speed from 5 to 40 MHz with all bytes read back intact
/// (2026-10-19); reads ran 1.34 MB/s. Writes stay ~0.3 MB/s at any clock:
/// the card's own busy time sets them. 5 MHz is the most the breadboard's
/// jumper wires ran reliably (20 MHz failed multi-block writes there)
const BUS_KHZ: u32 = 40_000;

/// How often waiting log lines go to the card: often enough that a power
/// cut loses at most this much, rarely enough that the card isn't woken
/// for every line
const LOG_WRITE_PERIOD: Duration = Duration::from_secs(1);

pub struct Peripherals {
    pub sdmmc: SDMMC1<'static>,
    pub clk: AnyIOPin<'static>,
    pub cmd: AnyIOPin<'static>,
    pub d0: AnyIOPin<'static>,
}

/// The card, once mounted; null until then (or with no card)
static CARD: AtomicPtr<sdmmc_card_t> = AtomicPtr::new(ptr::null_mut());

pub fn mounted() -> bool {
    !CARD.load(Ordering::Relaxed).is_null()
}

/// Find the card and mount it. Ve must already be on (board::take())
pub fn init(p: Peripherals) -> Result<(), EspError> {
    // 1-bit mode: CLK, CMD, D0. D3 is tied high on the board (high at the
    // first command keeps the card in SD mode, not SPI)
    let host = SdMmcHostDriver::new_1bit(
        p.sdmmc,
        p.cmd,
        p.clk,
        p.d0,
        None::<AnyIOPin>,
        None::<AnyIOPin>,
        &SdMmcHostConfiguration::new(),
    )?;
    let mut config = SdCardConfiguration::new();
    config.speed_khz = BUS_KHZ;
    // Kept for good: LittleFS holds on to the card, and the driver owns it
    let driver: &'static SdCardDriver<SdMmcHostDriver<'static>> =
        Box::leak(Box::new(SdCardDriver::new_mmc(host, &config)?));
    let card = driver.card() as *const sdmmc_card_t as *mut sdmmc_card_t;

    let mut conf = esp_vfs_littlefs_conf_t {
        base_path: c"/data/sd".as_ptr(),
        sdcard: card,
        ..Default::default()
    };
    conf.set_format_if_mount_failed(1);
    esp!(unsafe { esp_vfs_littlefs_register(&conf) })?;
    CARD.store(card, Ordering::Relaxed);

    let (used, total) = usage();
    log::info!(
        "SD card mounted at {}: {} of {} MB used",
        ROOT,
        used / (1024 * 1024),
        total / (1024 * 1024)
    );
    Ok(())
}

/// (used, total) bytes; (0, 0) with no card
pub fn usage() -> (usize, usize) {
    let card = CARD.load(Ordering::Relaxed);
    if card.is_null() {
        return (0, 0);
    }
    let (mut total, mut used) = (0usize, 0usize);
    unsafe { esp_littlefs_sdmmc_info(card, &mut total, &mut used) };
    (used, total)
}

/// Write the log lines waiting in RAM to the card, forever: the "log"
/// thread (main.rs). A thread, not an async task, because ESP-IDF's file
/// calls block until the card is done, and its SD driver spins meanwhile
/// (30-85 ms a write): in the app's executor that would freeze the app.
/// Here it holds up nothing but itself.
pub fn write_logs() -> ! {
    loop {
        if open_oswst_core::logger::pending() {
            while open_oswst_core::logger::flush_chunk() {}
        }
        std::thread::sleep(LOG_WRITE_PERIOD);
    }
}
