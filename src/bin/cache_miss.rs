//! Flash cache miss bench: how long does one miss take, and how bad is the
//! tail? Our code and constants run from flash through a cache shared by
//! both cores (I-cache 16KB, D-cache 32KB, 32-byte lines, flash DIO 80MHz),
//! so every miss stalls the CPU while the line is fetched over SPI.
//!
//! Reads one byte from random 32-byte lines of a 256KB table in flash: far
//! bigger than the cache, so nearly every read misses. Each read is timed
//! with the CPU cycle counter. Data misses take the same path as code misses
//! (same flash, same controller), so the numbers stand for both.
//!
//! Alternates two loads, forever:
//!   - alone: nothing else touching flash
//!   - codec: Codec2 encode + decode on the other core, as fast as it can.
//!     Both cores share the cache and the flash bus, so its misses queue
//!     with ours
//!
//! Hits are measured the same way, for the timer's own overhead.
//!
//! Build & flash: cargo build --bin cache_miss && espflash flash -p <PORT> --partition-table target/xtensa-esp32s3-espidf/debug/partition-table.bin target/xtensa-esp32s3-espidf/debug/cache_miss

use esp_idf_svc::hal::cpu::Core;
use esp_idf_svc::hal::task::block_on;
use esp_idf_svc::hal::task::thread::ThreadSpawnConfiguration;
use open_oswst_core::codec::{self, CodecRequest, CODEC2_FRAME_SAMPLES, DECODED, ENCODED};
use open_oswst_core::codec::{FRAMES_PER_PACKET, PAYLOAD_BYTES};
use std::sync::atomic::{AtomicBool, Ordering};

/// In flash: an immutable static with no interior mutability is rodata
static TABLE: [u8; TABLE_BYTES] = [0x5A; TABLE_BYTES];
const TABLE_BYTES: usize = 256 * 1024;
const LINE_BYTES: usize = 32;
/// Reads per run
const SAMPLES: usize = 5_000;
const CYCLES_PER_US: u32 = 240;

/// Whether the codec thread runs flat out
static CODEC_BUSY: AtomicBool = AtomicBool::new(false);

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("cache_miss starting");

    // The codec, flat out on core 0 while CODEC_BUSY is set
    let (codec_tx, codec_rx) = std::sync::mpsc::sync_channel::<CodecRequest>(2);
    spawn_on(Core::Core0, 5, 32768, move || codec::run(codec_rx));
    spawn_on(Core::Core0, 5, 8192, move || feed_codec(codec_tx));

    // The measuring thread, alone on core 1 at a high priority
    spawn_on(Core::Core1, 10, 16384, measure);

    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

fn spawn_on(core: Core, priority: u8, stack: usize, f: impl FnOnce() + Send + 'static) {
    ThreadSpawnConfiguration {
        priority,
        pin_to_core: Some(core),
        ..Default::default()
    }
    .set()
    .unwrap();
    std::thread::Builder::new()
        .stack_size(stack)
        .spawn(f)
        .unwrap();
    ThreadSpawnConfiguration::default().set().unwrap();
}

/// Encode + decode back to back, with no pause, while CODEC_BUSY is set.
fn feed_codec(codec_tx: std::sync::mpsc::SyncSender<CodecRequest>) {
    let pcm: Vec<i16> = (0..FRAMES_PER_PACKET * CODEC2_FRAME_SAMPLES)
        .map(|i| ((i * 7919) % 8000) as i16 - 4000)
        .collect();
    loop {
        if CODEC_BUSY.load(Ordering::Relaxed) {
            codec_tx
                .send(CodecRequest::encode([0; 2], pcm.clone().into()))
                .unwrap();
            block_on(ENCODED.receive());
            codec_tx
                .send(CodecRequest::decode(0, 0, [0x55; PAYLOAD_BYTES]))
                .unwrap();
            block_on(DECODED.receive());
        } else {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

fn measure() {
    // Allocated once: nothing on the measured path touches the heap
    let mut misses = vec![0u32; SAMPLES];
    let mut hits = vec![0u32; SAMPLES];
    loop {
        for codec_busy in [false, true] {
            CODEC_BUSY.store(codec_busy, Ordering::Relaxed);
            std::thread::sleep(std::time::Duration::from_millis(300)); // let it get going
            time_reads(&mut misses, &mut hits);
            let load = if codec_busy {
                "codec on core 0"
            } else {
                "alone"
            };
            report(load, "miss", &mut misses);
            report(load, "hit ", &mut hits);
        }
    }
}

/// Time SAMPLES reads of random lines (misses), each followed by a second
/// read of the same line (a hit), in CPU cycles.
fn time_reads(misses: &mut [u32], hits: &mut [u32]) {
    let lines = (TABLE_BYTES / LINE_BYTES) as u32;
    let mut rng = 0x1234_5678u32;
    for i in 0..SAMPLES {
        // A random line, so no prefetcher can guess the next one
        rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let line = (rng >> 8) % lines;
        let ptr = &TABLE[line as usize * LINE_BYTES] as *const u8;

        let start = cycles();
        unsafe { core::ptr::read_volatile(ptr) };
        let missed = cycles();
        unsafe { core::ptr::read_volatile(ptr) };
        let hit = cycles();

        misses[i] = missed.wrapping_sub(start);
        hits[i] = hit.wrapping_sub(missed);
    }
}

fn cycles() -> u32 {
    unsafe { esp_idf_svc::sys::xthal_get_ccount() }
}

fn report(load: &str, what: &str, samples: &mut [u32]) {
    samples.sort_unstable();
    let at = |fraction: f64| samples[((samples.len() - 1) as f64 * fraction) as usize];
    let us = |c: u32| c as f64 / CYCLES_PER_US as f64;
    let over_10us = samples.iter().filter(|&&c| c > 10 * CYCLES_PER_US).count();
    log::info!(
        "{:16} {}: min {:.2}us  p50 {:.2}  p90 {:.2}  p99 {:.2}  p99.9 {:.2}  max {:.2}us  (>10us: {} of {})",
        load,
        what,
        us(samples[0]),
        us(at(0.5)),
        us(at(0.9)),
        us(at(0.99)),
        us(at(0.999)),
        us(samples[samples.len() - 1]),
        over_10us,
        samples.len()
    );
}
