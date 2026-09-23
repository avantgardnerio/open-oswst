//! Minimal board bringup test — prints "Hello World" on the OLED.
//! Proves Vext power, I2C, and the SSD1306 display are all wired correctly.
//! Build & flash: cargo build --bin hello_world && espflash flash -p <PORT> --partition-table target/xtensa-esp32s3-espidf/debug/partition-table.bin target/xtensa-esp32s3-espidf/debug/hello_world

use embedded_graphics::mono_font::ascii::FONT_9X18;
use embedded_graphics::mono_font::MonoTextStyleBuilder;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use esp_idf_svc::hal::gpio::PinDriver;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::task::block_on;
use open_oswst::screen;
use std::thread;
use std::time::Duration;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("Hello World OLED test starting");

    let p = Peripherals::take().unwrap();

    // Vext power on (GPIO36 LOW) — OLED is dead without this.
    let mut vext = PinDriver::output(p.pins.gpio36).unwrap();
    vext.set_low().unwrap();
    thread::sleep(Duration::from_millis(50));
    log::info!("Vext enabled");

    // OLED: SDA=GPIO17, SCL=GPIO18, RST=GPIO21.
    let screen = screen::init(screen::Peripherals {
        i2c: p.i2c0,
        sda: p.pins.gpio17.into(),
        scl: p.pins.gpio18.into(),
        rst: p.pins.gpio21.into(),
    });

    let style = MonoTextStyleBuilder::new()
        .font(&FONT_9X18)
        .text_color(BinaryColor::On)
        .build();

    let mut frame = block_on(screen.frame());
    frame.clear(BinaryColor::Off).unwrap();
    Text::new("Hello", Point::new(8, 24), style)
        .draw(&mut frame)
        .unwrap();
    Text::new("World!", Point::new(8, 48), style)
        .draw(&mut frame)
        .unwrap();
    screen.show(frame);
    log::info!("Drawn to screen — if you can read this, the board works.");

    // Keep vext alive so the OLED stays powered.
    let _vext = vext;
    loop {
        thread::sleep(Duration::from_secs(1));
    }
}
