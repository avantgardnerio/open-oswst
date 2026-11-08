//! Voice playback test: a real voice clip on the speaker's I2S pins
//! (BCLK=GPIO47, WS/LRCLK=GPIO48, DIN=GPIO33), once at each level, so two
//! output paths (PCM5102A + LM386, MAX98357A) can be compared by ear on the
//! same speech. The OLED shows the level playing.
//! 16 kHz, 32-bit stereo slots: what the PCM5102A's PLL needs (see
//! pcm_test.rs); the MAX98357A takes it as well.
//! The clip is NOT in the repo (it's a recording of a person): 16 kHz mono
//! 16-bit little-endian raw at target/voice_clip.raw, peak-normalized
//! Build & flash: cargo build --bin voice_test --features voice-clip, then espflash with our
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
use std::time::Duration;

const SAMPLE_RATE: u32 = 16000;
static CLIP: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/target/voice_clip.raw"
));
/// The clip's peak at each level, in dBFS: one full play each, then around
/// again. -24 matches the tone step that was as loud as the radio's speaker
const LEVELS_DBFS: [i32; 3] = [-30, -24, -18];
/// Samples per I2S write
const CHUNK_SAMPLES: usize = 400;
const GAP_MS: u64 = 700;

fn pcm_as_bytes(pcm: &[i32]) -> &[u8] {
    unsafe { core::slice::from_raw_parts(pcm.as_ptr() as *const u8, pcm.len() * 4) }
}

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!(
        "Voice test: {} s clip at {:?} dBFS peak",
        CLIP.len() / 2 / SAMPLE_RATE as usize,
        LEVELS_DBFS
    );

    let p = Peripherals::take().unwrap();

    // Vext power on (GPIO36 LOW): the OLED runs from it, and on a radio board the MAX98357A
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

    draw(&mut display, "voice", "I2S init...");

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

    let silence = vec![0i32; CHUNK_SAMPLES * 2];
    let mut stereo = vec![0i32; CHUNK_SAMPLES * 2];
    loop {
        for &dbfs in &LEVELS_DBFS {
            log::info!("voice at {} dBFS peak", dbfs);
            draw(&mut display, "voice", &format!("{} dBFS", dbfs));
            // The clip's 16-bit samples, shifted into the 32-bit slot's top half and scaled
            let gain = 10f32.powf(dbfs as f32 / 20.0) * 65536.0;
            for chunk in CLIP.chunks(CHUNK_SAMPLES * 2) {
                stereo.fill(0);
                for (index, bytes) in chunk.chunks_exact(2).enumerate() {
                    let sample = (i16::from_le_bytes([bytes[0], bytes[1]]) as f32 * gain) as i32;
                    stereo[index * 2] = sample;
                    stereo[index * 2 + 1] = sample;
                }
                i2s.write_all(pcm_as_bytes(&stereo), BLOCK).unwrap();
            }
            let gap_writes = GAP_MS * SAMPLE_RATE as u64 / 1000 / CHUNK_SAMPLES as u64;
            for _ in 0..gap_writes {
                i2s.write_all(pcm_as_bytes(&silence), BLOCK).unwrap();
            }
        }
    }
}
