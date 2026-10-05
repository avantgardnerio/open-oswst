//! The ordered path from decoded packets to the speaker, for one
//! transmission at a time.
//!
//! Packets arrive in the order they were sent (each in its bin on the
//! conveyor), and the codec thread decodes them in that order. So all this
//! does is:
//! - hold the first packet until a second is decoded: a backlog of one
//!   packet, so a lost direct copy can still be covered by its relay, 80 ms
//!   later, without the speaker running dry
//! - put silence where a packet is known to be missing: a later one turned up
//! - play the squelch tail after the last decode still in flight
//!
//! Packets are counted on the conveyor (conveyor::Landing::packet), never by
//! a sequence number. Pure logic, tested here: the app passes what
//! comes out on to the speaker (SPK_AUDIO).

/// What the speaker gets next, in order
#[derive(Debug, PartialEq, Eq)]
pub enum Play<Audio> {
    Audio(Audio),
    /// A packet's worth of silence, where one was lost
    Silence,
    /// The tail after a received end packet
    Squelch,
}

/// Silences in a row we put in for one gap, at most. A transmission is over
/// after 6 lost packets anyway (conveyor::EMPTY_BINS_TO_END), and silence the
/// speaker can't hold is better left out than queued up as delay
const MOST_SILENCES: i64 = 4;

pub struct Playout<Audio> {
    /// The next packet the speaker should get; None until the first arrives
    next_packet: Option<i64>,
    /// The first packet, until a second is decoded
    held: Option<Audio>,
    started: bool,
    decodes_in_flight: u32,
    /// The end packet came while decodes were still in flight: the tail goes
    /// after them
    squelch_after_decodes: bool,
}

impl<Audio> Default for Playout<Audio> {
    fn default() -> Self {
        Playout {
            next_packet: None,
            held: None,
            started: false,
            decodes_in_flight: 0,
            squelch_after_decodes: false,
        }
    }
}

impl<Audio> Playout<Audio> {
    /// A packet went to the codec: its audio will come back, in order
    pub fn decoding(&mut self) {
        self.decodes_in_flight += 1;
    }

    /// Packet `packet`'s audio came back from the codec. `play` gets what the
    /// speaker should have next, in order
    pub fn decoded(&mut self, packet: i64, audio: Audio, mut play: impl FnMut(Play<Audio>)) {
        self.decodes_in_flight = self.decodes_in_flight.saturating_sub(1);
        let next = *self.next_packet.get_or_insert(packet);
        if packet < next {
            return; // already played past it
        }
        // Lost packets before this one: silence in their place
        let missing = (packet - next).min(MOST_SILENCES);
        if !self.started {
            if self.held.is_none() && missing == 0 {
                // The very first: hold it until a second is here
                self.held = Some(audio);
                self.next_packet = Some(packet + 1);
                return;
            }
            self.start(&mut play);
        }
        for _ in 0..missing {
            play(Play::Silence);
        }
        play(Play::Audio(audio));
        self.next_packet = Some(packet + 1);
        if self.squelch_after_decodes && self.decodes_in_flight == 0 {
            self.squelch_after_decodes = false;
            play(Play::Squelch);
        }
    }

    /// The end packet arrived: play what's held, and the tail once the
    /// decodes still in flight are done
    pub fn ended(&mut self, mut play: impl FnMut(Play<Audio>)) {
        self.start(&mut play);
        if self.decodes_in_flight == 0 {
            play(Play::Squelch);
        } else {
            self.squelch_after_decodes = true;
        }
    }

    /// A new transmission (or our own PTT, the menu, the old one timing out):
    /// forget this one. Decodes still in flight are dropped when they arrive
    /// (their txid isn't the new one's)
    pub fn reset(&mut self) {
        *self = Playout::default();
    }

    /// Start playing: whatever is held goes first
    fn start(&mut self, play: &mut impl FnMut(Play<Audio>)) {
        self.started = true;
        if let Some(audio) = self.held.take() {
            play(Play::Audio(audio));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(steps: impl FnOnce(&mut Playout<u32>, &mut Vec<Play<u32>>)) -> Vec<Play<u32>> {
        let mut playout = Playout::default();
        let mut out = Vec::new();
        steps(&mut playout, &mut out);
        out
    }

    #[test]
    fn the_first_packet_waits_for_the_second() {
        let out = run(|playout, out| {
            playout.decoding();
            playout.decoded(1, 10, |play| out.push(play));
            assert!(out.is_empty());
            playout.decoding();
            playout.decoded(2, 20, |play| out.push(play));
        });
        assert_eq!(out, vec![Play::Audio(10), Play::Audio(20)]);
    }

    #[test]
    fn a_lost_packet_becomes_silence_in_its_place() {
        let out = run(|playout, out| {
            for (packet, audio) in [(1, 10), (2, 20), (4, 40)] {
                playout.decoding();
                playout.decoded(packet, audio, |play| out.push(play));
            }
        });
        assert_eq!(
            out,
            vec![
                Play::Audio(10),
                Play::Audio(20),
                Play::Silence,
                Play::Audio(40)
            ]
        );
    }

    #[test]
    fn a_lost_second_packet_still_starts_playing() {
        let out = run(|playout, out| {
            playout.decoded(1, 10, |play| out.push(play));
            playout.decoded(3, 30, |play| out.push(play));
        });
        assert_eq!(out, vec![Play::Audio(10), Play::Silence, Play::Audio(30)]);
    }

    #[test]
    fn a_long_gap_gets_only_so_much_silence() {
        let out = run(|playout, out| {
            playout.decoded(1, 10, |play| out.push(play));
            playout.decoded(2, 20, |play| out.push(play));
            playout.decoded(12, 120, |play| out.push(play));
        });
        let silences = out.iter().filter(|play| **play == Play::Silence).count();
        assert_eq!(silences, MOST_SILENCES as usize);
        assert_eq!(out.last(), Some(&Play::Audio(120)));
    }

    #[test]
    fn the_squelch_waits_for_decodes_in_flight() {
        let out = run(|playout, out| {
            playout.decoding();
            playout.decoded(1, 10, |play| out.push(play));
            playout.decoding();
            playout.decoding();
            playout.decoded(2, 20, |play| out.push(play));
            playout.ended(|play| out.push(play));
            assert_ne!(out.last(), Some(&Play::Squelch));
            playout.decoded(3, 30, |play| out.push(play));
        });
        assert_eq!(
            out,
            vec![
                Play::Audio(10),
                Play::Audio(20),
                Play::Audio(30),
                Play::Squelch
            ]
        );
    }

    #[test]
    fn an_end_plays_whats_held_then_the_tail() {
        let out = run(|playout, out| {
            playout.decoding();
            playout.decoded(1, 10, |play| out.push(play));
            playout.ended(|play| out.push(play));
        });
        assert_eq!(out, vec![Play::Audio(10), Play::Squelch]);
    }

    #[test]
    fn an_old_packet_is_ignored() {
        let out = run(|playout, out| {
            playout.decoded(5, 50, |play| out.push(play));
            playout.decoded(6, 60, |play| out.push(play));
            playout.decoded(4, 40, |play| out.push(play));
        });
        assert_eq!(out, vec![Play::Audio(50), Play::Audio(60)]);
    }
}
