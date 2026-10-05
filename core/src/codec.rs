use codec2::{Codec2, Codec2Mode};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Instant;

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
        txid: u8,
        /// Which of the talker's packets, counted on its conveyor
        /// (conveyor::Landing::packet)
        packet: i64,
        payload: [u8; PAYLOAD_BYTES],
        /// When the app asked, handed back in the reply: how long it waited
        asked: Instant,
    },
}

impl CodecRequest {
    pub fn encode(header: [u8; 2], pcm: Box<[i16]>) -> Self {
        Self::Encode { header, pcm }
    }

    pub fn decode(txid: u8, packet: i64, payload: [u8; PAYLOAD_BYTES]) -> Self {
        Self::Decode {
            txid,
            packet,
            payload,
            asked: Instant::now(),
        }
    }
}

/// One packet's decoded audio, for the speaker
pub struct Decoded {
    pub txid: u8,
    /// Which of the talker's packets (from the request)
    pub packet: i64,
    /// Stereo, ready to play: 2560 samples (4 frames × 320 × 2 channels).
    /// Built as an Arc in place, so it goes to the speaker without a copy
    pub pcm: Arc<[i16]>,
    /// The codec thread's own time on it
    pub decode_us: u32,
    /// When the app asked (from the request)
    pub asked: Instant,
}

/// Encoded packets, back to the talker, which waits for each in turn
pub static ENCODED: Channel<CriticalSectionRawMutex, heapless::Vec<u8, 255>, 1> = Channel::new();

/// Decoded audio, back to the app as an event of its own: it never waits for a
/// decode. Apart from ENCODED, so a talker never takes a decode for its encode.
/// Requests are taken in order, so replies come back in packet order
pub static DECODED: Channel<CriticalSectionRawMutex, Decoded, 2> = Channel::new();

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

                ENCODED.try_send(packet).ok();
            }
            CodecRequest::Decode {
                txid,
                packet,
                payload,
                asked,
            } => {
                let started = Instant::now();
                // Collected straight into its Arc: one allocation, no copy.
                // TODO: take it from a pool allocated at boot instead, so the
                // audio path never touches the heap
                let mut pcm: Arc<[i16]> =
                    std::iter::repeat_n(0i16, STEREO_PACKET_SAMPLES).collect();
                let stereo = Arc::get_mut(&mut pcm).expect("not shared yet");

                for i in 0..FRAMES_PER_PACKET {
                    let coded = &payload[i * CODEC2_FRAME_BYTES..(i + 1) * CODEC2_FRAME_BYTES];
                    decoder.decode(&mut decode_buf, coded);
                    let offset = i * CODEC2_FRAME_SAMPLES * 2;
                    for (j, &sample) in decode_buf.iter().enumerate() {
                        stereo[offset + j * 2] = sample;
                        stereo[offset + j * 2 + 1] = sample;
                    }
                }

                let decoded = Decoded {
                    txid,
                    packet,
                    pcm,
                    decode_us: started.elapsed().as_micros() as u32,
                    asked,
                };
                if DECODED.try_send(decoded).is_err() {
                    log::warn!(
                        "Codec: decoded packet {} dropped, the app is behind",
                        packet
                    );
                }
            }
        }
    }

    log::warn!("Codec thread exiting — channel closed");
}
