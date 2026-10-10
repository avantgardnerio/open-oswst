//! The volume knob on the next rev: a SparkFun Qwiic Twist on its own I2C
//! bus (#44). Hands out turn, press and release events, like the wired
//! encoder (encoder.rs) it replaces; knows nothing about what they mean.
//!
//! Polled, not interrupt-driven: the pin its INT line would need went to the
//! SD card. A thread reads the knob every POLL and queues what changed, so
//! the I2C reads never block the app; `next()` just waits on the queue.
//!
//! Only reads, never writes, once running. From one 6-byte read (STATUS up
//! to COUNT):
//! - COUNT: the knob's running total of detents, never cleared. Turns are
//!   the difference from the last poll, so none are lost between a read and
//!   a clear.
//! - STATUS bit 1 ("pressed"): the Twist flips it on every edge of the
//!   switch, so while nobody clears it, it says whether the button is down.
//!   The Twist doesn't debounce; polling every POLL filters most bounce.
//!
//! Register map: docs/SparkFun_Qwiic_Twist_Arduino_Library.h; the Twist's own
//! firmware is in docs/Qwiic_Twist_firmware/.

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use esp_idf_svc::hal::delay::TickType;
use esp_idf_svc::hal::gpio::AnyIOPin;
use esp_idf_svc::hal::i2c::{I2cConfig, I2cDriver, I2C1};
use esp_idf_svc::hal::units::Hertz;
use std::thread;
use std::time::Duration;

pub use open_oswst_core::devices::knob::Event;

const ADDRESS: u8 = 0x3F;

// Registers
const STATUS: u8 = 0x01;
const RED: u8 = 0x0D; // then GREEN, BLUE

// STATUS bits
const PRESSED: u8 = 1 << 1;

/// Fast enough that a quick click (~100 ms down) is always seen pressed
const POLL: Duration = Duration::from_millis(20);
/// After a failed read: the knob unplugged, or not powered yet
const RETRY: Duration = Duration::from_secs(1);
/// 400 kHz: a poll's read takes ~0.2 ms of the bus
const BUS_HZ: u32 = 400_000;

/// Events waiting for the app. Full (nobody listening, or a wild spin), the
/// newest are dropped
static EVENTS: Channel<CriticalSectionRawMutex, Event, 16> = Channel::new();

pub struct Peripherals {
    pub i2c: I2C1<'static>,
    pub sda: AnyIOPin<'static>,
    pub scl: AnyIOPin<'static>,
}

pub struct Twist;

/// Start the polling thread. Ve must already be on (board::take()).
pub fn init(p: Peripherals) -> Twist {
    let config = I2cConfig::new().baudrate(Hertz(BUS_HZ));
    let bus = I2cDriver::new(p.i2c, p.sda, p.scl, &config).unwrap();
    crate::thread::spawn(c"knob", 3072, None, None, move || run(bus));
    Twist
}

impl Twist {
    /// Wait for the next turn (one detent), press or release. Cancel-safe:
    /// an event stays queued until a call takes it.
    pub async fn next(&mut self) -> Event {
        EVENTS.receive().await
    }
}

impl open_oswst_core::devices::knob::Knob for Twist {
    async fn next(&mut self) -> Event {
        Twist::next(self).await
    }
}

fn run(mut bus: I2cDriver<'static>) {
    let timeout = TickType::new_millis(10).ticks();
    // (count, pressed) at the last good read; None until the knob answers
    let mut last: Option<(i16, bool)> = None;
    loop {
        // STATUS, VERSION (2), ENABLE_INTS, COUNT (2, little-endian)
        let mut registers = [0u8; 6];
        if let Err(err) = bus.write_read(ADDRESS, &[STATUS], &mut registers, timeout) {
            if last.is_some() {
                log::warn!("Knob: read failed ({err}), retrying");
            }
            last = None;
            thread::sleep(RETRY);
            continue;
        }
        let pressed = registers[0] & PRESSED != 0;
        let count = i16::from_le_bytes([registers[4], registers[5]]);

        match last {
            None => {
                // The LED stays dark: a lit knob is seen at night. It keeps
                // its colour across power cycles, and starts white
                if bus.write(ADDRESS, &[RED, 0, 0, 0], timeout).is_err() {
                    thread::sleep(RETRY);
                    continue;
                }
                log::info!("Knob: Qwiic Twist at 0x{ADDRESS:02X}, LED off");
            }
            Some((last_count, last_pressed)) => {
                let turns = count.wrapping_sub(last_count);
                let turn = if turns > 0 { Event::Cw } else { Event::Ccw };
                for _ in 0..turns.unsigned_abs() {
                    queue(turn);
                }
                if pressed != last_pressed {
                    queue(if pressed {
                        Event::Press
                    } else {
                        Event::Release
                    });
                }
            }
        }
        last = Some((count, pressed));
        thread::sleep(POLL);
    }
}

fn queue(event: Event) {
    if EVENTS.try_send(event).is_err() {
        log::warn!("Knob: queue full, dropped {event:?}");
    }
}
