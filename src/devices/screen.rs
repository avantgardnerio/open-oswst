//! 128×64 SSD1306 OLED: puts the app's frames on the panel.
//!
//! Drawing and the latest-frame-wins handoff live in the core crate's
//! `devices::screen`. A dedicated thread here takes each frame and pushes it
//! over I2C (~90ms at 100kHz), so callers never wait on the bus.

use esp_idf_svc::hal::gpio::{AnyIOPin, PinDriver};
use esp_idf_svc::hal::i2c::config::Config as I2cConfig;
use esp_idf_svc::hal::i2c::{I2cDriver, I2C0};
use esp_idf_svc::hal::task::block_on;
use open_oswst_core::devices::screen::{next_to_show, shown};
use ssd1306::prelude::*;
use ssd1306::{I2CDisplayInterface, Ssd1306};
use std::thread;
use std::time::Duration;

// The frame type and handle the app draws with; this driver shows them
pub use open_oswst_core::devices::screen::{Frame, Screen, HEIGHT, WIDTH};

pub struct Peripherals {
    pub i2c: I2C0<'static>,
    pub sda: AnyIOPin<'static>,
    pub scl: AnyIOPin<'static>,
    pub rst: AnyIOPin<'static>,
}

/// Spawn the screen thread. It resets and initializes the panel, then flushes
/// frames as they're shown.
pub fn init(p: Peripherals) -> Screen {
    let screen = Screen::new();

    thread::Builder::new()
        .name("screen".into())
        .stack_size(8192)
        .spawn(move || screen_thread(p))
        .unwrap();

    screen
}

fn screen_thread(p: Peripherals) {
    // Reset OLED. rst stays in scope for the thread's lifetime so the pin
    // doesn't float low (holding the OLED in reset)
    let mut rst = PinDriver::output(p.rst).unwrap();
    rst.set_low().unwrap();
    thread::sleep(Duration::from_millis(50));
    rst.set_high().unwrap();
    thread::sleep(Duration::from_millis(50));

    let i2c = I2cDriver::new(p.i2c, p.sda, p.scl, &I2cConfig::default()).unwrap();
    let mut oled = Ssd1306::new(
        I2CDisplayInterface::new(i2c),
        DisplaySize128x64,
        DisplayRotation::Rotate0,
    );
    oled.init().unwrap();
    oled.set_brightness(Brightness::BRIGHTEST).unwrap();
    log::info!("OLED initialized");

    loop {
        let frame = block_on(next_to_show());
        // Reset the address pointer each frame so a short write can't skew the next one
        oled.set_draw_area((0, 0), (WIDTH as u8, HEIGHT as u8))
            .unwrap();
        oled.draw(frame.bytes()).unwrap();
        shown(frame);
    }
}
