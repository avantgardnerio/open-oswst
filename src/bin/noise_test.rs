//! Bringup test for noise.rs, Noise on ESP-IDF's mbedTLS: each primitive
//! against its published test vector (RFC 7748 X25519, FIPS 180-2 SHA-256,
//! NIST GCM case 13), then a whole Noise_XK_25519_AESGCM_SHA256 handshake
//! between two ends on the board and a message each way. 2026-10-08 on
//! F000: all three ok; handshake 1117 ms for both ends (~110 ms per X25519,
//! in software: no curve hardware), ~2.3 KB heap at the points sampled.
//! Build & flash: cargo build --bin noise_test && espflash flash -p <PORT> --bootloader <ours> --partition-table target/xtensa-esp32s3-espidf/debug/partition-table.bin target/xtensa-esp32s3-espidf/debug/noise_test

use std::thread;
use std::time::Duration;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    open_oswst::noise::self_test();
    loop {
        thread::sleep(Duration::from_secs(1));
    }
}
