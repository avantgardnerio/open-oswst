//! Knob bringup: a SparkFun Qwiic Twist on the I2C bus that replaces the VOL
//! encoder's A/B/SW wires (#44). Logs every turn and click.
//!
//! Wiring (rev 9's knob bus; the Twist's own 2.2k pull-ups, none added):
//! - GND  -> J2-1
//! - 3.3V -> J2-3/4 (Ve, switched on below)
//! - SDA  -> GPIO3 (J3-14)
//! - SCL  -> GPIO6 (J3-17)
//! - INT, RST unconnected: the knob is polled
//!
//! Steps, each logged:
//! 1. Scan the bus: every address that answers. The Twist is 0x3F.
//! 2. Read its ID and firmware version, and light it dim blue: proof that
//!    writes reach it too.
//! 3. Poll every POLL_MS: the status register says whether the knob moved or
//!    was clicked since the last read. Turns log the difference (detents) and
//!    the running count; I2C errors are counted, so a flaky breadboard wire
//!    shows up as a number rather than a guess.
//!
//! Register map: docs/SparkFun_Qwiic_Twist_Arduino_Library.h (16-bit values
//! are little-endian; status bits are cleared by writing them back as 0).
//!
//! Build & flash: cargo build --bin knob_test && espflash flash -p <PORT> --bootloader target/xtensa-esp32s3-espidf/debug/build/esp-idf-sys-*/out/build/bootloader/bootloader.bin --partition-table target/xtensa-esp32s3-espidf/debug/partition-table.bin target/xtensa-esp32s3-espidf/debug/knob_test

use esp_idf_svc::hal::delay::TickType;
use esp_idf_svc::hal::gpio::PinDriver;
use esp_idf_svc::hal::i2c::{I2cConfig, I2cDriver};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::units::Hertz;
use std::thread;
use std::time::Duration;

const TWIST: u8 = 0x3F;

// Registers
const ID: u8 = 0x00;
const STATUS: u8 = 0x01;
const VERSION: u8 = 0x02;
const COUNT: u8 = 0x05;
const DIFFERENCE: u8 = 0x07;
const RED: u8 = 0x0D;

// STATUS bits
const MOVED: u8 = 1 << 0;
const CLICKED: u8 = 1 << 2;

/// 100 kHz: standard mode, forgiving of breadboard wires
const BUS_HZ: u32 = 100_000;
const POLL_MS: u64 = 20;

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

    // Ve powers the knob (GPIO36 LOW = on). Kept alive to the end
    let mut vext = PinDriver::output(p.pins.gpio36).unwrap();
    vext.set_low().unwrap();
    thread::sleep(Duration::from_millis(100));

    // I2C1: I2C0 is the OLED's, on GPIO17/18
    let config = I2cConfig::new().baudrate(Hertz(BUS_HZ));
    let mut bus = I2cDriver::new(p.i2c1, p.pins.gpio3, p.pins.gpio6, &config).unwrap();
    let timeout = TickType::new_millis(50).ticks();

    // 1. Scan
    let found: Vec<u8> = (0x08..0x78)
        .filter(|&address| bus.write(address, &[], timeout).is_ok())
        .collect();
    log::info!("I2C scan (SDA GPIO3, SCL GPIO6): {:02X?}", found);
    if !found.contains(&TWIST) {
        log::error!("No Twist at 0x{TWIST:02X} (check Ve, SDA/SCL swapped?)");
        idle();
    }

    // 2. Who it is, and a write it can show
    let mut id = [0u8; 1];
    let mut version = [0u8; 2];
    bus.write_read(TWIST, &[ID], &mut id, timeout).unwrap();
    bus.write_read(TWIST, &[VERSION], &mut version, timeout)
        .unwrap();
    log::info!(
        "Twist: ID 0x{:02X}, firmware {}.{}",
        id[0],
        version[1],
        version[0]
    );
    // Red, green, blue in three consecutive registers
    bus.write(TWIST, &[RED, 0, 0, 32], timeout).unwrap();

    // 3. Poll
    let mut errors: u32 = 0;
    let mut clicks: u32 = 0;
    loop {
        thread::sleep(Duration::from_millis(POLL_MS));

        let mut status = [0u8; 1];
        if bus
            .write_read(TWIST, &[STATUS], &mut status, timeout)
            .is_err()
        {
            errors += 1;
            log::warn!("I2C error reading status ({errors} so far)");
            continue;
        }
        let status = status[0];
        if status & (MOVED | CLICKED) == 0 {
            continue;
        }

        if status & MOVED != 0 {
            let mut difference = [0u8; 2];
            let mut count = [0u8; 2];
            let read = bus
                .write_read(TWIST, &[DIFFERENCE], &mut difference, timeout)
                .and_then(|_| bus.write(TWIST, &[DIFFERENCE, 0, 0], timeout))
                .and_then(|_| bus.write_read(TWIST, &[COUNT], &mut count, timeout));
            match read {
                Ok(()) => log::info!(
                    "turn {:+} -> count {}",
                    i16::from_le_bytes(difference),
                    i16::from_le_bytes(count)
                ),
                Err(err) => {
                    errors += 1;
                    log::warn!("I2C error reading the turn: {err} ({errors} so far)");
                }
            }
        }
        if status & CLICKED != 0 {
            clicks += 1;
            log::info!("click {clicks}");
        }

        // Clear the bits just handled
        if bus
            .write(TWIST, &[STATUS, status & !(MOVED | CLICKED)], timeout)
            .is_err()
        {
            errors += 1;
            log::warn!("I2C error clearing status ({errors} so far)");
        }
    }
}

/// Failed: stay up so the log can be read
fn idle() -> ! {
    loop {
        thread::sleep(Duration::from_secs(1));
    }
}
