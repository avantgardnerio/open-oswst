//! Encoder bringup test — reads the VOL rotary encoder on GPIO1/2/3 and
//! shows live A/B/SW state plus a quadrature counter and click count on the OLED.
//!
//! Wiring (VOL JST on the board):
//!   A   -> GPIO3
//!   B   -> GPIO2
//!   SW  -> GPIO1
//!   GND -> encoder common + button common
//!
//! NOTE: GPIO1 is also the Heltec V4's VBAT_Read pin (onboard battery divider,
//! gated by ADC_Ctrl/GPIO37) and GPIO2 is FEM_EN. This test exists to find out
//! whether either fights the encoder — if so, that's a rev5 design fix.
//!
//! Build & flash: cargo build --bin encoder_test && espflash flash -p <PORT> --partition-table target/xtensa-esp32s3-espidf/debug/partition-table.bin target/xtensa-esp32s3-espidf/debug/encoder_test

use embedded_graphics::mono_font::ascii::FONT_9X18;
use embedded_graphics::mono_font::MonoTextStyleBuilder;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use esp_idf_svc::hal::gpio::{PinDriver, Pull};
use esp_idf_svc::hal::i2c::config::Config as I2cConfig;
use esp_idf_svc::hal::i2c::I2cDriver;
use esp_idf_svc::hal::peripherals::Peripherals;
use ssd1306::prelude::*;
use ssd1306::{I2CDisplayInterface, Ssd1306};
use std::thread;
use std::time::{Duration, Instant};

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("Encoder test starting (VOL on GPIO3/2/1)");

    let p = Peripherals::take().unwrap();

    // Vext power on (GPIO36 LOW) so the OLED comes up.
    let mut vext = PinDriver::output(p.pins.gpio36).unwrap();
    vext.set_low().unwrap();
    thread::sleep(Duration::from_millis(50));

    // OLED reset (GPIO21).
    let mut oled_rst = PinDriver::output(p.pins.gpio21).unwrap();
    oled_rst.set_low().unwrap();
    thread::sleep(Duration::from_millis(50));
    oled_rst.set_high().unwrap();
    thread::sleep(Duration::from_millis(50));

    let i2c = I2cDriver::new(p.i2c0, p.pins.gpio17, p.pins.gpio18, &I2cConfig::default()).unwrap();
    let mut display = Ssd1306::new(
        I2CDisplayInterface::new(i2c),
        DisplaySize128x64,
        DisplayRotation::Rotate0,
    )
    .into_buffered_graphics_mode();
    display.init().unwrap();
    display.set_brightness(Brightness::BRIGHTEST).unwrap();

    let style = MonoTextStyleBuilder::new()
        .font(&FONT_9X18)
        .text_color(BinaryColor::On)
        .build();

    // VOL encoder inputs with internal pull-ups (common ties to GND).
    let a = PinDriver::input(p.pins.gpio3, Pull::Up).unwrap();
    let b = PinDriver::input(p.pins.gpio2, Pull::Up).unwrap();
    let sw = PinDriver::input(p.pins.gpio1, Pull::Up).unwrap();

    let _oled_rst = oled_rst;

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

            display.clear_buffer();
            Text::new(&line1, Point::new(2, 16), style)
                .draw(&mut display)
                .unwrap();
            Text::new(&line2, Point::new(2, 36), style)
                .draw(&mut display)
                .unwrap();
            Text::new(&line3, Point::new(2, 56), style)
                .draw(&mut display)
                .unwrap();
            display.flush().unwrap();
            last_flush = Instant::now();
        }

        thread::sleep(Duration::from_millis(1));
    }
}
