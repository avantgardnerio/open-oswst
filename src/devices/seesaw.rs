//! The volume knob on rev 9: an Adafruit I2C QT Rotary Encoder (an
//! ATtiny running Adafruit's seesaw firmware) on its own I2C bus. Hands out
//! turn, press and release events, like the wired encoder (encoder.rs) it
//! replaces; knows nothing about what they mean.
//!
//! Polled, not interrupt-driven: the pin its INT line would need went to the
//! SD card. A thread reads the knob every 20 ms and queues what changed, so
//! the I2C reads never block the app; `next()` just waits on the queue.
//!
//! Each poll reads two registers:
//! - ENCODER POSITION: the knob's running total of detents, never cleared.
//!   Turns are the difference from the last poll, so none are lost between a
//!   read and a clear.
//! - GPIO BULK: the switch on seesaw pin 24, low while pressed (its pull-up
//!   is switched on at start). Polling every 20 ms filters most bounce.
//!
//! seesaw reads are a write of [module, register], a pause while the ATtiny
//! fetches the value, then a read: not a repeated-start write_read. The two
//! pauses are the poll interval: each is one FreeRTOS tick (10 ms), the
//! shortest sleep that really sleeps. Under a tick, ESP-IDF's usleep (so
//! thread::sleep) spins the CPU instead (newlib/time.c): 1 ms pauses here
//! cost ~7% of core 0.
//!
//! Register map: docs/Adafruit_seesaw/Adafruit_seesaw.h (values big-endian);
//! docs/Adafruit_seesaw/encoder_basic.ino is Adafruit's own use of it.

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use esp_idf_svc::hal::delay::TickType;
use esp_idf_svc::hal::gpio::AnyIOPin;
use esp_idf_svc::hal::i2c::{I2cConfig, I2cDriver, I2C1};
use esp_idf_svc::hal::units::Hertz;
use std::thread;
use std::time::Duration;

pub use open_oswst_core::devices::knob::Event;

const ADDRESS: u8 = 0x36;

// Modules, then their registers
const STATUS: u8 = 0x00;
const HW_ID: u8 = 0x01;
const GPIO: u8 = 0x01;
const DIRCLR_BULK: u8 = 0x03;
const BULK: u8 = 0x04;
const BULK_SET: u8 = 0x05;
const PULLENSET: u8 = 0x0B;
const ENCODER: u8 = 0x11;
const POSITION: u8 = 0x30;

/// The knob's switch: seesaw pin 24, as a bit of the 32-bit GPIO registers
const SWITCH: u32 = 1 << 24;

/// Between the register write and the read: the ATtiny needs 250 us
/// (Adafruit_seesaw.h's default read delay); one tick is the shortest real
/// sleep. Two per poll: a poll every 20 ms, fast enough that a quick click
/// (~100 ms down) is always seen pressed
const READ_DELAY: Duration = Duration::from_millis(10);
/// After a failed read: the knob unplugged, or not powered yet
const RETRY: Duration = Duration::from_secs(1);
/// 400 kHz: a poll's two reads take ~0.4 ms of the bus
const BUS_HZ: u32 = 400_000;

/// Events waiting for the app. Full (nobody listening, or a wild spin), the
/// newest are dropped
static EVENTS: Channel<CriticalSectionRawMutex, Event, 16> = Channel::new();

pub struct Peripherals {
    pub i2c: I2C1<'static>,
    pub sda: AnyIOPin<'static>,
    pub scl: AnyIOPin<'static>,
}

pub struct Seesaw;

/// Start the polling thread. Ve must already be on (board::take()).
pub fn init(p: Peripherals) -> Seesaw {
    let config = I2cConfig::new().baudrate(Hertz(BUS_HZ));
    let bus = I2cDriver::new(p.i2c, p.sda, p.scl, &config).unwrap();
    crate::thread::spawn(c"knob", 3072, None, None, move || run(bus));
    Seesaw
}

impl Seesaw {
    /// Wait for the next turn (one detent), press or release. Cancel-safe:
    /// an event stays queued until a call takes it.
    pub async fn next(&mut self) -> Event {
        EVENTS.receive().await
    }
}

impl open_oswst_core::devices::knob::Knob for Seesaw {
    async fn next(&mut self) -> Event {
        Seesaw::next(self).await
    }
}

fn run(mut bus: I2cDriver<'static>) {
    // (position, pressed) at the last good read; None until the knob answers
    let mut last: Option<(i32, bool)> = None;
    loop {
        if last.is_none() {
            if let Err(err) = start(&mut bus) {
                log::debug!("Knob: no seesaw at 0x{ADDRESS:02X} ({err})");
                thread::sleep(RETRY);
                continue;
            }
        }
        let (position, pressed) = match poll(&mut bus) {
            Ok(state) => state,
            Err(err) => {
                log::warn!("Knob: read failed ({err}), retrying");
                last = None;
                thread::sleep(RETRY);
                continue;
            }
        };

        if let Some((last_position, last_pressed)) = last {
            let turns = position.wrapping_sub(last_position);
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
        last = Some((position, pressed));
    }
}

/// Say who answered, and switch on the switch's pull-up (the seesaw resets
/// it at power-up)
fn start(bus: &mut I2cDriver<'static>) -> Result<(), esp_idf_svc::sys::EspError> {
    let mut id = [0u8];
    read(bus, STATUS, HW_ID, &mut id)?;
    let mask = SWITCH.to_be_bytes();
    for register in [DIRCLR_BULK, PULLENSET, BULK_SET] {
        let [b0, b1, b2, b3] = mask;
        bus.write(ADDRESS, &[GPIO, register, b0, b1, b2, b3], timeout())?;
    }
    log::info!("Knob: seesaw at 0x{ADDRESS:02X}, chip id 0x{:02X}", id[0]);
    Ok(())
}

/// (position, pressed)
fn poll(bus: &mut I2cDriver<'static>) -> Result<(i32, bool), esp_idf_svc::sys::EspError> {
    let mut position = [0u8; 4];
    read(bus, ENCODER, POSITION, &mut position)?;
    let mut pins = [0u8; 4];
    read(bus, GPIO, BULK, &mut pins)?;
    let pressed = u32::from_be_bytes(pins) & SWITCH == 0;
    Ok((i32::from_be_bytes(position), pressed))
}

fn read(
    bus: &mut I2cDriver<'static>,
    module: u8,
    register: u8,
    buffer: &mut [u8],
) -> Result<(), esp_idf_svc::sys::EspError> {
    bus.write(ADDRESS, &[module, register], timeout())?;
    thread::sleep(READ_DELAY);
    bus.read(ADDRESS, buffer, timeout())
}

fn timeout() -> u32 {
    TickType::new_millis(10).ticks()
}

fn queue(event: Event) {
    if EVENTS.try_send(event).is_err() {
        log::warn!("Knob: queue full, dropped {event:?}");
    }
}
