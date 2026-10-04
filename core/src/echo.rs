//! Echo mode, for range testing without a second person: record a
//! transmission as it's heard, then play it back over the air.
//!
//! Packets are stored exactly as received, never decoded and re-encoded, so
//! the replay is what this station heard. A missing packet is replaced with
//! encoded silence, so dropouts are heard where they happened, and the
//! listener gets an unbroken sequence.

use crate::codec::{FRAMES_PER_PACKET, PAYLOAD_BYTES};
use crate::devices::radio::{TxRequest, TX_CHAN};
use embassy_time::{Duration, Ticker, Timer};
use std::time::Instant;

use crate::packet::{self, PacketType};

type Payload = [u8; PAYLOAD_BYTES];

/// Audio per packet: FRAMES_PER_PACKET × 40ms
const PACKET_MS: u64 = FRAMES_PER_PACKET as u64 * 40;
/// Recording limit: 30s of audio (~5KB)
const MAX_PACKETS: usize = (30_000 / PACKET_MS) as usize;
/// Talker gone quiet this long without an EOT (it was lost): replay anyway
const TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);
/// Pause before replaying, so the talker has let go of PTT
const REPLAY_DELAY_MS: u64 = 1000;

pub struct Recorder {
    txid: Option<u8>, // who we're recording; None = idle
    next_seq: u8,
    packets: Vec<Payload>,
    silence: Payload,
    last_rx: Instant,
}

impl Recorder {
    /// `silence` is one packet of encoded silence, used to fill gaps.
    pub fn new(silence: Payload) -> Self {
        Recorder {
            txid: None,
            next_seq: 0,
            packets: Vec::new(),
            silence,
            last_rx: Instant::now(),
        }
    }

    /// Who we're recording, if anyone.
    pub fn txid(&self) -> Option<u8> {
        self.txid
    }

    /// Store one packet. The first packet starts a recording; packets from
    /// anyone else are ignored until it ends. False if not stored (someone
    /// else's, or a copy already stored)
    pub fn record(&mut self, txid: u8, seq: u8, payload: &[u8]) -> bool {
        match self.txid {
            None => {
                self.txid = Some(txid);
                self.next_seq = seq;
            }
            Some(current) if current != txid => return false,
            Some(_) => {}
        }
        self.last_rx = Instant::now();

        // A backwards step is a duplicate, e.g. heard again via a repeater
        let missing = seq.wrapping_sub(self.next_seq) & 0x0F;
        if missing > 7 {
            return false;
        }
        for _ in 0..missing {
            self.push(self.silence);
        }
        let mut stored = [0u8; PAYLOAD_BYTES];
        stored.copy_from_slice(payload);
        self.push(stored);
        self.next_seq = seq.wrapping_add(1) & 0x0F;
        true
    }

    fn push(&mut self, payload: Payload) {
        if self.packets.len() < MAX_PACKETS {
            self.packets.push(payload);
        }
    }

    /// Recording, but the talker has gone quiet without an EOT (it was lost).
    pub fn timed_out(&self) -> bool {
        self.txid.is_some() && self.last_rx.elapsed() > TIMEOUT
    }

    /// End the recording and hand it over for replay.
    pub fn take(&mut self) -> Vec<Payload> {
        self.txid = None;
        std::mem::take(&mut self.packets)
    }
}

/// Send a recording back out at the pace it was spoken, then the end packet
/// `end` makes for the seq after the last. With `wake`, a wake-up packet
/// goes first, in the slot before the audio.
pub async fn replay(
    packets: Vec<Payload>,
    txid: u8,
    wake: bool,
    end: impl FnOnce(u8) -> heapless::Vec<u8, 255>,
) {
    Timer::after_millis(REPLAY_DELAY_MS).await;
    if wake {
        TX_CHAN.send(packet::wake(txid)).await;
    }

    let mut ticker = Ticker::every(Duration::from_millis(PACKET_MS));
    let mut seq = 0u8;
    for payload in &packets {
        ticker.next().await;
        let mut data = heapless::Vec::new();
        let _ = data.extend_from_slice(&packet::pack(PacketType::Echo, txid, seq));
        let _ = data.extend_from_slice(payload);
        TX_CHAN
            .send(TxRequest {
                data,
                preamble: None,
                channel: 0,
            })
            .await;
        seq = (seq + 1) & 0x0F;
    }

    // On the beat, a slot after the last packet, like any packet: sent
    // straight after it, it was on the air while a repeater relayed that
    // last packet, and the repeater never heard it
    ticker.next().await;
    TX_CHAN
        .send(TxRequest {
            data: end(seq),
            preamble: None,
            channel: 0,
        })
        .await;
}
