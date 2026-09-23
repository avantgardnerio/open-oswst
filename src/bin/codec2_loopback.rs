//! Audio loopback test with Codec2 — hold PTT to record, release to encode→decode→play.
//! Same Codec2 mode as the main app (MODE_1200), no radio.
//! Build & flash: cargo build --bin codec2_loopback && espflash flash -p /dev/ttyACM1 --partition-table target/xtensa-esp32s3-espidf/debug/partition-table.bin target/xtensa-esp32s3-espidf/debug/codec2_loopback

use codec2::{Codec2, Codec2Mode};
use embassy_futures::join::join;
use embedded_graphics::mono_font::ascii::FONT_10X20;
use embedded_graphics::mono_font::{MonoTextStyle, MonoTextStyleBuilder};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use esp_idf_svc::hal::gpio::{PinDriver, Pull};
use esp_idf_svc::hal::task::block_on;
use open_oswst::board;
use open_oswst::mic::{self, FRAME_SAMPLES};
use open_oswst::screen::{self, Screen};
use open_oswst::speaker::{self, SPK_FRAMES};
use std::sync::Arc;

/// Codec2 MODE_1200: 320 samples (40ms) → 6 bytes per frame
const FRAME_BYTES: usize = 6;

/// Max recording: 5 seconds = 125 frames
const MAX_FRAMES: usize = 125;
const MAX_SAMPLES: usize = MAX_FRAMES * FRAME_SAMPLES; // 40000

fn show_status(screen: &Screen, style: MonoTextStyle<'_, BinaryColor>, msg: &str) {
    let mut frame = screen.frame();
    let _ = Text::new(msg, Point::new(10, 38), style).draw(&mut frame);
    screen.show(frame);
}

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("Audio loopback test starting...");

    let board = board::take();
    let mut button = PinDriver::input(board.ptt, Pull::Up).unwrap();
    let mut mic = mic::init(board.mic);
    let screen = screen::init(board.screen);
    let style = MonoTextStyleBuilder::new()
        .font(&FONT_10X20)
        .text_color(BinaryColor::On)
        .build();

    let mut encoder = Box::new(Codec2::new(Codec2Mode::MODE_1200));
    let mut decoder = Box::new(Codec2::new(Codec2Mode::MODE_1200));
    log::info!("Codec2 initialized (MODE_1200)");

    let mut rec_buf = vec![0i16; MAX_SAMPLES];
    let mut codec_buf = vec![0u8; MAX_FRAMES * FRAME_BYTES];

    block_on(async {
        let speaker_fut = speaker::init(board.speaker).await;

        let loopback = async {
            log::info!("Ready — hold PTT to record (max 5s), release to encode→decode→play");
            show_status(&screen, style, "IDLE");

            loop {
                button.wait_for_low().await.unwrap();

                // --- Record ---
                log::info!("Recording...");
                show_status(&screen, style, "LISTENING");
                mic.drain();
                let mut num_frames = 0;
                while button.is_low() && num_frames < MAX_FRAMES {
                    let start = num_frames * FRAME_SAMPLES;
                    mic.read(&mut rec_buf[start..start + FRAME_SAMPLES]).await;
                    num_frames += 1;
                }
                log::info!("Recorded {} frames ({}ms)", num_frames, num_frames * 40);
                if num_frames == 0 {
                    continue;
                }

                // --- Encode ---
                let t0 = std::time::Instant::now();
                for f in 0..num_frames {
                    let pcm = &rec_buf[f * FRAME_SAMPLES..(f + 1) * FRAME_SAMPLES];
                    let coded = &mut codec_buf[f * FRAME_BYTES..(f + 1) * FRAME_BYTES];
                    encoder.encode(coded, pcm);
                }
                let enc_ms = t0.elapsed().as_millis();

                // --- Decode (in place over the recording, before playback, so
                // codec time can't starve the speaker) ---
                let t0 = std::time::Instant::now();
                for f in 0..num_frames {
                    let coded = &codec_buf[f * FRAME_BYTES..(f + 1) * FRAME_BYTES];
                    decoder.decode(
                        &mut rec_buf[f * FRAME_SAMPLES..(f + 1) * FRAME_SAMPLES],
                        coded,
                    );
                }
                let dec_ms = t0.elapsed().as_millis();
                log::info!(
                    "Encoded {} frames in {}ms, decoded in {}ms",
                    num_frames,
                    enc_ms,
                    dec_ms
                );

                // --- Play ---
                log::info!("Playing...");
                show_status(&screen, style, "PLAYING");
                for chunk in rec_buf[..num_frames * FRAME_SAMPLES].chunks(FRAME_SAMPLES) {
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
