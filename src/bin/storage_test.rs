//! LittleFS bringup: mounts /data, keeps a boot counter, and measures how long
//! flash writes stall the chip, since that's the risk for the logger.
//!
//! A 1ms periodic timer records the longest gap between its callbacks, while
//! the main thread appends 1KB chunks (about one logger batch) with an fsync
//! each. Gaps well over 1ms are the stall.
//!
//! First run (2026-09-30, Boya flash): idle gap 1.1ms, worst stall 18.5ms,
//! write+fsync avg 13.6ms / worst 87ms (the writer waits; others run).
//!
//! Needs the new partition table (factory 4MB + storage), flashed as usual:
//! Build & flash: cargo build --bin storage_test && espflash flash -p <PORT> --partition-table target/xtensa-esp32s3-espidf/debug/partition-table.bin target/xtensa-esp32s3-espidf/debug/storage_test

use esp_idf_svc::timer::EspTaskTimerService;
use open_oswst::devices::storage;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

/// Longest gap between timer callbacks, in µs
static MAX_GAP_US: AtomicU32 = AtomicU32::new(0);

const CHUNK: usize = 1024;
const CHUNKS: usize = 128; // 128KB: enough to force many block erases

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    storage::init().unwrap();

    // Boot counter: proves files survive a reset
    let counter = format!("{}/boots.txt", storage::ROOT);
    let boots: u32 = fs::read_to_string(&counter)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
        + 1;
    fs::write(&counter, boots.to_string()).unwrap();
    log::info!("Boot number {}", boots);

    // Timer callbacks run from flash, so they stop during a flash stall too
    let timers = EspTaskTimerService::new().unwrap();
    let mut last = Instant::now();
    let watchdog = timers
        .timer(move || {
            let now = Instant::now();
            let gap = now.duration_since(last).as_micros() as u32;
            MAX_GAP_US.fetch_max(gap, Ordering::Relaxed);
            last = now;
        })
        .unwrap();
    watchdog.every(Duration::from_millis(1)).unwrap();
    thread::sleep(Duration::from_millis(200));
    let idle_gap = MAX_GAP_US.swap(0, Ordering::Relaxed);
    log::info!("Watchdog idle: max gap {} us", idle_gap);

    // Write test
    let path = format!("{}/stall_test.txt", storage::ROOT);
    let _ = fs::remove_file(&path);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .unwrap();
    let line = [b'x'; CHUNK];
    let (mut worst_write, mut total) = (Duration::ZERO, Duration::ZERO);
    for _ in 0..CHUNKS {
        let started = Instant::now();
        file.write_all(&line).unwrap();
        file.sync_all().unwrap();
        let took = started.elapsed();
        worst_write = worst_write.max(took);
        total += took;
    }
    let write_gap = MAX_GAP_US.swap(0, Ordering::Relaxed);
    log::info!(
        "Wrote {} x {}B: avg {} us, worst {} us per write+fsync",
        CHUNKS,
        CHUNK,
        total.as_micros() / CHUNKS as u128,
        worst_write.as_micros()
    );
    log::info!(
        "Watchdog during writes: max gap {} us  <-- the stall",
        write_gap
    );

    let (used, total) = storage::usage();
    log::info!("Storage: {} of {} KB used", used / 1024, total / 1024);

    loop {
        thread::sleep(Duration::from_secs(10));
    }
}
