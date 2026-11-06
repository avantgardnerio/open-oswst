//! Mic stream test: sends every mic sample over the USB serial port, so a host script can record it
//! to a WAV with no speaker and no WiFi. Uses the radio's own mic driver (8 kHz continuous ADC on
//! GPIO4), so this is the exact sample path the firmware encodes.
//!
//! Always streaming, PTT or not. One line per 40 ms frame:
//!   PCM <frame number> <320 x 4 hex digits>
//! Each sample is the driver's i16 PCM with bit 0 replaced by the PTT level (1 = pressed). The
//! driver's PCM is the 12-bit ADC shifted left by 4, so bit 0 carries no audio.
//!
//! The host sends cues for the person at the bench, one line each: "CUE <text>", where '|' starts
//! a new line on the OLED (3 lines of 12 characters fit). Host side: scripts/mic_stream.py.

use embedded_graphics::mono_font::ascii::FONT_10X20;
use embedded_graphics::mono_font::{MonoTextStyle, MonoTextStyleBuilder};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use esp_idf_svc::hal::gpio::Pull;
use esp_idf_svc::hal::task::block_on;
use esp_idf_svc::sys::{
    esp, esp_vfs_usb_serial_jtag_use_driver, usb_serial_jtag_driver_config_t,
    usb_serial_jtag_driver_install, usb_serial_jtag_read_bytes,
};
use open_oswst::board;
use open_oswst::devices::irq_pin::IrqPin;
use open_oswst::devices::mic::{self, FRAME_SAMPLES};
use open_oswst::devices::screen::{self, Screen};
use std::fmt::Write;

/// Lines of the 10x20 font that fit on the 64-pixel-high OLED
const CUE_LINES: usize = 3;

fn show_cue(screen: &Screen, style: MonoTextStyle<'_, BinaryColor>, cue: &str) {
    let mut frame = screen.frame();
    for (line_number, text) in cue.split('|').take(CUE_LINES).enumerate() {
        let baseline = 16 + 21 * line_number as i32;
        let _ = Text::new(text, Point::new(0, baseline), style).draw(&mut frame);
    }
    screen.show(frame);
}

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("Mic stream test starting...");

    // The console's default USB driver can only write. The interrupt driver adds a receive buffer
    // for the host's cues; output moves onto it too, so the two never interleave
    let mut usb_config = usb_serial_jtag_driver_config_t {
        tx_buffer_size: 4096,
        rx_buffer_size: 512,
    };
    unsafe {
        esp!(usb_serial_jtag_driver_install(&mut usb_config)).unwrap();
        esp_vfs_usb_serial_jtag_use_driver();
    }

    let board = board::take();
    // PTT is the PRG button on GPIO0, with the K1 3.5 sleeve wired in parallel
    let button = IrqPin::new(board.ptt.into(), Pull::Up);
    let mut mic = mic::init(board.mic);
    let screen = screen::init(board.screen);
    let style = MonoTextStyleBuilder::new()
        .font(&FONT_10X20)
        .text_color(BinaryColor::On)
        .build();
    show_cue(&screen, style, "WAIT");

    let mut pcm = [0i16; FRAME_SAMPLES];
    let mut line = String::with_capacity(16 + FRAME_SAMPLES * 4);
    let mut received = [0u8; 64];
    let mut command = Vec::with_capacity(64);

    block_on(async {
        mic.start();
        log::info!("Streaming {} samples per frame at 8 kHz", FRAME_SAMPLES);

        let mut frame_number: u32 = 0;
        loop {
            mic.read(&mut pcm).await;
            let ptt_bit = button.is_low() as u16;

            line.clear();
            let _ = write!(line, "PCM {} ", frame_number);
            for sample in &pcm {
                let _ = write!(line, "{:04x}", (*sample as u16 & !1) | ptt_bit);
            }
            println!("{}", line);
            frame_number = frame_number.wrapping_add(1);

            // Cues from the host: whatever arrived since the last frame, without waiting
            let count = unsafe {
                usb_serial_jtag_read_bytes(received.as_mut_ptr().cast(), received.len() as u32, 0)
            };
            for &byte in &received[..count.max(0) as usize] {
                if byte != b'\n' {
                    command.push(byte);
                    continue;
                }
                let text = String::from_utf8_lossy(&command);
                if let Some(cue) = text.trim().strip_prefix("CUE ") {
                    log::info!("Cue: {}", cue);
                    show_cue(&screen, style, cue);
                }
                command.clear();
            }
        }
    });
}
