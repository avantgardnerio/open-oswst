//! Mic test — shows live waveform + VU meter on OLED.
//! Build & flash: cargo build --bin mic_test && espflash flash -p /dev/ttyACM1 --partition-table target/xtensa-esp32s3-espidf/debug/partition-table.bin target/xtensa-esp32s3-espidf/debug/mic_test

use embedded_graphics::mono_font::ascii::FONT_6X10;
use embedded_graphics::mono_font::MonoTextStyleBuilder;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Line, PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use esp_idf_svc::hal::adc::continuous::config::Config as AdcContConfig;
use esp_idf_svc::hal::adc::continuous::{AdcDriver as AdcContDriver, AdcMeasurement, Attenuated};
use esp_idf_svc::hal::gpio::PinDriver;
use esp_idf_svc::hal::i2c::config::Config as I2cConfig;
use esp_idf_svc::hal::i2c::I2cDriver;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::units::Hertz;
use ssd1306::prelude::*;
use ssd1306::{I2CDisplayInterface, Ssd1306};
use std::fmt::Write as FmtWrite;
use std::thread;
use std::time::{Duration, Instant};

/// Display width = number of waveform samples to show
const WAVE_W: usize = 128;
/// Waveform area height (top portion of display)
const WAVE_H: i32 = 40;
/// VU bar Y position
const VU_Y: i32 = 54;
const VU_H: i32 = 10;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("Mic test starting...");

    let p = Peripherals::take().unwrap();

    // Vext power on (GPIO36 LOW)
    let mut vext = PinDriver::output(p.pins.gpio36).unwrap();
    vext.set_low().unwrap();
    thread::sleep(Duration::from_millis(50));

    // ADC continuous for mic on GPIO4 (ADC1_CH3) at 8kHz — matches the PCB's MIC_OUT net.
    // (The original radio was wired to GPIO7; the PCB moved MIC_OUT to GPIO4.)
    let adc_cfg = AdcContConfig::new()
        .sample_freq(Hertz(8000))
        .frame_measurements(320)
        .frames_count(2);
    let mut adc = AdcContDriver::new(p.adc1, &adc_cfg, Attenuated::db12(p.pins.gpio4)).unwrap();

    // Reset OLED
    let mut oled_rst = PinDriver::output(p.pins.gpio21).unwrap();
    oled_rst.set_low().unwrap();
    thread::sleep(Duration::from_millis(50));
    oled_rst.set_high().unwrap();
    thread::sleep(Duration::from_millis(50));

    // I2C for OLED
    let i2c = I2cDriver::new(
        p.i2c0,
        p.pins.gpio17, // SDA
        p.pins.gpio18, // SCL
        &I2cConfig::default(),
    )
    .unwrap();

    let interface = I2CDisplayInterface::new(i2c);
    let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();
    display.init().unwrap();
    display.set_brightness(Brightness::BRIGHTEST).unwrap();
    log::info!("OLED initialized");

    let style = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(BinaryColor::On)
        .build();
    let line_style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let fill_style = PrimitiveStyle::with_fill(BinaryColor::On);

    let mut adc_buf = vec![AdcMeasurement::new(); 320];
    let mut wave = [2048u16; WAVE_W]; // ring buffer of raw 12-bit samples
    let mut wave_pos: usize = 0;
    let mut label_buf = heapless::String::<32>::new();

    let mut last_log = Instant::now();

    adc.start().unwrap();
    log::info!("ADC started on GPIO4 — showing waveform + VU");

    loop {
        // Read a batch of ADC samples
        let count = adc.read(&mut adc_buf, 100).unwrap_or(0);
        if count == 0 {
            thread::sleep(Duration::from_millis(5));
            continue;
        }

        // Feed into ring buffer + compute peak
        let mut peak: i32 = 0;
        let mut raw_min = u16::MAX;
        let mut raw_max = 0u16;
        for sample in &adc_buf[..count] {
            let raw = sample.data(); // 12-bit, 0-4095
            wave[wave_pos] = raw;
            wave_pos = (wave_pos + 1) % WAVE_W;

            if raw < raw_min {
                raw_min = raw;
            }
            if raw > raw_max {
                raw_max = raw;
            }

            let centered = (raw as i32) - 2048;
            let abs = centered.abs();
            if abs > peak {
                peak = abs;
            }
        }

        // Throttled serial log. raw min/max matter more than `peak` here: the MAX9814 biases
        // its output near 1.25V (~1550 counts), not mid-rail, so `peak` reads ~500 at silence.
        // A live mic shows min/max spreading apart when you talk; a dead one stays pinned.
        if last_log.elapsed() >= Duration::from_millis(250) {
            log::info!(
                "raw min={} max={} spread={} peak={} n={}",
                raw_min,
                raw_max,
                raw_max.saturating_sub(raw_min),
                peak,
                count
            );
            last_log = Instant::now();
        }

        // Draw
        display.clear_buffer();

        // --- Waveform (oscilloscope) ---
        // Draw from wave_pos (oldest) to wave_pos-1 (newest)
        for x in 0..(WAVE_W - 1) {
            let idx0 = (wave_pos + x) % WAVE_W;
            let idx1 = (wave_pos + x + 1) % WAVE_W;
            // Map 0-4095 to 0..WAVE_H (inverted: high value = top)
            let y0 = WAVE_H - 1 - (wave[idx0] as i32 * (WAVE_H - 1) / 4095);
            let y1 = WAVE_H - 1 - (wave[idx1] as i32 * (WAVE_H - 1) / 4095);
            Line::new(Point::new(x as i32, y0), Point::new(x as i32 + 1, y1))
                .into_styled(line_style)
                .draw(&mut display)
                .unwrap();
        }

        // Center line (DC midpoint)
        Line::new(Point::new(0, WAVE_H / 2), Point::new(127, WAVE_H / 2))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut display)
            .unwrap();

        // --- VU bar ---
        // peak is 0-2048, map to 0-128 pixels
        let vu_width = (peak * 128 / 2048).min(128) as u32;
        if vu_width > 0 {
            Rectangle::new(Point::new(0, VU_Y), Size::new(vu_width, VU_H as u32))
                .into_styled(fill_style)
                .draw(&mut display)
                .unwrap();
        }

        // Peak label (between waveform and VU bar)
        label_buf.clear();
        let _ = core::write!(label_buf, "pk:{}", peak);
        Text::new(&label_buf, Point::new(0, WAVE_H + 10), style)
            .draw(&mut display)
            .unwrap();

        display.flush().unwrap();
    }
}
