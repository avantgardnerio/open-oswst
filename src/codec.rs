use codec2::{Codec2, Codec2Mode};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use std::sync::mpsc::Receiver;

/// Codec2 MODE_1200: 320 samples → 6 bytes per frame
pub const CODEC2_FRAME_BYTES: usize = 6;
pub const CODEC2_FRAME_SAMPLES: usize = 320;

/// Pack 4 Codec2 frames per LoRa packet (24 bytes payload, 160ms audio).
/// 2-byte header for repeater dedup/reorder: |5b type|7b txid|4b seq| = 16 bits.
/// 26 bytes total sits in the same SF8 symbol bin as 24 — zero air time cost.
pub const FRAMES_PER_PACKET: usize = 4;
pub const HEADER_BYTES: usize = 2;
pub const PAYLOAD_BYTES: usize = CODEC2_FRAME_BYTES * FRAMES_PER_PACKET; // 24
pub const PACKET_BYTES: usize = HEADER_BYTES + PAYLOAD_BYTES; // 26

/// Stereo-interleaved samples for one decoded packet (4 frames × 320 samples × 2 channels)
pub const STEREO_PACKET_SAMPLES: usize = FRAMES_PER_PACKET * CODEC2_FRAME_SAMPLES * 2;

pub enum CodecRequest {
    Encode {
        header: [u8; 2],
        pcm: Box<[i16]>, // 1280 samples (4×320)
    },
    Decode {
        seq: u8,
        txid: u8,
        payload: [u8; PAYLOAD_BYTES],
    },
}

pub enum CodecResponse {
    Encoded {
        packet: heapless::Vec<u8, 255>,
    },
    Decoded {
        seq: u8,
        txid: u8,
        pcm: Box<[i16]>, // 2560 stereo samples
    },
}

/// Single-slot reply channel — acts as a oneshot since app always awaits before next request.
pub static CODEC_REPLY: Channel<CriticalSectionRawMutex, CodecResponse, 1> = Channel::new();

/// Codec thread entry point. Owns encoder + decoder, loops on requests.
pub fn run(rx: Receiver<CodecRequest>) {
    let mut encoder = Box::new(Codec2::new(Codec2Mode::MODE_1200));
    log::info!("Codec2 encoder initialized (thread)");
    let mut decoder = Box::new(Codec2::new(Codec2Mode::MODE_1200));
    log::info!("Codec2 decoder initialized (thread)");

    let mut decode_buf = vec![0i16; CODEC2_FRAME_SAMPLES].into_boxed_slice();

    log::info!("Codec thread ready");

    while let Ok(req) = rx.recv() {
        match req {
            CodecRequest::Encode { header, pcm } => {
                let mut packet = heapless::Vec::<u8, 255>::new();
                let _ = packet.extend_from_slice(&header);

                for i in 0..FRAMES_PER_PACKET {
                    let start = i * CODEC2_FRAME_SAMPLES;
                    let end = start + CODEC2_FRAME_SAMPLES;
                    let mut frame_bytes = [0u8; CODEC2_FRAME_BYTES];
                    encoder.encode(&mut frame_bytes, &pcm[start..end]);
                    let _ = packet.extend_from_slice(&frame_bytes);
                }

                CODEC_REPLY.try_send(CodecResponse::Encoded { packet }).ok();
            }
            CodecRequest::Decode { seq, txid, payload } => {
                let mut pcm = vec![0i16; STEREO_PACKET_SAMPLES].into_boxed_slice();

                for i in 0..FRAMES_PER_PACKET {
                    let coded = &payload[i * CODEC2_FRAME_BYTES..(i + 1) * CODEC2_FRAME_BYTES];
                    decoder.decode(&mut decode_buf, coded);
                    let offset = i * CODEC2_FRAME_SAMPLES * 2;
                    for (j, &sample) in decode_buf.iter().enumerate() {
                        pcm[offset + j * 2] = sample;
                        pcm[offset + j * 2 + 1] = sample;
                    }
                }

                CODEC_REPLY
                    .try_send(CodecResponse::Decoded { seq, txid, pcm })
                    .ok();
            }
        }
    }

    log::warn!("Codec thread exiting — channel closed");
}
