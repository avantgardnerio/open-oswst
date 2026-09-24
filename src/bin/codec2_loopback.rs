//! Audio loopback test with Codec2 — hold PTT to record, release to encode→decode→play.
//! Goes through the main app's codec thread (codec.rs) in whole packets, no radio.
//! Build & flash: cargo build --bin codec2_loopback && espflash flash -p /dev/ttyACM1 --partition-table target/xtensa-esp32s3-espidf/debug/partition-table.bin target/xtensa-esp32s3-espidf/debug/codec2_loopback

use embassy_futures::join::join;
use embedded_graphics::mono_font::ascii::FONT_10X20;
use embedded_graphics::mono_font::{MonoTextStyle, MonoTextStyleBuilder};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use esp_idf_svc::hal::gpio::{PinDriver, Pull};
use esp_idf_svc::hal::task::block_on;
use open_oswst::board;
use open_oswst::codec::{
    self, CodecRequest, CodecResponse, CODEC_REPLY, FRAMES_PER_PACKET, HEADER_BYTES, PAYLOAD_BYTES,
};
use open_oswst::devices::mic::{self, FRAME_SAMPLES};
use open_oswst::devices::screen::{self, Screen};
use open_oswst::devices::speaker::{self, SPK_FRAMES};
use std::sync::Arc;

/// Mono samples per packet (4 frames × 320 = 160ms)
const PACKET_SAMPLES: usize = FRAMES_PER_PACKET * FRAME_SAMPLES;

/// Max recording: ~5 seconds = 31 packets
const MAX_PACKETS: usize = 31;
const MAX_SAMPLES: usize = MAX_PACKETS * PACKET_SAMPLES; // 39680

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

    // Codec thread, same as the main app
    let (codec_tx, codec_rx) = std::sync::mpsc::sync_channel::<CodecRequest>(2);
    std::thread::Builder::new()
        .name("codec".into())
        .stack_size(32768)
        .spawn(move || codec::run(codec_rx))
        .unwrap();

    let mut rec_buf = vec![0i16; MAX_SAMPLES];
    let mut payloads = vec![[0u8; PAYLOAD_BYTES]; MAX_PACKETS];

    block_on(async {
        let speaker_fut = speaker::init(board.speaker).await;

        let loopback = async {
            log::info!("Ready — hold PTT to record (max 5s), release to encode→decode→play");
            show_status(&screen, style, "IDLE");

            loop {
                button.wait_for_low().await.unwrap();

                // --- Record (whole packets) ---
                log::info!("Recording...");
                show_status(&screen, style, "LISTENING");
                mic.drain();
                let mut num_packets = 0;
                while button.is_low() && num_packets < MAX_PACKETS {
                    for i in 0..FRAMES_PER_PACKET {
                        let start = num_packets * PACKET_SAMPLES + i * FRAME_SAMPLES;
                        mic.read(&mut rec_buf[start..start + FRAME_SAMPLES]).await;
                    }
                    num_packets += 1;
                }
                log::info!("Recorded {} packets ({}ms)", num_packets, num_packets * 160);
                if num_packets == 0 {
                    continue;
                }

                // --- Encode ---
                let t0 = std::time::Instant::now();
                for (p, payload) in payloads[..num_packets].iter_mut().enumerate() {
                    let pcm: Box<[i16]> =
                        rec_buf[p * PACKET_SAMPLES..(p + 1) * PACKET_SAMPLES].into();
                    codec_tx
                        .send(CodecRequest::encode([0; HEADER_BYTES], pcm))
                        .unwrap();
                    if let CodecResponse::Encoded { packet } = CODEC_REPLY.receive().await {
                        payload.copy_from_slice(&packet[HEADER_BYTES..]);
                    }
                }
                let enc_ms = t0.elapsed().as_millis();

                // --- Decode (all before playback, so codec time can't starve the
                // speaker). Output is stereo L=R; keep the left channel, back over
                // the recording, to avoid a second 160KB buffer ---
                let t0 = std::time::Instant::now();
                for (p, payload) in payloads[..num_packets].iter().enumerate() {
                    let seq = (p & 0x0F) as u8;
                    codec_tx
                        .send(CodecRequest::decode(seq, 0, *payload))
                        .unwrap();
                    if let CodecResponse::Decoded { pcm, .. } = CODEC_REPLY.receive().await {
                        let mono = &mut rec_buf[p * PACKET_SAMPLES..(p + 1) * PACKET_SAMPLES];
                        for (dst, lr) in mono.iter_mut().zip(pcm.chunks(2)) {
                            *dst = lr[0];
                        }
                    }
                }
                let dec_ms = t0.elapsed().as_millis();
                log::info!(
                    "Encoded {} packets in {}ms ({}ms/packet), decoded in {}ms",
                    num_packets,
                    enc_ms,
                    enc_ms / num_packets as u128,
                    dec_ms
                );

                // --- Play ---
                log::info!("Playing...");
                show_status(&screen, style, "PLAYING");
                for chunk in rec_buf[..num_packets * PACKET_SAMPLES].chunks(FRAME_SAMPLES) {
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
