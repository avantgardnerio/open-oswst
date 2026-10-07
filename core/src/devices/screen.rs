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

/// A copy of a frame's pixels, kept after the frame goes back to the pool:
/// the screen as it was, for a screenshot.
pub struct Snapshot(Box<[u8; FRAME_BYTES]>);

impl Default for Snapshot {
    fn default() -> Self {
        Snapshot(Box::new([0; FRAME_BYTES]))
    }
}

impl Snapshot {
    /// Take the frame's pixels, in place: no allocation per copy
    pub fn copy(&mut self, frame: &Frame) {
        self.0.copy_from_slice(&frame.0[..]);
    }

    pub fn pixel(&self, x: usize, y: usize) -> bool {
        self.0[(y / 8) * WIDTH + x] & (1 << (y % 8)) != 0
    }

    /// As a 1-bit BMP file, 1086 bytes: lit pixels white on black, as the
    /// panel shows them. BMP stores rows bottom-up, each pixel one bit with
    /// the leftmost in the top bit; 128 pixels make a 16-byte row, already
    /// the 4-byte multiple BMP rows must be.
    pub fn bmp(&self) -> Vec<u8> {
        const HEADERS: u32 = 14 + 40 + 8; // file header, info header, 2-colour palette
        const ROW_BYTES: usize = WIDTH / 8;
        let image_bytes = (ROW_BYTES * HEIGHT) as u32;

        let mut bmp = Vec::with_capacity((HEADERS + image_bytes) as usize);
        // File header
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&(HEADERS + image_bytes).to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes()); // reserved
        bmp.extend_from_slice(&HEADERS.to_le_bytes()); // where the pixels start
                                                       // Info header
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&(WIDTH as i32).to_le_bytes());
        bmp.extend_from_slice(&(HEIGHT as i32).to_le_bytes()); // positive: bottom-up
        bmp.extend_from_slice(&1u16.to_le_bytes()); // planes
        bmp.extend_from_slice(&1u16.to_le_bytes()); // bits per pixel
        bmp.extend_from_slice(&0u32.to_le_bytes()); // no compression
        bmp.extend_from_slice(&image_bytes.to_le_bytes());
        bmp.extend_from_slice(&2835i32.to_le_bytes()); // 72 dpi across
        bmp.extend_from_slice(&2835i32.to_le_bytes()); // and down
        bmp.extend_from_slice(&2u32.to_le_bytes()); // colours in the palette
        bmp.extend_from_slice(&0u32.to_le_bytes()); // all of them matter
                                                    // Palette, blue-green-red-unused: 0 = off = black, 1 = lit = white
        bmp.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        bmp.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0x00]);
        // Pixels
        for y in (0..HEIGHT).rev() {
            for byte_x in 0..ROW_BYTES {
                let mut byte = 0u8;
                for bit in 0..8 {
                    if self.pixel(byte_x * 8 + bit, y) {
                        byte |= 0x80 >> bit;
                    }
                }
                bmp.push(byte);
            }
        }
        bmp
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snapshot_is_a_1bit_bmp_of_the_frame() {
        let mut frame = Frame::new();
        frame.set_pixel(0, 0, true); // top left
        frame.set_pixel(127, 63, true); // bottom right
        let mut snapshot = Snapshot::default();
        snapshot.copy(&frame);

        let bmp = snapshot.bmp();
        assert_eq!(bmp.len(), 1086);
        assert_eq!(&bmp[0..2], b"BM");
        assert_eq!(u32::from_le_bytes(bmp[2..6].try_into().unwrap()), 1086);
        assert_eq!(u32::from_le_bytes(bmp[10..14].try_into().unwrap()), 62);
        let pixels = &bmp[62..];
        // Bottom-up: the first row stored is the screen's bottom one
        assert_eq!(pixels[15], 0x01); // bottom right, the row's last bit
        assert_eq!(pixels[16 * 63], 0x80); // top left, the last row's first bit
        assert_eq!(pixels.iter().filter(|&&byte| byte != 0).count(), 2);
    }
}
