//! Reorder buffer for one talker's voice packets: which to play, which to
//! drop, and what the speaker gets next. Pure logic, so it can be tested
//! without a radio or a codec.
//!
//! A packet goes through `check` (lock onto the talker, judge its seq), then
//! the app decodes it and hands the audio to `insert`. The speaker pulls with
//! `for_speaker`. Seqs are 4 bits and wrap at 16.

pub struct RxBuffer<T> {
    txid: Option<u8>,       // the talker we're locked to
    last_played: u8,        // seq most recently handed to the speaker
    slots: [Option<T>; 16], // decoded audio, by seq
    started: bool,          // speaker kicked off for this talker
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
    /// Too far from what we expected. The buffer has been reset
    Unexpected(i8),
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
        match diff {
            -2..=-1 => Verdict::Old(diff),
            0..=2 => Verdict::Take,
            _ => {
                self.reset();
                Verdict::Unexpected(diff)
            }
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
            // Gap: play silence for this seq
            // self.last_played = next;
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
