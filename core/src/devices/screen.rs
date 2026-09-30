//! The 128×64 screen. Knows about pixels, nothing else.
//!
//! Double buffered, latest frame wins: callers draw into a [`Frame`] (any
//! embedded-graphics `DrawTarget` op works) and hand it to [`Screen::show`].
//! The driver (the OLED thread on the board, a window on the desktop) takes
//! frames with [`next_to_show`] and gives them back with [`shown`]. Callers
//! never block: a frame shown while the driver is busy replaces any still
//! pending one.

use core::convert::Infallible;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use std::thread;

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

    /// Raw bytes in SSD1306 page layout
    pub fn bytes(&self) -> &[u8] {
        &self.0[..]
    }

    pub fn pixel(&self, x: usize, y: usize) -> bool {
        self.0[(y / 8) * WIDTH + x] & (1 << (y % 8)) != 0
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

/// Frames circulate FREE → caller draws → READY → driver shows → FREE.
/// Two frames total. The driver holds at most one, so as long as the caller
/// holds at most one too, the other is always in FREE or READY.
static FREE: Channel<CriticalSectionRawMutex, Frame, 2> = Channel::new();
static READY: Channel<CriticalSectionRawMutex, Frame, 1> = Channel::new();

/// Handle to the screen. Only one can exist — it owns the frame pool.
pub struct Screen(());

impl Screen {
    /// Set up the frame pool. Call once, before starting the driver.
    // Not Default: it fills the one global frame pool, so it isn't a casual value
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let _ = FREE.try_send(Frame::new());
        let _ = FREE.try_send(Frame::new());
        Screen(())
    }

    /// Get a blank frame to draw into. Never blocks: if a frame is still
    /// waiting to be shown, it's reclaimed — it would be overwritten anyway.
    /// Hold at most one frame at a time.
    pub fn frame(&self) -> Frame {
        loop {
            // Both can be momentarily empty while the driver swaps frames
            if let Ok(mut frame) = READY.try_receive().or_else(|_| FREE.try_receive()) {
                let _ = frame.clear(BinaryColor::Off);
                return frame;
            }
            thread::yield_now();
        }
    }

    /// Display a frame, replacing any frame still waiting to be shown.
    pub fn show(&self, frame: Frame) {
        if let Ok(stale) = READY.try_receive() {
            let _ = FREE.try_send(stale);
        }
        let _ = READY.try_send(frame);
    }
}

/// Driver side: wait for the next frame to put on the display.
pub async fn next_to_show() -> Frame {
    READY.receive().await
}

/// Driver side: done with a frame, back to the pool.
pub fn shown(frame: Frame) {
    let _ = FREE.try_send(frame);
}
