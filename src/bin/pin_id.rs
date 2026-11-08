//! Pin identifier for bench wiring: each candidate pin gets its own slow,
//! steady pattern, so a meter (or the Jumperless ADC) on the far end of a
//! jumper says which GPIO it reaches.
//!   GPIO47 (J2 pin 13): always HIGH
//!   GPIO48 (J2 pin 14): always LOW
//!   GPIO33 (J2 pin 12): toggles every 500 ms
//!   GPIO34 (J2 pin 11): toggles every 2 s
//! GPIO26 (J2 pin 15) is the PSRAM's chip select: never touched.

use esp_idf_svc::hal::gpio::PinDriver;
use esp_idf_svc::hal::peripherals::Peripherals;
use std::thread;
use std::time::Duration;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    let p = Peripherals::take().unwrap();

    let mut gpio47 = PinDriver::output(p.pins.gpio47).unwrap();
    let mut gpio48 = PinDriver::output(p.pins.gpio48).unwrap();
    let mut gpio33 = PinDriver::output(p.pins.gpio33).unwrap();
    let mut gpio34 = PinDriver::output(p.pins.gpio34).unwrap();
    gpio47.set_high().unwrap();
    gpio48.set_low().unwrap();
    log::info!("pin_id: 47 HIGH, 48 LOW, 33 toggles 500 ms, 34 toggles 2 s");

    let mut ticks: u32 = 0;
    loop {
        gpio33.toggle().unwrap();
        if ticks % 4 == 0 {
            gpio34.toggle().unwrap();
        }
        ticks += 1;
        thread::sleep(Duration::from_millis(500));
    }
}
