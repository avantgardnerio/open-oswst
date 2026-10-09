//! microSD bringup: an SD/TF breakout on J3, in SDMMC 1-bit mode.
//!
//! Wiring (J3, pin 1 = GND at the USB-C end):
//! - 3.3V -> J3-2 (3V3), GND -> J3-1
//! - CLK  -> GPIO3 (J3-14)
//! - CMD  -> GPIO6 (J3-17)
//! - D0   -> GPIO4 (J3-15)
//! - D3   -> 3V3 (J3-3): high at the first command keeps the card in SD mode
//!   (low would switch it to SPI mode)
//! - D1, D2, CD unconnected
//!
//! These are the encoder and mic pins in the radio firmware, so this is for a
//! bare breadboard board only.
//!
//! Steps, each logged:
//! 1. Talk to the card: its name and size come from its own registers, so
//!    this proves the wiring without any filesystem.
//! 2. Mount its FAT filesystem at /sdcard. A new card comes formatted FAT32;
//!    exFAT (most cards over 32GB) won't mount.
//! 3. Write HELLO.TXT, read it back, compare, list the root directory.
//!
//! Build & flash: cargo build --bin sd_test && espflash flash -p <PORT> --bootloader target/xtensa-esp32s3-espidf/debug/build/esp-idf-sys-*/out/build/bootloader/bootloader.bin --partition-table target/xtensa-esp32s3-espidf/debug/partition-table.bin target/xtensa-esp32s3-espidf/debug/sd_test

use esp_idf_svc::fs::fatfs::Fatfs;
use esp_idf_svc::hal::gpio::AnyIOPin;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::sd::mmc::{SdMmcHostConfiguration, SdMmcHostDriver};
use esp_idf_svc::hal::sd::{SdCardConfiguration, SdCardDriver};
use esp_idf_svc::io::vfs::MountedFatfs;
use std::fs;
use std::io::{Read, Write};
use std::thread;
use std::time::{Duration, Instant};

/// FAT without long-file-name support (ESP-IDF's default): 8.3 names only
const PATH: &str = "/sdcard/HELLO.TXT";

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let p = Peripherals::take().unwrap();

    // 1. Talk to the card. The S3's SDMMC host routes through the GPIO
    // matrix, so any pins work. Internal pull-ups on CMD and D0 in case the
    // breakout has none (the SD bus is open-drain during init).
    let host = SdMmcHostDriver::new_1bit(
        p.sdmmc1,
        p.pins.gpio6, // CMD
        p.pins.gpio3, // CLK
        p.pins.gpio4, // D0
        None::<AnyIOPin>,
        None::<AnyIOPin>,
        &SdMmcHostConfiguration::new(),
    )
    .unwrap();
    let card = match SdCardDriver::new_mmc(host, &SdCardConfiguration::new()) {
        Ok(card) => card,
        Err(err) => {
            log::error!("No card: {err} (check wiring, D3 high, card seated)");
            idle();
        }
    };

    let info = card.card();
    // SAFETY: a C union of the decoded and raw CID; ESP-IDF fills the decoded
    // one for SD cards
    let cid = unsafe { info.__bindgen_anon_1.cid };
    let name: String = cid
        .name
        .iter()
        .take_while(|&&byte| byte != 0)
        .map(|&byte| byte as char)
        .collect();
    let bytes = info.csd.capacity as u64 * info.csd.sector_size as u64;
    log::info!(
        "Card '{}': {} sectors x {} B = {:.2} GB, bus {} kHz",
        name,
        info.csd.capacity,
        info.csd.sector_size,
        bytes as f64 / 1e9,
        info.real_freq_khz
    );

    // 2. Mount FAT. Dropping the guard unmounts, so it lives to the end.
    let fatfs = Fatfs::new_sdcard(0, card).unwrap();
    let _mounted = match MountedFatfs::mount(fatfs, "/sdcard", 4) {
        Ok(mounted) => mounted,
        Err(err) => {
            log::error!("Mount failed: {err} (card not FAT? exFAT won't mount)");
            idle();
        }
    };
    log::info!("Mounted at /sdcard");

    // 3. Write, read back, compare
    let message = format!("Hello from open-oswst, {} ms after boot\n", boot_ms());
    let started = Instant::now();
    fs::File::create(PATH)
        .and_then(|mut file| file.write_all(message.as_bytes()))
        .unwrap();
    log::info!("Wrote {} B in {:?}", message.len(), started.elapsed());

    let mut read_back = String::new();
    fs::File::open(PATH)
        .and_then(|mut file| file.read_to_string(&mut read_back))
        .unwrap();
    log::info!("Read back: {:?}", read_back);
    if read_back == message {
        log::info!("PASS: read matches write");
    } else {
        log::error!("FAIL: read doesn't match write");
    }

    for entry in fs::read_dir("/sdcard").unwrap() {
        let entry = entry.unwrap();
        let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        log::info!("  {:?} {} B", entry.file_name(), size);
    }

    idle();
}

/// Time since boot, from ESP-IDF's microsecond timer
fn boot_ms() -> u128 {
    (unsafe { esp_idf_svc::sys::esp_timer_get_time() } / 1000) as u128
}

/// Done (or failed): stay up so the log can be read
fn idle() -> ! {
    loop {
        thread::sleep(Duration::from_secs(1));
    }
}
