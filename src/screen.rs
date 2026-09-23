//! 128×64 SSD1306 OLED. Knows about pixels, nothing else.
//!
//! Double buffered: callers draw into a [`Frame`] (any embedded-graphics
//! `DrawTarget` op works) and hand it to [`Screen::show`]. A dedicated thread
//! pushes it over I2C (~90ms at 100kHz) so the caller never blocks on the bus.

use core::convert::Infallible;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use esp_idf_svc::hal::gpio::{AnyIOPin, PinDriver};
use esp_idf_svc::hal::i2c::config::Config as I2cConfig;
use esp_idf_svc::hal::i2c::{I2cDriver, I2C0};
use esp_idf_svc::hal::task::block_on;
use ssd1306::prelude::*;
use ssd1306::{I2CDisplayInterface, Ssd1306};
use std::thread;
use std::time::Duration;

pub const WIDTH: usize = 128;
pub const HEIGHT: usize = 64;
const FRAME_BYTES: usize = WIDTH * HEIGHT / 8;

/// A 1bpp framebuffer in SSD1306 page layout: each byte is a vertical strip of
/// 8 pixels, so it goes to the panel as-is with no conversion.
pub struct Frame(Box<[u8; FRAME_BYTES]>);

impl Frame {
    fn new() -> Self {
        Frame(Box::new([0; FRAME_BYTES]))
    }

    fn set_pixel(&mut self, x: usize, y: usize, on: bool) {
        let idx = (y / 8) * WIDTH + x;
        let bit = 1 << (y % 8);
        if on {
            self.0[idx] |= bit;
        } else {
            self.0[idx] &= !bit;
        }
    }
}

impl OriginDimensions for Frame {
    fn size(&self) -> Size {
        Size::new(WIDTH as u32, HEIGHT as u32)
    }
}

impl DrawTarget for Frame {
    type Color = BinaryColor;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        let bounds = self.bounding_box();
        for Pixel(p, color) in pixels {
            if bounds.contains(p) {
                self.set_pixel(p.x as usize, p.y as usize, color.is_on());
            }
        }
        Ok(())
    }

    fn clear(&mut self, color: Self::Color) -> Result<(), Self::Error> {
        self.0.fill(if color.is_on() { 0xFF } else { 0x00 });
        Ok(())
    }
}

/// Frames circulate FREE → caller draws → READY → thread flushes → FREE.
/// Two frames total: one being drawn while the other is on the bus.
static FREE: Channel<CriticalSectionRawMutex, Frame, 2> = Channel::new();
static READY: Channel<CriticalSectionRawMutex, Frame, 2> = Channel::new();

pub struct Peripherals {
    pub i2c: I2C0<'static>,
    pub sda: AnyIOPin<'static>,
    pub scl: AnyIOPin<'static>,
    pub rst: AnyIOPin<'static>,
}

/// Handle to the screen. Only one can exist — it owns the frame pool.
pub struct Screen(());

impl Screen {
    /// Wait for a free frame. Contents are whatever was drawn last — clear it
    /// if you want a blank slate.
    pub async fn frame(&self) -> Frame {
        FREE.receive().await
    }

    /// Queue a frame for display.
    pub fn show(&self, frame: Frame) {
        // Can't fail: only 2 frames exist and READY holds 2
        let _ = READY.try_send(frame);
    }
}

/// Spawn the screen thread. It resets and initializes the panel, then flushes
/// frames as they're shown.
pub fn init(p: Peripherals) -> Screen {
    let _ = FREE.try_send(Frame::new());
    let _ = FREE.try_send(Frame::new());

    thread::Builder::new()
        .name("screen".into())
        .stack_size(8192)
        .spawn(move || screen_thread(p))
        .unwrap();

    Screen(())
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
        let frame = block_on(READY.receive());
        // Reset the address pointer each frame so a short write can't skew the next one
        oled.set_draw_area((0, 0), (WIDTH as u8, HEIGHT as u8)).unwrap();
        oled.draw(&frame.0[..]).unwrap();
        let _ = FREE.try_send(frame);
    }
}
