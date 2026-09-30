//! GPS bringup: powers the L76K on the Heltec's GNSS connector and prints its
//! NMEA sentences. Pins per Meshtastic's heltec_v4 variant.h.
//!
//! Look for $GNRMC / $GNGGA: fix status, lat/lon, UTC time. A cold start
//! outdoors takes ~30s+ for a fix; indoors it may only report time or nothing.
//!
//! Build & flash: cargo build --bin gps_test && espflash flash -p <PORT> --partition-table target/xtensa-esp32s3-espidf/debug/partition-table.bin target/xtensa-esp32s3-espidf/debug/gps_test

use esp_idf_svc::hal::delay::TickType;
use esp_idf_svc::hal::gpio::{AnyIOPin, PinDriver};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::uart::{config::Config, UartDriver};
use esp_idf_svc::hal::units::Hertz;
use std::thread;
use std::time::Duration;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let p = Peripherals::take().unwrap();

    // GPS power (VGNSS_Ctrl), active LOW
    let mut power = PinDriver::output(p.pins.gpio34).unwrap();
    power.set_low().unwrap();
    // Out of reset (active LOW), and forced awake
    let mut reset = PinDriver::output(p.pins.gpio42).unwrap();
    reset.set_high().unwrap();
    let mut wake = PinDriver::output(p.pins.gpio40).unwrap();
    wake.set_high().unwrap();
    thread::sleep(Duration::from_millis(100));

    // ESP TX -> GPS RX on GPIO38, GPS TX -> ESP RX on GPIO39
    let uart = UartDriver::new(
        p.uart1,
        p.pins.gpio38,
        p.pins.gpio39,
        Option::<AnyIOPin>::None,
        Option::<AnyIOPin>::None,
        &Config::new().baudrate(Hertz(9600)),
    )
    .unwrap();
    log::info!("GPS powered, reading NMEA at 9600 baud");

    let mut line = Vec::new();
    let mut buf = [0u8; 128];
    let mut silent_secs = 0;
    loop {
        let n = uart
            .read(&mut buf, TickType::new_millis(1000).ticks())
            .unwrap_or(0);
        if n == 0 {
            silent_secs += 1;
            log::warn!("No data from GPS for {}s", silent_secs);
            continue;
        }
        silent_secs = 0;
        for &b in &buf[..n] {
            if b == b'\n' {
                log::info!("{}", String::from_utf8_lossy(&line).trim_end());
                line.clear();
            } else {
                line.push(b);
            }
        }
    }
}
