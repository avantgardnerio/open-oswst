//! Raw audio loopback test (no Codec2) — hold PTT to record, release to play back raw PCM.
//! Build & flash: cargo build --bin raw_loopback && espflash flash -p /dev/ttyACM1 --partition-table target/xtensa-esp32s3-espidf/debug/partition-table.bin target/xtensa-esp32s3-espidf/debug/raw_loopback

use embassy_futures::join::join;
use embedded_graphics::mono_font::ascii::FONT_10X20;
use embedded_graphics::mono_font::{MonoTextStyle, MonoTextStyleBuilder};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use esp_idf_svc::hal::gpio::{PinDriver, Pull};
use esp_idf_svc::hal::task::block_on;
use open_oswst::board;
use open_oswst::devices::mic::{self, FRAME_SAMPLES};
use open_oswst::devices::screen::{self, Screen};
use open_oswst::devices::speaker::{self, SPK_FRAMES};
use std::sync::Arc;

/// Max recording: 5 seconds at 8kHz (125 frames)
const MAX_SAMPLES: usize = 40000;

fn show_status(screen: &Screen, style: MonoTextStyle<'_, BinaryColor>, msg: &str) {
    let mut frame = screen.frame();
    let _ = Text::new(msg, Point::new(10, 38), style).draw(&mut frame);
    screen.show(frame);
}

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("Raw audio loopback test starting...");

    let board = board::take();
    let mut button = PinDriver::input(board.ptt, Pull::Up).unwrap();
    let mut mic = mic::init(board.mic);
    let screen = screen::init(board.screen);
    let style = MonoTextStyleBuilder::new()
        .font(&FONT_10X20)
        .text_color(BinaryColor::On)
        .build();

    let mut rec_buf = vec![0i16; MAX_SAMPLES];

    block_on(async {
        let speaker_fut = speaker::init(board.speaker).await;

        let loopback = async {
            log::info!("Ready — hold PTT to record (max 5s), release to play raw PCM");
            show_status(&screen, style, "IDLE");

            loop {
                button.wait_for_low().await.unwrap();

                // --- Record ---
                log::info!("Recording...");
                show_status(&screen, style, "LISTENING");
                mic.drain();
                let mut rec_len: usize = 0;
                while button.is_low() && rec_len < MAX_SAMPLES {
                    mic.read(&mut rec_buf[rec_len..rec_len + FRAME_SAMPLES])
                        .await;
                    rec_len += FRAME_SAMPLES;
                }
                log::info!("Recorded {} samples ({}ms)", rec_len, rec_len / 8);

                // --- Play back raw ---
                log::info!("Playing raw...");
                show_status(&screen, style, "PLAYING");
                for chunk in rec_buf[..rec_len].chunks(FRAME_SAMPLES) {
                    // Mono → stereo interleave
                    let stereo: Arc<[i16]> = chunk.iter().flat_map(|&s| [s, s]).collect();
                    SPK_FRAMES.send(stereo).await;
                }

                log::info!("Playback done");
                show_status(&screen, style, "IDLE");
            }
        };

        join(speaker_fut, loopback).await;
    });
}
