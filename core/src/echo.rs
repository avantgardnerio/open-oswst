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

use crate::packet::{self, PacketType};

type Payload = [u8; PAYLOAD_BYTES];

/// Audio per packet: FRAMES_PER_PACKET × 40ms
const PACKET_MS: u64 = FRAMES_PER_PACKET as u64 * 40;
/// Recording limit: 30s of audio (~5KB)
const MAX_PACKETS: usize = (30_000 / PACKET_MS) as usize;
/// Pause before replaying, so the talker has let go of PTT
const REPLAY_DELAY_MS: u64 = 1000;

pub struct Recorder {
    txid: Option<u8>, // who we're recording; None = idle
    /// The talker's packet we expect next, counted on the conveyor
    /// (conveyor::Landing::packet), so a gap of any length is measured right
    next_packet: i64,
    packets: Vec<Payload>,
    silence: Payload,
}

impl Recorder {
    /// `silence` is one packet of encoded silence, used to fill gaps.
    pub fn new(silence: Payload) -> Self {
        Recorder {
            txid: None,
            next_packet: 0,
            packets: Vec::new(),
            silence,
        }
    }

    /// Who we're recording, if anyone.
    pub fn txid(&self) -> Option<u8> {
        self.txid
    }

    /// Store one packet: the talker's `packet`th on the conveyor. The first
    /// packet starts a recording; packets from anyone else are ignored until
    /// it ends. False if not stored (someone else's, or a copy already
    /// stored, e.g. a repeater's relay of it)
    pub fn record(&mut self, txid: u8, packet: i64, payload: &[u8]) -> bool {
        match self.txid {
            None => {
                self.txid = Some(txid);
                self.next_packet = packet;
            }
            Some(current) if current != txid => return false,
            Some(_) => {}
        }

        if packet < self.next_packet {
            return false;
        }
        for _ in self.next_packet..packet {
            self.push(self.silence);
        }
        let mut stored = [0u8; PAYLOAD_BYTES];
        stored.copy_from_slice(payload);
        self.push(stored);
        self.next_packet = packet + 1;
        true
    }

    fn push(&mut self, payload: Payload) {
        if self.packets.len() < MAX_PACKETS {
            self.packets.push(payload);
        }
    }

    /// End the recording and hand it over for replay.
    pub fn take(&mut self) -> Vec<Payload> {
        self.txid = None;
        std::mem::take(&mut self.packets)
    }
}

/// Send a recording back out at the pace it was spoken, then the end packet
/// `end`. With `wake`, a wake-up packet goes first, in the slot before the
/// audio.
pub async fn replay(packets: Vec<Payload>, txid: u8, wake: bool, end: heapless::Vec<u8, 255>) {
    Timer::after_millis(REPLAY_DELAY_MS).await;
    if wake {
        TX_CHAN.send(packet::wake(txid)).await;
    }

    let mut ticker = Ticker::every(Duration::from_millis(PACKET_MS));
    let header = packet::pack(PacketType::Echo, txid);
    for payload in &packets {
        ticker.next().await;
        let mut data = heapless::Vec::new();
        let _ = data.extend_from_slice(&header);
        let _ = data.extend_from_slice(payload);
        TX_CHAN
            .send(TxRequest {
                data,
                preamble: None,
                channel: 0,
                clear_air_first: true,
            })
            .await;
    }

    // On the beat, a slot after the last packet, like any packet: sent
    // straight after it, it was on the air while a repeater relayed that
    // last packet, and the repeater never heard it
    ticker.next().await;
    TX_CHAN
        .send(TxRequest {
            data: end,
            preamble: None,
            channel: 0,
            clear_air_first: true,
        })
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    const SILENCE: Payload = [0; PAYLOAD_BYTES];

    fn voice(byte: u8) -> Payload {
        [byte; PAYLOAD_BYTES]
    }

    #[test]
    fn a_relay_of_a_stored_packet_isnt_stored_again() {
        let mut recorder = Recorder::new(SILENCE);
        assert!(recorder.record(7, 3, &voice(1)));
        assert!(!recorder.record(7, 3, &voice(1))); // its relay
        assert!(recorder.record(7, 4, &voice(2)));
        assert_eq!(recorder.take(), vec![voice(1), voice(2)]);
    }

    #[test]
    fn a_gap_of_any_length_is_filled_with_silence() {
        // Longer than the old 4-bit seq could count: 10 packets missing
        let mut recorder = Recorder::new(SILENCE);
        recorder.record(7, 0, &voice(1));
        recorder.record(7, 11, &voice(2));
        let packets = recorder.take();
        assert_eq!(packets.len(), 12);
        assert_eq!(
            (packets[0], packets[5], packets[11]),
            (voice(1), SILENCE, voice(2))
        );
    }

    #[test]
    fn someone_else_is_ignored_until_the_recording_ends() {
        let mut recorder = Recorder::new(SILENCE);
        recorder.record(7, 0, &voice(1));
        assert!(!recorder.record(9, 1, &voice(2)));
        recorder.take();
        assert!(recorder.record(9, 1, &voice(2)));
        assert_eq!(recorder.txid(), Some(9));
    }
}
