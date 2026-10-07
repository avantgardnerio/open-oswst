//! Encoder bringup test — reads the VOL rotary encoder and shows live A/B/SW
//! state plus a quadrature counter and click count on the OLED.
//!
//! Pins from board.rs, like the app (on PCB rev `4468800`: A = GPIO3,
//! B = GPIO6, SW = GPIO45; a rev `dbc0ed0` board has SW on GPIO1). GND goes
//! to the encoder common and the button common.
//!
//! Build & flash: cargo build --bin encoder_test && espflash flash -p <PORT> --partition-table target/xtensa-esp32s3-espidf/debug/partition-table.bin target/xtensa-esp32s3-espidf/debug/encoder_test

use embedded_graphics::mono_font::ascii::FONT_9X18;
use embedded_graphics::mono_font::MonoTextStyleBuilder;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use esp_idf_svc::hal::gpio::{PinDriver, Pull};
use open_oswst::board;
use open_oswst::devices::screen;
use std::thread;
use std::time::{Duration, Instant};

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("Encoder test starting (VOL pins from board.rs)");

    let board = board::take();
    let screen = screen::init(board.screen);

    let style = MonoTextStyleBuilder::new()
        .font(&FONT_9X18)
        .text_color(BinaryColor::On)
        .build();

    // VOL encoder inputs with internal pull-ups (common ties to GND).
    let a = PinDriver::input(board.vol.a, Pull::Up).unwrap();
    let b = PinDriver::input(board.vol.b, Pull::Up).unwrap();
    let sw = PinDriver::input(board.vol.sw, Pull::Up).unwrap();

    let mut last_ab = (a.is_high(), b.is_high());
    let mut last_sw = sw.is_high();
    let mut quad_count: i32 = 0; // raw quadrature transitions (~4 per detent)
    let mut clicks: u32 = 0;
    let mut last_flush = Instant::now();

    loop {
        let cur_a = a.is_high();
        let cur_b = b.is_high();
        let cur_sw = sw.is_high();

        // Quadrature decode: CW if the new state matches the gray-code sequence
        // 00 -> 10 -> 11 -> 01 -> 00; CCW is the reverse.
        if (cur_a, cur_b) != last_ab {
            let cw = matches!(
                (last_ab, (cur_a, cur_b)),
                ((false, false), (true, false))
                    | ((true, false), (true, true))
                    | ((true, true), (false, true))
                    | ((false, true), (false, false))
            );
            let ccw = matches!(
                (last_ab, (cur_a, cur_b)),
                ((false, false), (false, true))
                    | ((false, true), (true, true))
                    | ((true, true), (true, false))
                    | ((true, false), (false, false))
            );
            if cw {
                quad_count += 1;
            } else if ccw {
                quad_count -= 1;
            }
            // Log every A/B transition so a one-way or stuck-line fault is visible
            // on serial: "??" means the pair jumped a state (bounce or a dead line).
            log::info!(
                "AB {}{} -> {}{} {} quad={}",
                last_ab.0 as u8,
                last_ab.1 as u8,
                cur_a as u8,
                cur_b as u8,
                if cw {
                    "CW "
                } else if ccw {
                    "CCW"
                } else {
                    "?? "
                },
                quad_count
            );
            last_ab = (cur_a, cur_b);
        }

        // Falling edge on SW = click.
        if last_sw && !cur_sw {
            clicks += 1;
            log::info!("click {} (total)", clicks);
        }
        last_sw = cur_sw;

        // Throttle OLED writes to ~20Hz; polling stays tight for clean reads.
        if last_flush.elapsed() >= Duration::from_millis(50) {
            let detents = quad_count / 4;
            let line1 = format!(
                "A{} B{} SW{}",
                if cur_a { 1 } else { 0 },
                if cur_b { 1 } else { 0 },
                if cur_sw { 1 } else { 0 },
            );
            let line2 = format!("rot {:>4} ({})", detents, quad_count);
            let line3 = format!("clk {}", clicks);

            let mut frame = screen.frame();
            Text::new(&line1, Point::new(2, 16), style)
                .draw(&mut frame)
                .unwrap();
            Text::new(&line2, Point::new(2, 36), style)
                .draw(&mut frame)
                .unwrap();
            Text::new(&line3, Point::new(2, 56), style)
                .draw(&mut frame)
                .unwrap();
            screen.show(frame);
            last_flush = Instant::now();
        }

        thread::sleep(Duration::from_millis(1));
    }
}
