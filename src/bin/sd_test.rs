//! microSD bringup: an SD/TF breakout in SDMMC 1-bit mode, on rev 9's
//! pins (#44).
//!
//! Wiring (pin 1 = GND at the USB-C end):
//! - 3.3V -> J2-3/4 (Ve, switched on below), GND -> J2-1
//! - CLK  -> GPIO45 (J3-6). WARNING: strapping pin: a pull-up on it at boot selects
//!   1.8 V flash. The breakout's pull-ups go to Ve, which is off at reset
//! - CMD  -> GPIO43 (J2-6, UART0 TX until the console moved to USB only)
//! - D0   -> GPIO44 (J2-5, UART0 RX until then)
//! - D3   -> Ve: high at the first command keeps the card in SD mode
//!   (low would switch it to SPI mode)
//! - D1, D2, CD unconnected
//!
//! Steps, each logged:
//! 1. Talk to the card: its name and size come from its own registers, so
//!    this proves the wiring without any filesystem.
//! 2. Mount its FAT filesystem at /sdcard. A new card comes formatted FAT32;
//!    exFAT (most cards over 32GB) won't mount.
//! 3. Write HELLO.TXT, read it back, compare, list the root directory.
//! 4. Write speed: 1 MiB to SPEED.BIN in chunks of 512 B, 4 KiB and 32 KiB.
//!    Once with an fsync per chunk (the logger's pattern: every chunk safe on
//!    the card), once with one fsync at the end (raw throughput). Logs MB/s
//!    and the average and worst time per chunk.
//! 5. Read speed: SPEED.BIN read back in 32 KiB chunks, every byte checked
//!    against the pattern written, so a bus too fast for the board shows up
//!    as a mismatch, not just as a slow number.
//!
//! All of it runs once per bus clock in BUS_KHZ, card re-initialised each
//! time, to find how fast the real board's traces can go.
//!
//! Build & flash: cargo build --bin sd_test && espflash flash -p <PORT> --bootloader target/xtensa-esp32s3-espidf/debug/build/esp-idf-sys-*/out/build/bootloader/bootloader.bin --partition-table target/xtensa-esp32s3-espidf/debug/partition-table.bin target/xtensa-esp32s3-espidf/debug/sd_test

use esp_idf_svc::fs::fatfs::Fatfs;
use esp_idf_svc::hal::gpio::{AnyIOPin, PinDriver};
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
const SPEED_PATH: &str = "/sdcard/SPEED.BIN";

/// Bytes written per speed run
const SPEED_TOTAL: usize = 1024 * 1024;
const SPEED_CHUNKS: [usize; 3] = [512, 4 * 1024, 32 * 1024];

/// SD bus clocks to try, slowest first. ESP-IDF's default is 20 MHz; 40 MHz
/// is SD high speed (the card has to support it)
const BUS_KHZ: [u32; 4] = [5_000, 10_000, 20_000, 40_000];

/// Read speed chunk size: the same as the last write's chunk (see
/// read_speed)
const READ_CHUNK: usize = 32 * 1024;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let p = Peripherals::take().unwrap();

    // Ve also powers the speaker amp: with its I2S lines floating it plays
    // whatever they pick up, loud. Held low, it has no clock and stays silent
    let mut amp_din = PinDriver::output(p.pins.gpio33).unwrap();
    let mut amp_bclk = PinDriver::output(p.pins.gpio47).unwrap();
    let mut amp_lrc = PinDriver::output(p.pins.gpio48).unwrap();
    amp_din.set_low().unwrap();
    amp_bclk.set_low().unwrap();
    amp_lrc.set_low().unwrap();

    // Ve powers the card (GPIO36 LOW = on). Kept alive to the end
    let mut vext = PinDriver::output(p.pins.gpio36).unwrap();
    vext.set_low().unwrap();
    thread::sleep(Duration::from_millis(100));

    let mut slot = p.sdmmc1;
    let mut cmd = p.pins.gpio43;
    let mut clk = p.pins.gpio45;
    let mut d0 = p.pins.gpio44;
    for bus_khz in BUS_KHZ {
        log::info!("=== Bus {bus_khz} kHz ===");
        // SAFETY: the previous speed's card and host driver were dropped at
        // the end of its run, so nothing else holds the slot or the pins
        let host = unsafe {
            SdMmcHostDriver::new_1bit(
                slot.reborrow(),
                cmd.reborrow(),
                clk.reborrow(),
                d0.reborrow(),
                None::<AnyIOPin>,
                None::<AnyIOPin>,
                &SdMmcHostConfiguration::new(),
            )
        }
        .unwrap();
        test_card(host, bus_khz);
    }
    log::info!("Done");

    idle();
}

/// One speed run on a freshly initialised host: card info, mount, a small
/// write and read back, then the write and read speed tests. The card and
/// the mount drop at the end, freeing the host for the next speed.
fn test_card(host: SdMmcHostDriver<'_>, bus_khz: u32) {
    // 1. Talk to the card
    let mut card_config = SdCardConfiguration::new();
    card_config.speed_khz = bus_khz;
    let card = match SdCardDriver::new_mmc(host, &card_config) {
        Ok(card) => card,
        Err(err) => {
            log::error!("No card: {err} (check wiring, D3 high, card seated)");
            return;
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
            return;
        }
    };
    log::info!("Mounted at /sdcard");

    // 3. Write, read back, compare
    let message = format!("Hello from open-oswst, {} ms after boot\n", boot_ms());
    let started = Instant::now();
    if let Err(err) = fs::File::create(PATH).and_then(|mut file| file.write_all(message.as_bytes()))
    {
        log::error!("Write failed: {err}");
        return;
    }
    log::info!("Wrote {} B in {:?}", message.len(), started.elapsed());

    let mut read_back = String::new();
    if let Err(err) = fs::File::open(PATH).and_then(|mut file| file.read_to_string(&mut read_back))
    {
        log::error!("Read failed: {err}");
        return;
    }
    log::info!("Read back: {:?}", read_back);
    if read_back == message {
        log::info!("PASS: read matches write");
    } else {
        log::error!("FAIL: read doesn't match write");
    }

    // 4. Write speed
    for chunk in SPEED_CHUNKS {
        for fsync_each in [true, false] {
            if let Err(err) = write_speed(chunk, fsync_each) {
                log::error!("{chunk:>5} B chunks, fsync each {fsync_each}: FAILED: {err}");
            }
        }
    }

    // 5. Read speed, every byte checked
    if let Err(err) = read_speed() {
        log::error!("Read: FAILED: {err}");
    }
    let _ = fs::remove_file(SPEED_PATH);
}

/// Writes SPEED_TOTAL bytes in `chunk`-byte writes to a fresh file and logs
/// the throughput and the time per chunk. With `fsync_each`, every chunk is
/// flushed to the card (data and FAT) before the next one.
fn write_speed(chunk: usize, fsync_each: bool) -> std::io::Result<()> {
    let data: Vec<u8> = (0..chunk).map(pattern).collect();
    let mut file = fs::File::create(SPEED_PATH)?;

    let mut worst = Duration::ZERO;
    let started = Instant::now();
    for _ in 0..SPEED_TOTAL / chunk {
        let chunk_started = Instant::now();
        file.write_all(&data)?;
        if fsync_each {
            file.sync_all()?;
        }
        worst = worst.max(chunk_started.elapsed());
    }
    if !fsync_each {
        file.sync_all()?;
    }
    let total = started.elapsed();

    let chunks = (SPEED_TOTAL / chunk) as u32;
    log::info!(
        "{:>5} B chunks, fsync {}: {:.2} MB/s | per chunk avg {:?}, worst {:?}",
        chunk,
        if fsync_each { "each" } else { "at end" },
        SPEED_TOTAL as f64 / total.as_secs_f64() / 1e6,
        total / chunks,
        worst
    );
    Ok(())
}

/// Reads SPEED_PATH back in READ_CHUNK pieces, checks every byte against
/// the pattern, and logs the throughput and the number of bad bytes
fn read_speed() -> std::io::Result<()> {
    let mut buffer = vec![0u8; READ_CHUNK];
    let mut file = fs::File::open(SPEED_PATH)?;

    let mut bad_bytes = 0usize;
    let mut offset = 0usize;
    let mut worst = Duration::ZERO;
    // Only the reads are timed, not the checking
    let mut total = Duration::ZERO;
    while offset < SPEED_TOTAL {
        let chunk_started = Instant::now();
        file.read_exact(&mut buffer)?;
        let chunk_time = chunk_started.elapsed();
        worst = worst.max(chunk_time);
        total += chunk_time;
        // The last write used 32 KiB chunks, the same as READ_CHUNK, so
        // every chunk read holds the pattern from index 0
        bad_bytes += buffer
            .iter()
            .enumerate()
            .filter(|&(index, &byte)| byte != pattern(index))
            .count();
        offset += READ_CHUNK;
    }

    log::info!(
        "Read {} KiB in {} B chunks: {:.2} MB/s | worst chunk {:?} | {}",
        SPEED_TOTAL / 1024,
        READ_CHUNK,
        SPEED_TOTAL as f64 / total.as_secs_f64() / 1e6,
        worst,
        if bad_bytes == 0 {
            "PASS: all bytes match".to_string()
        } else {
            format!("FAIL: {bad_bytes} bad bytes")
        }
    );
    Ok(())
}

/// Test data by index within a write chunk: varies byte to byte and repeats
/// every 251 B (a prime, so it never lines up with 512 B sectors), so a
/// slipped or flipped bit shows up
fn pattern(index: usize) -> u8 {
    (index % 251) as u8
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
