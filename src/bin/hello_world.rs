//! Minimal board bringup test — prints "Hello World" on the OLED.
//! Proves Vext power, I2C, and the SSD1306 display are all wired correctly.
//! Build & flash: cargo build --bin hello_world && espflash flash -p <PORT> --partition-table target/xtensa-esp32s3-espidf/debug/partition-table.bin target/xtensa-esp32s3-espidf/debug/hello_world

use embedded_graphics::mono_font::ascii::FONT_9X18;
use embedded_graphics::mono_font::MonoTextStyleBuilder;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use open_oswst::{board, screen};
use std::thread;
use std::time::Duration;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("Hello World OLED test starting");

    let board = board::take();
    let screen = screen::init(board.screen);

    let style = MonoTextStyleBuilder::new()
        .font(&FONT_9X18)
        .text_color(BinaryColor::On)
        .build();

    let mut frame = screen.frame();
    Text::new("Hello", Point::new(8, 24), style)
        .draw(&mut frame)
        .unwrap();
    Text::new("World!", Point::new(8, 48), style)
        .draw(&mut frame)
        .unwrap();
    screen.show(frame);
    log::info!("Drawn to screen — if you can read this, the board works.");

    loop {
        thread::sleep(Duration::from_secs(1));
    }
}
