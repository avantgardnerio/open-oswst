//! Raw audio loopback test (no Codec2) — hold PTT to record, release to play back raw PCM.
//! Build & flash: cargo build --bin raw_loopback && espflash flash -p /dev/ttyACM1 --partition-table target/xtensa-esp32s3-espidf/debug/partition-table.bin target/xtensa-esp32s3-espidf/debug/raw_loopback

use embedded_graphics::mono_font::ascii::FONT_10X20;
use embedded_graphics::mono_font::MonoTextStyleBuilder;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use esp_idf_svc::hal::adc::continuous::config::Config as AdcContConfig;
use esp_idf_svc::hal::adc::continuous::{AdcDriver as AdcContDriver, AdcMeasurement, Attenuated};
use esp_idf_svc::hal::delay::BLOCK;
use esp_idf_svc::hal::gpio::{AnyIOPin, PinDriver, Pull};
use esp_idf_svc::hal::i2c::config::Config as I2cConfig;
use esp_idf_svc::hal::i2c::I2cDriver;
use esp_idf_svc::hal::i2s::config::{
    Config as I2sChannelConfig, DataBitWidth, SlotMode, StdClkConfig, StdConfig, StdGpioConfig,
    StdSlotConfig,
};
use esp_idf_svc::hal::i2s::{I2sDriver, I2sTx};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::units::Hertz;
use ssd1306::prelude::*;
use ssd1306::{I2CDisplayInterface, Ssd1306};
use std::thread;
use std::time::Duration;

/// Max recording: 5 seconds at 8kHz
const MAX_SAMPLES: usize = 40000;

/// Convert 12-bit unsigned ADC to signed 16-bit PCM.
fn adc_to_pcm(sample: &AdcMeasurement) -> i16 {
    (sample.data() as i16 - 2048) * 16
}

fn pcm_as_bytes(pcm: &[i16]) -> &[u8] {
    unsafe { core::slice::from_raw_parts(pcm.as_ptr() as *const u8, pcm.len() * 2) }
}

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("Raw audio loopback test starting...");

    let p = Peripherals::take().unwrap();

    // Vext power on (GPIO36 LOW)
    let mut vext = PinDriver::output(p.pins.gpio36).unwrap();
    vext.set_low().unwrap();
    thread::sleep(Duration::from_millis(50));

    // PTT button (GPIO0, active LOW)
    let button = PinDriver::input(p.pins.gpio0, Pull::Up).unwrap();

    // ADC continuous for mic on GPIO4 (ADC1_CH3) at 8kHz — rev4 pinout
    let adc_cfg = AdcContConfig::new()
        .sample_freq(Hertz(8000))
        .frame_measurements(320)
        .frames_count(2);
    let mut adc = AdcContDriver::new(p.adc1, &adc_cfg, Attenuated::db12(p.pins.gpio4)).unwrap();

    // I2S TX for speaker — rev4 pinout: BCLK=47, WS=48, DIN=33
    let i2s_cfg = StdConfig::new(
        I2sChannelConfig::new()
            .dma_buffer_count(8)
            .frames_per_buffer(320)
            .auto_clear(true),
        StdClkConfig::from_sample_rate_hz(8000),
        StdSlotConfig::philips_slot_default(DataBitWidth::Bits16, SlotMode::Stereo),
        StdGpioConfig::default(),
    );
    let mut i2s = I2sDriver::<I2sTx>::new_std_tx(
        p.i2s0,
        &i2s_cfg,
        p.pins.gpio47, // BCLK
        p.pins.gpio33, // DIN
        None::<AnyIOPin>,
        p.pins.gpio48, // WS
    )
    .unwrap();
    i2s.tx_enable().unwrap();

    // OLED init
    let mut oled_rst = PinDriver::output(p.pins.gpio21).unwrap();
    oled_rst.set_low().unwrap();
    thread::sleep(Duration::from_millis(50));
    oled_rst.set_high().unwrap();
    thread::sleep(Duration::from_millis(50));
    let i2c = I2cDriver::new(p.i2c0, p.pins.gpio17, p.pins.gpio18, &I2cConfig::default()).unwrap();
    let interface = I2CDisplayInterface::new(i2c);
    let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();
    display.init().unwrap();
    display.set_brightness(Brightness::BRIGHTEST).unwrap();
    let style = MonoTextStyleBuilder::new()
        .font(&FONT_10X20)
        .text_color(BinaryColor::On)
        .build();

    let show_status =
        |display: &mut ssd1306::Ssd1306<_, _, ssd1306::mode::BufferedGraphicsMode<_>>,
         msg: &str| {
            display.clear_buffer();
            let _ = Text::new(msg, Point::new(10, 38), style).draw(display);
            let _ = display.flush();
        };

    // Heap buffers
    let mut mic_buf = vec![AdcMeasurement::new(); 320];
    let mut rec_buf = vec![0i16; MAX_SAMPLES];
    let mut stereo_buf = vec![0i16; 640]; // 320 mono → 640 stereo interleaved

    log::info!("Ready — hold PTT to record (max 5s), release to play raw PCM");
    adc.start().unwrap();
    show_status(&mut display, "IDLE");

    loop {
        // Wait for PTT press
        while button.is_high() {
            thread::sleep(Duration::from_millis(10));
        }

        // --- Record ---
        log::info!("Recording...");
        show_status(&mut display, "LISTENING");
        let _ = adc.read(&mut mic_buf, 0); // drain stale
        let mut rec_len: usize = 0;

        while button.is_low() && rec_len < MAX_SAMPLES {
            let count = adc.read(&mut mic_buf, 100).unwrap_or(0);
            let remaining = MAX_SAMPLES - rec_len;
            let n = count.min(remaining);
            for i in 0..n {
                rec_buf[rec_len + i] = adc_to_pcm(&mic_buf[i]);
            }
            rec_len += n;
        }
        log::info!("Recorded {} samples ({}ms)", rec_len, rec_len / 8);

        // --- Play back raw ---
        log::info!("Playing raw...");
        show_status(&mut display, "PLAYING");

        // Play in 320-sample chunks (40ms each)
        let mut pos = 0;
        while pos < rec_len {
            let chunk_end = (pos + 320).min(rec_len);
            let chunk = &rec_buf[pos..chunk_end];

            // Mono → stereo interleave
            for (i, &sample) in chunk.iter().enumerate() {
                stereo_buf[i * 2] = sample;
                stereo_buf[i * 2 + 1] = sample;
            }
            let stereo_bytes = pcm_as_bytes(&stereo_buf[..chunk.len() * 2]);
            i2s.write_all(stereo_bytes, BLOCK).unwrap();
            pos = chunk_end;
        }

        log::info!("Playback done");
        show_status(&mut display, "IDLE");
    }
}
