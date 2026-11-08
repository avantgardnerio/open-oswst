//! PCM5102A earpiece test: a 440 Hz tone on the speaker's I2S pins
//! (BCLK=GPIO47, WS/LRCLK=GPIO48, DIN=GPIO33), stepping through levels so the
//! ear and a scope can find how loud the PCM5102A's line out gets into an
//! 18 ohm earpiece. The OLED shows the level playing.
//! The PCM5102A's PLL (SCK grounded) needs BCLK at 32/48/64 x the sample
//! rate and can't lock below 16 kHz (datasheet table 11): 16 kHz, 32-bit
//! stereo slots = 64 x fs.
//! Its line out is 2.1 V rms at full scale (unloaded); the K1 earpiece was
//! "fine" at 0.13 V rms (2026-10-29), about -24 dBFS. The steps stop at
//! -12 dBFS (~0.5 V rms unloaded): 0.65 V rms was still "not too loud".
//! Build & flash: cargo build --bin pcm_test, then espflash with our
//! --bootloader and --partition-table (see scripts/flash-all.sh)

use embedded_graphics::mono_font::ascii::FONT_9X18;
use embedded_graphics::mono_font::MonoTextStyleBuilder;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use esp_idf_svc::hal::delay::BLOCK;
use esp_idf_svc::hal::gpio::{AnyIOPin, PinDriver};
use esp_idf_svc::hal::i2c::config::Config as I2cConfig;
use esp_idf_svc::hal::i2c::I2cDriver;
use esp_idf_svc::hal::i2s::config::{
    Config as I2sChannelConfig, DataBitWidth, SlotMode, StdClkConfig, StdConfig, StdGpioConfig,
    StdSlotConfig,
};
use esp_idf_svc::hal::i2s::{I2sDriver, I2sTx};
use esp_idf_svc::hal::peripherals::Peripherals;
use ssd1306::mode::BufferedGraphicsMode;
use ssd1306::prelude::*;
use ssd1306::{I2CDisplayInterface, Ssd1306};
use std::thread;
use std::time::{Duration, Instant};

const SAMPLE_RATE: u32 = 16000;
const FREQ: f32 = 440.0;
/// 16000/gcd(16000,440) = 400 samples = exactly 11 cycles of 440 Hz. Seamless loop.
const BUF_SAMPLES: usize = 400;
/// Levels in dBFS, each played for STEP_SECONDS, then around again. Just the
/// quietest while an amp (LM386) follows the PCM5102A: [-36, -30, -24, -18, -12]
/// was for the PCM5102A alone
const LEVELS_DBFS: [i32; 4] = [-36, -30, -24, -18];
const STEP_SECONDS: u64 = 4;
/// The PCM5102A's full scale, unloaded (datasheet)
const FULL_SCALE_VRMS: f32 = 2.1;

fn pcm_as_bytes(pcm: &[i32]) -> &[u8] {
    unsafe { core::slice::from_raw_parts(pcm.as_ptr() as *const u8, pcm.len() * 4) }
}

/// One loop of the tone at `dbfs`, stereo-interleaved 32-bit samples
fn tone(dbfs: i32) -> Vec<i32> {
    let amplitude = i32::MAX as f32 * 10f32.powf(dbfs as f32 / 20.0);
    let mut stereo = vec![0i32; BUF_SAMPLES * 2];
    for index in 0..BUF_SAMPLES {
        let seconds = index as f32 / SAMPLE_RATE as f32;
        let sample = (amplitude * (2.0 * core::f32::consts::PI * FREQ * seconds).sin()) as i32;
        stereo[index * 2] = sample;
        stereo[index * 2 + 1] = sample;
    }
    stereo
}

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("PCM5102A test: 440 Hz, levels {:?} dBFS", LEVELS_DBFS);

    let p = Peripherals::take().unwrap();

    // Vext power on (GPIO36 LOW): the OLED runs from it
    let mut vext = PinDriver::output(p.pins.gpio36).unwrap();
    vext.set_low().unwrap();
    thread::sleep(Duration::from_millis(50));

    // OLED for status readout.
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
    let _oled_rst = oled_rst;

    let draw = |display: &mut _, line1: &str, line2: &str| {
        let d: &mut Ssd1306<_, _, BufferedGraphicsMode<DisplaySize128x64>> = display;
        d.clear_buffer();
        Text::new(line1, Point::new(4, 22), style).draw(d).unwrap();
        Text::new(line2, Point::new(4, 48), style).draw(d).unwrap();
        d.flush().unwrap();
    };

    draw(&mut display, "PCM 440Hz", "I2S init...");

    let i2s_cfg = StdConfig::new(
        I2sChannelConfig::new()
            .dma_buffer_count(8)
            .frames_per_buffer(400)
            .auto_clear(true),
        StdClkConfig::from_sample_rate_hz(SAMPLE_RATE),
        StdSlotConfig::philips_slot_default(DataBitWidth::Bits32, SlotMode::Stereo),
        StdGpioConfig::default(),
    );
    let mut i2s = I2sDriver::<I2sTx>::new_std_tx(
        p.i2s0,
        &i2s_cfg,
        p.pins.gpio47, // BCLK
        p.pins.gpio33, // DIN
        None::<AnyIOPin>,
        p.pins.gpio48, // WS (LRCLK)
    )
    .unwrap();
    i2s.tx_enable().unwrap();

    let tones: Vec<Vec<i32>> = LEVELS_DBFS.iter().map(|&dbfs| tone(dbfs)).collect();
    loop {
        for (dbfs, stereo) in LEVELS_DBFS.iter().zip(&tones) {
            let volts = FULL_SCALE_VRMS * 10f32.powf(*dbfs as f32 / 20.0);
            log::info!("{} dBFS (~{:.3} V rms unloaded)", dbfs, volts);
            draw(
                &mut display,
                &format!("{} dBFS", dbfs),
                &format!("~{:.3} V", volts),
            );
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(STEP_SECONDS) {
                i2s.write_all(pcm_as_bytes(stereo), BLOCK).unwrap();
            }
        }
    }
}
