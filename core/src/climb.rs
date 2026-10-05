//! Which channels to listen on during a transmission, climbing toward its
//! strongest copy. Every packet goes out once per hop: the talker's copy,
//! then each repeater's, a bin later and a channel on. A listener keeps a
//! hold on the channel with the strongest copies so far, in that copy's
//! bins, and in the other bins it reaches, always seeking: one channel on
//! (the next repeater's copy), then one back (the hop before's), turn and
//! turn about. A copy the reach happens to decode is a bonus: a second
//! chance at that packet. A side whose copies are clearly stronger becomes
//! the hold, and the seeking goes on around it.
//!
//! The pattern repeats every 4 bins, so the radio turns through it on its
//! own (devices::radio::Rotation): nothing has to drive the seeking, and
//! it never stops.
//!
//! ```text
//! bin:    0       1       2       3       4 ...
//!         hold    on      hold    back    hold      (hold in the even bins)
//! ```
//!
//! Every packet says its hops, so the channel each hop is on follows from
//! the hold (hold channel - hold hops is the talker's). Moving the hold
//! never plays audio out of order: each copy carries its packet number, and
//! only the first copy of each number is played, in order (conveyor.rs
//! first_copy). A move one hop on delays the next packet by a bin (the
//! speaker may run dry once); one hop back brings it a bin early, so that
//! side must first have heard a packet the hold heard too: it's the same
//! transmission one hop back.
//!
//! Pure logic on what the radio heard, so it's tested here.

/// A side becomes the hold once it has heard this many packets...
const PACKETS_TO_MOVE: u8 = 3;

/// ...and its copies are this much stronger on average, dB
const STRONGER_BY_DB: i32 = 3;

/// RSSI is averaged over about this many packets: a hold that fades (a
/// walk out of range) soon loses to a side
const AVERAGE_OVER: u8 = 8;

/// The bins of one turn of the pattern: the hold's twice, each side once
pub const PATTERN_BINS: usize = 4;

pub struct Climb {
    /// How many hop channels there are (1: a radio that doesn't sweep)
    channels: u8,
    hold: Hold,
    /// What the reach has heard on each side: one channel on, one back
    sides: [Side; 2],
}

/// The channel we trust, in the bins of its copies' hops
struct Hold {
    channel: u8,
    /// The hops its copies carry: they're in the bins of this parity
    hops: u8,
    rssi: Average,
    /// The last packet heard here
    last_packet: Option<i64>,
}

/// One side of the hold, as the reach has heard it
#[derive(Default, Clone, Copy)]
struct Side {
    /// The hops its copies carry, once heard
    hops: Option<u8>,
    rssi: Average,
    /// The last packet heard here
    last_packet: Option<i64>,
    /// It has heard a packet the hold heard too
    same_packets: bool,
}

const ON: usize = 0;
const BACK: usize = 1;

/// A running mean of RSSI, dBm, over about the last AVERAGE_OVER packets
#[derive(Default, Clone, Copy)]
struct Average {
    sum: i32,
    count: u8,
}

impl Average {
    /// Past AVERAGE_OVER packets, each new one pushes out an average one
    fn add(&mut self, rssi: i16) {
        if self.count == AVERAGE_OVER {
            self.sum -= self.sum / self.count as i32;
            self.count -= 1;
        }
        self.sum += rssi as i32;
        self.count += 1;
    }

    fn mean(&self) -> Option<i32> {
        (self.count > 0).then(|| self.sum / self.count as i32)
    }
}

impl Climb {
    /// Start from the first good packet with trusted timing: `hops` on
    /// `channel` of `channels`
    pub fn start(channels: u8, channel: u8, hops: u8, rssi: i16, packet: Option<i64>) -> Climb {
        let mut rssi_average = Average::default();
        rssi_average.add(rssi);
        Climb {
            channels: channels.max(1),
            hold: Hold {
                channel,
                hops,
                rssi: rssi_average,
                last_packet: packet,
            },
            sides: [Side::default(); 2],
        }
    }

    /// The channel to listen on in `bin` (counted from the belt's bin 0): the
    /// hold's in its bins; in the others one on, then one back, in turn. A
    /// repeater listens on its hold only: in the other bins it's sending
    pub fn channel_for(&self, bin: i64, repeater: bool) -> u8 {
        match self.side_for(bin) {
            Some(side) if !repeater => self.side_channel(side),
            _ => self.hold.channel,
        }
    }

    /// A good packet with trusted timing, in `bin`: `hops` heard on
    /// `channel`. Counts for the hold or a side if it's where we were
    /// listening for them; anything else is ignored (heard on a channel we
    /// were just leaving, say). True if the hold moved: where to listen
    /// changed
    pub fn heard(
        &mut self,
        bin: i64,
        channel: u8,
        hops: u8,
        rssi: i16,
        packet: Option<i64>,
    ) -> bool {
        match self.side_for(bin) {
            None if channel == self.hold.channel => {
                self.hold.rssi.add(rssi);
                self.hold.last_packet = packet;
                for side in &mut self.sides {
                    if packet.is_some() && packet == side.last_packet {
                        side.same_packets = true;
                    }
                }
                self.climb()
            }
            Some(side) if channel == self.side_channel(side) => {
                let hold_packet = self.hold.last_packet;
                let side = &mut self.sides[side];
                side.hops = Some(hops);
                side.rssi.add(rssi);
                side.last_packet = packet;
                if packet.is_some() && packet == hold_packet {
                    side.same_packets = true;
                }
                self.climb()
            }
            _ => false,
        }
    }

    /// The channel hop `hops` is on: the hold's channel, moved by how many
    /// hops on or back from the hold's
    pub fn channel_of(&self, hops: u8) -> u8 {
        self.offset(hops as i32 - self.hold.hops as i32)
    }

    /// A side becomes the hold if it's earned it: enough packets, clearly
    /// stronger, and one back only once it's heard the same packets as the
    /// hold. True if one did
    fn climb(&mut self) -> bool {
        let Some(hold_rssi) = self.hold.rssi.mean() else {
            return false;
        };
        for side in [ON, BACK] {
            let heard = self.sides[side];
            let (Some(rssi), Some(hops)) = (heard.rssi.mean(), heard.hops) else {
                continue;
            };
            let earned = heard.rssi.count >= PACKETS_TO_MOVE
                && rssi >= hold_rssi + STRONGER_BY_DB
                && (side == ON || heard.same_packets);
            if earned {
                self.hold = Hold {
                    channel: self.side_channel(side),
                    hops,
                    rssi: heard.rssi,
                    last_packet: heard.last_packet,
                };
                self.sides = [Side::default(); 2];
                return true;
            }
        }
        false
    }

    /// Which side the reach is on in `bin`; None in the hold's bins
    fn side_for(&self, bin: i64) -> Option<usize> {
        if bin.rem_euclid(2) == (self.hold.hops % 2) as i64 {
            return None;
        }
        Some(if bin.div_euclid(2).rem_euclid(2) == 0 {
            ON
        } else {
            BACK
        })
    }

    fn side_channel(&self, side: usize) -> u8 {
        self.offset(if side == ON { 1 } else { -1 })
    }

    fn offset(&self, by: i32) -> u8 {
        (self.hold.channel as i32 + by).rem_euclid(self.channels as i32) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANNELS: u8 = 5;

    /// The hold on the talker: hops 0 on channel 2, in the even bins
    fn on_the_talker() -> Climb {
        Climb::start(CHANNELS, 2, 0, -90, Some(0))
    }

    /// The channels for bins 0..4
    fn pattern(climb: &Climb, repeater: bool) -> Vec<u8> {
        (0..4).map(|bin| climb.channel_for(bin, repeater)).collect()
    }

    #[test]
    fn it_holds_the_first_copy_and_seeks_both_ways() {
        let climb = on_the_talker();
        assert_eq!(pattern(&climb, false), [2, 3, 2, 1]);
        // And round again, before bin 0 too
        assert_eq!(climb.channel_for(5, false), 3);
        assert_eq!(climb.channel_for(-1, false), 1);
        assert_eq!(climb.channel_of(0), 2);
        assert_eq!(climb.channel_of(1), 3);
    }

    #[test]
    fn a_hold_in_the_odd_bins_seeks_in_the_even() {
        let climb = Climb::start(CHANNELS, 3, 1, -90, Some(0));
        assert_eq!(pattern(&climb, false), [4, 3, 2, 3]);
    }

    #[test]
    fn a_stronger_copy_one_on_becomes_the_hold() {
        let mut climb = on_the_talker();
        // Packet n: the talker's copy in bin 2n at -90; the repeater's in
        // bin 2n + 1 at -60, heard when the reach is one on (bins 1, 5, 9)
        let mut moved = false;
        for packet in 1..=6 {
            assert!(!moved);
            climb.heard(2 * packet, 2, 0, -90, Some(packet));
            let bin = 2 * packet + 1;
            if climb.channel_for(bin, false) == 3 {
                moved = climb.heard(bin, 3, 1, -60, Some(packet));
            }
        }
        assert!(moved);
        // Holding the repeater in the odd bins, seeking 4 and 2 in the even
        assert_eq!(pattern(&climb, false), [4, 3, 2, 3]);
        assert_eq!(climb.channel_of(0), 2); // the talker's still on 2
    }

    #[test]
    fn a_copy_only_a_little_stronger_doesnt_move_it() {
        let mut climb = on_the_talker();
        for bin in 0..40 {
            let channel = climb.channel_for(bin, false);
            let rssi = if channel == 2 { -90 } else { -88 };
            assert!(!climb.heard(bin, channel, (bin % 2) as u8, rssi, Some(bin / 2)));
        }
        assert_eq!(pattern(&climb, false), [2, 3, 2, 1]);
    }

    #[test]
    fn one_back_waits_for_the_same_packets_as_the_hold() {
        // Holding the repeater (hops 1, channel 3, odd bins); the talker's
        // copies one back (channel 2, bins 2, 6, 10...) are stronger
        let mut climb = Climb::start(CHANNELS, 3, 1, -90, None);
        // Stronger, but the hold has heard none of the same packets
        for bin in [2, 6, 10, 14] {
            assert!(!climb.heard(bin, 2, 0, -50, Some(bin / 2)));
        }
        // The hold hears packet 7 (bin 15), which the talker sent in bin 14
        assert!(climb.heard(15, 3, 1, -90, Some(7)));
        assert_eq!(climb.channel_of(0), 2);
        assert_eq!(pattern(&climb, false), [2, 3, 2, 1]);
    }

    #[test]
    fn it_never_stops_seeking_however_quiet() {
        // Nothing heard at all: the pattern is fixed, the radio turns
        // through it on its own
        let climb = on_the_talker();
        let seen: Vec<u8> = (100..108)
            .map(|bin| climb.channel_for(bin, false))
            .collect();
        assert_eq!(seen, [2, 3, 2, 1, 2, 3, 2, 1]);
    }

    #[test]
    fn a_repeater_listens_on_its_hold_only() {
        assert_eq!(pattern(&on_the_talker(), true), [2, 2, 2, 2]);
    }

    #[test]
    fn packets_where_we_werent_listening_are_ignored() {
        let mut climb = on_the_talker();
        assert!(!climb.heard(1, 1, 1, -20, Some(0))); // bin 1 seeks one on (3), not 1
        assert!(!climb.heard(2, 3, 0, -20, Some(1))); // the hold's bin, another channel
        assert_eq!(climb.sides[ON].rssi.count + climb.sides[BACK].rssi.count, 0);
    }

    #[test]
    fn with_two_channels_both_sides_are_the_other_one() {
        let climb = Climb::start(2, 0, 0, -50, Some(0));
        assert_eq!(pattern(&climb, false), [0, 1, 0, 1]);
    }

    #[test]
    fn without_sweeping_everything_is_on_one_channel() {
        let climb = Climb::start(1, 0, 2, -50, Some(0));
        assert_eq!(pattern(&climb, false), [0, 0, 0, 0]);
        assert_eq!(climb.channel_of(5), 0);
    }
}
