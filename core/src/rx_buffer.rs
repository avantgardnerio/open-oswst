//! Reorder buffer for one talker's voice packets: which to play, which to
//! drop, and what the speaker gets next. Pure logic, so it can be tested
//! without a radio or a codec.
//!
//! A packet goes through `check` (lock onto the talker, judge its seq), then
//! the app decodes it and hands the audio to `insert`. The speaker pulls with
//! `for_speaker`. Seqs are 4 bits and wrap at 16.

/// Impossible seqs in a row before we decide we've lost track and resync.
/// Fewer is almost certainly a corrupt header: at low SNR one can slip past
/// the radio's CRC check.
const RESYNC_AFTER: u8 = 3;

pub struct RxBuffer<T> {
    txid: Option<u8>,       // the talker we're locked to
    last_played: u8,        // seq most recently handed to the speaker
    slots: [Option<T>; 16], // decoded audio, by seq
    started: bool,          // speaker kicked off for this talker
    impossible_in_a_row: u8,
}

/// What `check` makes of a packet's header
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Decode it and `insert` the audio
    Take,
    /// Already played, or a duplicate: drop it
    Old(i8),
    /// Someone else, while we're locked to `locked`: drop it
    OtherTxid { locked: u8 },
    /// Too far from what we expected: probably a corrupt header. Drop it
    Corrupt(i8),
    /// The third impossible seq in a row: we've lost track. The buffer has
    /// been reset, so the next packet starts afresh
    Resync(i8),
}

/// What the speaker gets when it asks for more
#[derive(Debug, PartialEq, Eq)]
pub enum Next<T> {
    Audio(T),
    /// Receiving, but the packet for this seq isn't here
    Gap(u8),
    /// Not receiving anything
    Idle,
}

impl<T> Default for RxBuffer<T> {
    fn default() -> Self {
        RxBuffer {
            txid: None,
            last_played: 0,
            slots: Default::default(),
            started: false,
            impossible_in_a_row: 0,
        }
    }
}

impl<T> RxBuffer<T> {
    /// The talker we're locked to, if any
    pub fn txid(&self) -> Option<u8> {
        self.txid
    }

    pub fn last_played(&self) -> u8 {
        self.last_played
    }

    /// First step for a voice packet: lock onto its talker if we have none,
    /// then judge its seq against what we expect next.
    pub fn check(&mut self, txid: u8, seq: u8) -> Verdict {
        if self.txid.is_none() {
            self.txid = Some(txid);
            self.last_played = seq.wrapping_sub(1) & 0x0F;
        }
        if let Some(locked) = self.txid.filter(|&locked| locked != txid) {
            return Verdict::OtherTxid { locked };
        }

        let expected = self.last_played.wrapping_add(1) & 0x0F;
        let diff = (seq.wrapping_sub(expected) & 0x0F) as i8;
        let diff = if diff > 7 { diff - 16 } else { diff };
        if !(-2..=2).contains(&diff) {
            self.impossible_in_a_row += 1;
            if self.impossible_in_a_row < RESYNC_AFTER {
                return Verdict::Corrupt(diff);
            }
            self.reset();
            return Verdict::Resync(diff);
        }
        self.impossible_in_a_row = 0;
        if diff < 0 {
            Verdict::Old(diff)
        } else {
            Verdict::Take
        }
    }

    /// Repeater mode: the packet was relayed instead of played.
    pub fn relayed(&mut self, seq: u8) {
        self.last_played = seq;
    }

    /// Second step: the decoded audio for a packet `check` said to take.
    /// Returns audio to start the speaker with, once two in a row are here.
    pub fn insert(&mut self, txid: u8, seq: u8, audio: T) -> Option<T> {
        if self.txid != Some(txid) {
            return None;
        }
        self.slots[seq as usize] = Some(audio);

        if !self.started {
            let next = self.last_played.wrapping_add(1) & 0x0F;
            let next2 = next.wrapping_add(1) & 0x0F;
            if self.slots[next as usize].is_some() && self.slots[next2 as usize].is_some() {
                self.last_played = next;
                self.started = true;
                return self.slots[next as usize].take();
            }
        }
        None
    }

    /// The speaker wants its next packet.
    pub fn for_speaker(&mut self) -> Next<T> {
        let next = self.last_played.wrapping_add(1) & 0x0F;
        if let Some(audio) = self.slots[next as usize].take() {
            self.last_played = next;
            Next::Audio(audio)
        } else if self.txid.is_some() {
            // Gap: play silence in this seq's place and move on. Waiting for
            // it instead lets later packets pile up until they look
            // "unexpected" and reset everything. If it turns up late, it's
            // just old and gets dropped.
            self.last_played = next;
            Next::Gap(next)
        } else {
            Next::Idle
        }
    }

    /// The transmission is over (EOT, timeout, our own PTT, the menu).
    pub fn end(&mut self) {
        self.reset();
        self.started = false;
    }

    fn reset(&mut self) {
        self.txid = None;
        self.last_played = 0;
        self.impossible_in_a_row = 0;
        self.slots.iter_mut().for_each(|slot| *slot = None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TXID: u8 = 43;

    /// What came out of the speaker, one entry per 160ms packet slot
    #[derive(Debug, PartialEq, Clone, Copy)]
    enum Heard {
        Packet(u32), // which transmitted packet (counting from 0, not wrapped)
        Silence,
    }
    use Heard::{Packet, Silence};

    /// Run a transmission through the buffer the way the app does: each
    /// packet period, whatever arrived goes through check → decode → insert,
    /// then (once playing) the speaker asks for one packet. `air` is what
    /// arrived in each period: `Some((seq, packet number))`, or None if lost.
    fn play(air: &[Option<(u8, u32)>]) -> Vec<Heard> {
        let mut rx = RxBuffer::<u32>::default();
        let mut heard = Vec::new();
        let mut playing = false;
        let speaker = |rx: &mut RxBuffer<u32>, heard: &mut Vec<Heard>| match rx.for_speaker() {
            Next::Audio(n) => heard.push(Packet(n)),
            Next::Gap(_) => heard.push(Silence),
            Next::Idle => {}
        };

        for arrival in air {
            if let Some((seq, n)) = *arrival {
                if rx.check(TXID, seq) == Verdict::Take {
                    if let Some(first) = rx.insert(TXID, seq, n) {
                        heard.push(Packet(first));
                        playing = true;
                        continue; // the kick is this period's audio
                    }
                }
            }
            if playing {
                speaker(&mut rx, &mut heard);
            }
        }
        // Playback runs one packet behind, so one is left once the talker stops
        speaker(&mut rx, &mut heard);
        heard
    }

    /// Packets 0..n as sent: seq wraps at 16
    fn clean(n: u32) -> Vec<Option<(u8, u32)>> {
        (0..n).map(|i| Some(((i % 16) as u8, i))).collect()
    }

    #[test]
    fn clean_stream_plays_every_packet() {
        let heard = play(&clean(20));
        let expected: Vec<Heard> = (0..20).map(Packet).collect();
        assert_eq!(heard, expected);
    }

    /// Theory 2 from the dog walk: one lost packet should cost one 160ms gap.
    #[test]
    fn a_lost_packet_costs_one_gap() {
        let mut air = clean(20);
        air[4] = None;
        let heard = play(&air);

        let mut expected: Vec<Heard> = (0..20).map(Packet).collect();
        expected[4] = Silence;
        assert_eq!(heard, expected);
    }

    /// Genuinely losing track (here the talker's seqs jump by 8, as after a
    /// long fade) resyncs after RESYNC_AFTER impossible packets, then plays on.
    #[test]
    fn losing_track_resyncs() {
        let mut air = clean(10);
        air.extend((10..20).map(|i| Some((((i + 8) % 16) as u8, i))));
        let heard = play(&air);

        // Packets 10-12 are the impossible ones; 13 onwards plays unbroken
        let after: Vec<Heard> = (13..20).map(Packet).collect();
        assert!(
            heard.windows(after.len()).any(|w| w == after),
            "heard {:?}",
            heard
        );
    }

    /// Theory 1 from the dog walk: a packet whose header arrives corrupted
    /// (here packet 5 claims seq 12) should cost only that packet.
    #[test]
    fn a_corrupt_packet_costs_only_itself() {
        let mut air = clean(20);
        air[5] = Some((12, 5));
        let heard = play(&air);

        let mut expected: Vec<Heard> = (0..20).map(Packet).collect();
        expected[5] = Silence;
        assert_eq!(heard, expected);
    }
}
