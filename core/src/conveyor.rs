//! A transmission's timing, as a conveyor belt of bins going by, one every
//! 80 ms (air::bin_us). The talker sends packet n in bin 2n, a repeater
//! relays it in the bin after, the next repeater in the bin after that: hop
//! h of packet n is in bin 2n + h. Every packet says its hops (packet.rs).
//!
//! The belt starts from the first good packet heard, whichever it is: from
//! when that packet ended, back to when its bin started, and from its hops,
//! back to bin 0. Every later bin is counted from that one moment, never
//! from the last packet, so the small errors in each packet's timestamp
//! don't add up. Every copy of a packet, however many hops it took (even
//! round a loop of repeaters), comes out as the same packet: (bin - hops) /
//! 2. Senders put each packet in the middle of its bin, air::guard_us after
//! the bin's edge (write tight); a packet that starts within a guard of
//! there rode the belt (read loose): it's part of this transmission, even if
//! its CRC failed. One that lands between bins is someone else's, or noise. A packet whose timing the radio flags
//! as skewed (`RxPacket::timing_ok`: decoded off its channel) never sets the
//! belt's time and gets no number: its bin can't be trusted.
//!
//! Pure logic on µs timestamps (`RxPacket::end_us`), so it's tested here
//! without a radio or a clock.

use crate::air;
use crate::climb::{Climb, PATTERN_BINS};
use crate::codec::PACKET_BYTES;
use crate::devices::radio::Rotation;
use core::fmt;

/// How far from the middle of its bin a packet may start and still be on
/// the belt: the guard either side of it (7.1 ms), so it's still inside its
/// bin. Landings measured on the desk: direct ±1.4 ms, relayed +2.5..4.3 ms
fn tolerance_us() -> i64 {
    air::guard_us() as i64
}

/// Empty bins in a row that end a transmission whose end packet we missed:
/// 12, ~1 s: six of the talker's bins and their relays
const EMPTY_BINS_TO_END: i64 = 12;

/// Where a packet landed on the belt
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Landing {
    /// The nearest bin, counted from bin 0 (the talker's bin of the packet
    /// the belt started from)
    pub bin: i64,
    /// How far from the middle of that bin the packet started (its bin's
    /// edge + air::guard_us, where senders start it): + late, - early
    pub off_by_us: i64,
    /// Which of the talker's packets it is: (bin - hops) / 2. None if its
    /// bin and hops don't fit (a good packet in the wrong bin for its hops),
    /// or for a garbled packet, whose hops can't be trusted
    pub packet: Option<i64>,
}

impl Landing {
    /// Close enough to the middle of its bin to be part of this transmission
    pub fn on_belt(&self) -> bool {
        self.off_by_us.abs() <= tolerance_us()
    }
}

pub struct Conveyor {
    /// When bin 0 started (its edge), in µs since boot: the talker's bin of
    /// the first good packet heard
    bin_0_us: i64,
}

impl Conveyor {
    /// Start the belt from the first good packet heard, which ended at
    /// `end_us` after `hops` relays: it's in bin `hops`, and started a guard
    /// after that bin's edge
    pub fn start(end_us: i64, hops: u8) -> Conveyor {
        Conveyor {
            bin_0_us: end_us - packet_air_us() - guard_us() - hops as i64 * bin_us(),
        }
    }

    /// Where a packet that ended at `end_us` landed: the nearest bin, how far
    /// from the middle of that bin it started, and, given its `hops`, which
    /// packet
    pub fn landing(&self, end_us: i64, hops: Option<u8>) -> Landing {
        let since_bin_0_us = end_us - packet_air_us() - guard_us() - self.bin_0_us;
        let bin = div_round(since_bin_0_us, bin_us());
        let packet = hops.and_then(|hops| {
            let talkers_bin = bin - hops as i64;
            (talkers_bin.rem_euclid(2) == 0).then_some(talkers_bin / 2)
        });
        Landing {
            bin,
            off_by_us: since_bin_0_us - bin * bin_us(),
            packet,
        }
    }

    /// When `bin` starts (its edge), in µs since boot. Its packet goes on
    /// the air a guard later
    pub fn bin_start(&self, bin: i64) -> i64 {
        self.bin_0_us + bin * bin_us()
    }
}

/// A good packet, as the radio heard it
#[derive(Clone, Copy, Debug)]
pub struct Arrival {
    /// When it ended on the air, µs since boot (`RxPacket::end_us`)
    pub end_us: i64,
    /// Relays it has had, from its header
    pub hops: u8,
    /// Where it was heard (`RxPacket::channel`)
    pub channel: u8,
    pub rssi: i16,
    /// `RxPacket::timing_ok`
    pub timing_ok: bool,
}

/// One transmission on its conveyor: whose it is, where to listen for it
/// (climb.rs), the last bin that brought anything, and how its packets
/// landed (for the log)
pub struct Transmission {
    pub txid: u8,
    conveyor: Conveyor,
    /// How many hop channels there are (1: a radio that doesn't sweep)
    channels: u8,
    /// Which channels to listen on, from the first packet whose timing we
    /// trust: a skewed packet was decoded off its channel, so its channel
    /// says nothing
    climb: Option<Climb>,
    /// The belt's time came from a packet whose timing we trust. False while
    /// it's started from a skewed one: the first trusted packet sets it
    anchored: bool,
    last_bin: i64,
    /// The last of the talker's packets taken (first_copy), if any
    last_taken: Option<i64>,
    tally: Tally,
}

impl Transmission {
    /// The first good packet of a transmission: `txid`'s, with `channels`
    /// hop channels. Where it landed: packet 0, or no number if its timing
    /// is skewed
    pub fn start(txid: u8, channels: u8, arrival: Arrival) -> (Transmission, Landing) {
        let conveyor = Conveyor::start(arrival.end_us, arrival.hops);
        let landing = conveyor.landing(arrival.end_us, Some(arrival.hops));
        let mut transmission = Transmission {
            txid,
            conveyor,
            channels: channels.max(1),
            climb: None,
            anchored: arrival.timing_ok,
            last_bin: landing.bin,
            last_taken: None,
            tally: Tally::default(),
        };
        let landing = transmission.trusted(arrival, landing);
        (transmission, landing)
    }

    /// A good packet of this transmission (its txid says it's ours, so it
    /// counts even off the belt): where it landed. With skewed timing it
    /// keeps the transmission going but gets no number. The first trusted
    /// packet on a belt started from a skewed one sets the belt's time,
    /// keeping the bin it was nearest
    pub fn heard(&mut self, arrival: Arrival) -> Landing {
        let mut landing = self.conveyor.landing(arrival.end_us, Some(arrival.hops));
        if arrival.timing_ok && !self.anchored {
            self.conveyor.bin_0_us += landing.off_by_us;
            self.anchored = true;
            landing = self.conveyor.landing(arrival.end_us, Some(arrival.hops));
        }
        self.last_bin = self.last_bin.max(landing.bin);
        self.trusted(arrival, landing)
    }

    /// Counts a good packet in the tally, and, with its timing trusted, in
    /// the climb. A skewed one gets no number
    fn trusted(&mut self, arrival: Arrival, mut landing: Landing) -> Landing {
        if !arrival.timing_ok {
            self.tally.skewed += 1;
            landing.packet = None;
            return landing;
        }
        self.tally.good(landing, arrival.hops);
        let packet = landing.packet.filter(|_| landing.on_belt());
        match &mut self.climb {
            Some(climb) => {
                climb.heard(
                    landing.bin,
                    arrival.channel,
                    arrival.hops,
                    arrival.rssi,
                    packet,
                );
            }
            None => {
                self.climb = Some(Climb::start(
                    self.channels,
                    arrival.channel,
                    arrival.hops,
                    arrival.rssi,
                    packet,
                ))
            }
        }
        landing
    }

    /// A packet whose CRC failed. Nothing in it can be trusted, so it's ours
    /// only if it rode the belt: then the transmission is still going. True
    /// if it did. With skewed timing, or on a belt whose time isn't trusted
    /// yet, there's no telling: false
    pub fn heard_garbled(&mut self, end_us: i64, timing_ok: bool) -> bool {
        if !timing_ok || !self.anchored {
            self.tally.skewed += 1;
            return false;
        }
        let landing = self.conveyor.landing(end_us, None);
        self.tally.garbled(landing);
        if landing.on_belt() {
            self.last_bin = self.last_bin.max(landing.bin);
        }
        landing.on_belt()
    }

    /// A garbled packet worth playing in place of a silence
    /// (config::PLAY_GARBLED): which of the talker's packets it is, or None.
    /// Its header can't be trusted, so its bin says which: the talker's copy
    /// of packet n is in bin 2n, a repeater's in bin 2n + 1 (one repeater at
    /// most: with more, an odd bin could hold any of their copies). And only
    /// the last copy that can still come is played: the repeater's once one
    /// has been heard in this transmission, otherwise the talker's own. An
    /// earlier copy is left for a good one to replace. Call heard_garbled
    /// first: this only reads where it landed
    pub fn garbled_packet(&self, end_us: i64, timing_ok: bool) -> Option<i64> {
        if !timing_ok || !self.anchored {
            return None;
        }
        let landing = self.conveyor.landing(end_us, None);
        if !landing.on_belt() {
            return None;
        }
        let last_hops = if self.tally.relayed > 0 { 1 } else { 0 };
        if landing.bin.rem_euclid(2) != last_hops {
            return None;
        }
        Some((landing.bin - last_hops) / 2)
    }

    /// Where to listen, bin by bin (climb.rs): the hold's channel in its
    /// bins, seeking one on and one back in the others (a repeater: the
    /// hold's in all). None until the belt's time is trusted
    pub fn rotation(&self, repeater: bool) -> Option<Rotation> {
        let climb = self.climb.as_ref().filter(|_| self.anchored)?;
        let mut channels = [0; PATTERN_BINS];
        for (bin, channel) in channels.iter_mut().enumerate() {
            *channel = climb.channel_for(bin as i64, repeater);
        }
        Some(Rotation {
            from_us: self.conveyor.bin_start(0),
            every_us: air::bin_us(),
            channels,
        })
    }

    /// The channel hop `hops` is on, once a trusted packet has told us
    pub fn channel_of(&self, hops: u8) -> Option<u8> {
        Some(self.climb.as_ref()?.channel_of(hops))
    }

    /// EMPTY_BINS_TO_END bins have gone by empty since the last one that
    /// brought anything: a packet in the last of them would have ended by now
    pub fn over(&self, now_us: i64) -> bool {
        let last_could_end_us = self.conveyor.bin_start(self.last_bin + EMPTY_BINS_TO_END)
            + guard_us()
            + packet_air_us();
        now_us > last_could_end_us + tolerance_us()
    }

    /// Is this the first copy of the talker's `packet` (any hops)? Then it's
    /// taken: to play, or for a repeater to relay. A later copy, or a packet
    /// older than one already taken, isn't
    pub fn first_copy(&mut self, packet: i64) -> bool {
        if self.last_taken.is_some_and(|taken| packet <= taken) {
            return false;
        }
        self.last_taken = Some(packet);
        true
    }

    /// How its packets landed, for the log
    pub fn tally(&self) -> &Tally {
        &self.tally
    }
}

/// How a transmission's packets landed on the belt
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Tally {
    /// Good packets straight from the talker (0 hops)
    direct: u32,
    /// Good packets through one or more repeaters
    relayed: u32,
    /// Good packets more than a guard from the middle of their bin
    off_belt: u32,
    /// Good packets in a bin that doesn't fit their hops: no packet number
    wrong_bin: u32,
    /// CRC failed, on the belt: counted as part of the transmission
    garbled_on_belt: u32,
    /// CRC failed, between bins: ignored
    garbled_off_belt: u32,
    /// Skewed timing (decoded off its channel), good or garbled: no number,
    /// never sets the belt's time
    skewed: u32,
    /// The furthest any good packet landed from its bin's start, + or -
    worst_direct_us: i64,
    worst_relayed_us: i64,
}

impl Tally {
    fn good(&mut self, landing: Landing, hops: u8) {
        let worst = if hops == 0 {
            self.direct += 1;
            &mut self.worst_direct_us
        } else {
            self.relayed += 1;
            &mut self.worst_relayed_us
        };
        if landing.off_by_us.abs() > worst.abs() {
            *worst = landing.off_by_us;
        }
        if !landing.on_belt() {
            self.off_belt += 1;
        }
        if landing.packet.is_none() {
            self.wrong_bin += 1;
        }
    }

    fn garbled(&mut self, landing: Landing) {
        if landing.on_belt() {
            self.garbled_on_belt += 1;
        } else {
            self.garbled_off_belt += 1;
        }
    }
}

impl fmt::Display for Tally {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "direct {} (worst {:+.1}ms), relayed {} (worst {:+.1}ms), off the belt {}, \
             wrong bin for its hops {}, garbled on the belt {}, garbled between bins {}, \
             skewed {}",
            self.direct,
            self.worst_direct_us as f32 / 1000.0,
            self.relayed,
            self.worst_relayed_us as f32 / 1000.0,
            self.off_belt,
            self.wrong_bin,
            self.garbled_on_belt,
            self.garbled_off_belt,
            self.skewed
        )
    }
}

/// One bin: 80 ms
fn bin_us() -> i64 {
    air::bin_us() as i64
}

/// The spare air either side of a packet in its bin: 7.1 ms
fn guard_us() -> i64 {
    air::guard_us() as i64
}

/// A voice packet's air time, 65.8 ms. A wake-up packet takes the same (air.rs)
fn packet_air_us() -> i64 {
    air::packet_us(air::preamble_symbols(), PACKET_BYTES) as i64
}

/// `value` / `divisor` to the nearest whole number (halves round up).
/// `divisor` is positive
fn div_round(value: i64, divisor: i64) -> i64 {
    (value + divisor / 2).div_euclid(divisor)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BIN: i64 = 80_000;
    const AIR: i64 = 65_792;
    const GUARD: i64 = 7_104;
    const TOLERANCE: i64 = GUARD;
    /// Hop channels: the tests hear hop h on channel h (the talker on 0)
    const CHANNELS: u8 = 5;
    /// When the talker's first packet (the one we hear first) ends
    const FIRST_END: i64 = 5_000_000;

    /// A good packet heard at -80 dBm
    fn arrival(end_us: i64, hops: u8, channel: u8, timing_ok: bool) -> Arrival {
        Arrival {
            end_us,
            hops,
            channel,
            rssi: -80,
            timing_ok,
        }
    }

    /// When a packet in `bin` ends, `late_us` after it should (centred)
    fn ends(bin: i64, late_us: i64) -> i64 {
        FIRST_END + bin * BIN + late_us
    }

    #[test]
    fn every_hop_of_a_packet_is_the_same_packet() {
        let belt = Conveyor::start(FIRST_END, 0);
        let direct = belt.landing(ends(4, 0), Some(0));
        let relay = belt.landing(ends(5, -4_200), Some(1)); // a repeater starts ~4 ms early
        let second_relay = belt.landing(ends(6, 1_000), Some(2));
        assert_eq!((direct.bin, direct.packet), (4, Some(2)));
        assert_eq!(
            (relay.bin, relay.packet, relay.off_by_us),
            (5, Some(2), -4_200)
        );
        assert_eq!((second_relay.bin, second_relay.packet), (6, Some(2)));
        assert!(relay.on_belt());
    }

    #[test]
    fn a_copy_round_a_loop_of_repeaters_is_still_the_same_packet() {
        let mut transmission = Transmission::start(42, CHANNELS, arrival(FIRST_END, 0, 0, true)).0;
        assert!(transmission.first_copy(1)); // packet 1, direct, in bin 2
        let looped = transmission.heard(arrival(ends(5, 0), 3, 3, true)); // packet 1, three hops on
        assert_eq!(looped.packet, Some(1));
        assert!(!transmission.first_copy(1));
    }

    #[test]
    fn starting_from_a_relay_gives_the_same_belt() {
        let from_direct = Conveyor::start(FIRST_END, 0);
        let from_relay = Conveyor::start(ends(1, 0), 1);
        let from_second_relay = Conveyor::start(ends(2, 0), 2);
        assert_eq!(from_direct.bin_start(7), from_relay.bin_start(7));
        assert_eq!(from_direct.bin_start(7), from_second_relay.bin_start(7));
    }

    #[test]
    fn a_packet_in_the_wrong_bin_for_its_hops_has_no_number() {
        let mut transmission = Transmission::start(42, CHANNELS, arrival(FIRST_END, 0, 0, true)).0;
        let landing = transmission.heard(arrival(ends(3, 0), 0, 0, true)); // direct, but in an odd bin
        assert_eq!(landing.packet, None);
        assert_eq!(transmission.tally().wrong_bin, 1);
    }

    #[test]
    fn a_skewed_packet_never_sets_the_belts_time() {
        // The first good packet was decoded off its channel: it ended 28 ms
        // late (walk of the desk, 2026-10-05) and gets no number
        let (mut transmission, first) =
            Transmission::start(42, CHANNELS, arrival(ends(1, 28_000), 1, 1, false));
        assert_eq!(first.packet, None);
        // Garbled packets can't be placed on a belt whose time isn't trusted
        assert!(!transmission.heard_garbled(ends(2, 0), true));
        // The first trusted packet sets the belt's time
        let trusted = transmission.heard(arrival(ends(2, 0), 0, 0, true));
        assert_eq!(
            (trusted.bin, trusted.off_by_us, trusted.packet),
            (2, 0, Some(1))
        );
        let next = transmission.heard(arrival(ends(4, 500), 0, 0, true));
        assert_eq!((next.packet, next.off_by_us), (Some(2), 500));
        // A later skewed one still gets no number, and moves nothing
        let skewed = transmission.heard(arrival(ends(6, -20_000), 0, 0, false));
        assert_eq!(skewed.packet, None);
        assert_eq!(transmission.tally().skewed, 3);
        assert_eq!(
            transmission
                .heard(arrival(ends(8, 0), 0, 0, true))
                .off_by_us,
            0
        );
    }

    #[test]
    fn packets_count_on_past_where_seq_used_to_wrap() {
        let belt = Conveyor::start(FIRST_END, 0);
        let late = belt.landing(ends(2 * 40, 2_000), Some(0));
        assert_eq!((late.bin, late.packet), (80, Some(40)));
    }

    #[test]
    fn between_bins_is_off_the_belt() {
        let belt = Conveyor::start(FIRST_END, 0);
        assert!(belt.landing(ends(3, TOLERANCE), None).on_belt());
        let between = belt.landing(ends(3, TOLERANCE + 1), None);
        assert_eq!(between.bin, 3);
        assert!(!between.on_belt());
        let halfway = belt.landing(ends(3, BIN / 2 - 1), None);
        assert_eq!(halfway.bin, 3); // still nearest bin 3, just off the belt
    }

    #[test]
    fn bin_start_is_a_guard_before_its_packet_goes_on_the_air() {
        let belt = Conveyor::start(FIRST_END, 0);
        assert_eq!(belt.bin_start(0), FIRST_END - AIR - GUARD);
        assert_eq!(belt.bin_start(5), FIRST_END - AIR - GUARD + 5 * BIN);
    }

    #[test]
    fn the_base_channel_comes_from_the_first_trusted_packet() {
        // Heard first through two repeaters, on channel 1: the talker is on
        // channel 4 (of 5), its first relay on 0
        let (transmission, _) = Transmission::start(42, CHANNELS, arrival(FIRST_END, 2, 1, true));
        assert_eq!(transmission.channel_of(0), Some(4));
        assert_eq!(transmission.channel_of(1), Some(0));
        assert_eq!(transmission.channel_of(2), Some(1));
    }

    #[test]
    fn a_skewed_packet_says_nothing_about_channels() {
        // Direct, but decoded on the next channel at point-blank range
        let (mut transmission, _) =
            Transmission::start(42, CHANNELS, arrival(ends(0, 28_000), 0, 1, false));
        assert_eq!(transmission.channel_of(0), None);
        transmission.heard(arrival(ends(1, 0), 1, 1, true));
        assert_eq!(transmission.channel_of(0), Some(0));
    }

    #[test]
    fn the_rotation_holds_the_first_copy_and_seeks_both_ways() {
        // Heard the talker first (on channel 2): its packets in the even
        // bins, seeking the first repeater's relays (channel 3) and one
        // back (1) in the odd ones
        let direct = Transmission::start(42, CHANNELS, arrival(FIRST_END, 0, 2, true)).0;
        let rotation = direct.rotation(false).unwrap();
        assert_eq!(rotation.channels, [2, 3, 2, 1]);
        assert_eq!(rotation.every_us, BIN as u32);
        // Turns start at the bins' edges, a guard before their packets
        assert_eq!(rotation.from_us, FIRST_END - AIR - GUARD);
        assert_eq!(rotation.channel_at(ends(4, 0) - 1), 2);
        assert_eq!(rotation.channel_at(ends(5, 0) - 1), 3);
        // Heard a second repeater first (hops 2, channel 4): its relays in
        // the even bins, seeking one on (channel 0, round the 5) and one back (3) in the odd
        let far = Transmission::start(42, CHANNELS, arrival(FIRST_END, 2, 4, true)).0;
        assert_eq!(far.rotation(false).unwrap().channels, [4, 0, 4, 3]);
        // Heard the first repeater first: its relays in the odd bins,
        // seeking one on (4) and one back (2) in the even
        let relayed = Transmission::start(42, CHANNELS, arrival(FIRST_END, 1, 3, true)).0;
        assert_eq!(relayed.rotation(false).unwrap().channels, [4, 3, 2, 3]);
        // The same belt, whichever copy started it
        assert_eq!(
            relayed.rotation(false).unwrap().from_us,
            Transmission::start(42, CHANNELS, arrival(ends(-1, 0), 0, 2, true))
                .0
                .rotation(false)
                .unwrap()
                .from_us
        );
    }

    #[test]
    fn a_repeater_listens_for_its_primary_only() {
        let transmission = Transmission::start(42, CHANNELS, arrival(FIRST_END, 0, 2, true)).0;
        let rotation = transmission.rotation(true).unwrap();
        assert_eq!(rotation.channels, [2, 2, 2, 2]);
        assert!(rotation.stays());
    }

    #[test]
    fn no_rotation_until_the_belts_time_is_trusted() {
        let (mut transmission, _) =
            Transmission::start(42, CHANNELS, arrival(ends(0, 28_000), 0, 1, false));
        assert_eq!(transmission.rotation(false), None);
        // Holding the relay on channel 1 in the odd bins, seeking 2 and 0
        transmission.heard(arrival(ends(1, 0), 1, 1, true));
        assert_eq!(transmission.rotation(false).unwrap().channels, [2, 1, 0, 1]);
    }

    #[test]
    fn without_sweeping_everything_is_on_one_channel() {
        let (transmission, _) = Transmission::start(42, 1, arrival(FIRST_END, 2, 0, true));
        assert_eq!(transmission.channel_of(0), Some(0));
        assert_eq!(transmission.channel_of(3), Some(0));
    }

    #[test]
    fn a_garbled_packet_plays_only_as_the_last_copy_that_can_come() {
        let (mut transmission, _) = Transmission::start(7, 1, arrival(ends(0, 0), 0, 0, true));
        // No repeater heard: the talker's copy is the last, in even bins
        assert_eq!(transmission.garbled_packet(ends(4, 2_000), true), Some(2));
        assert_eq!(transmission.garbled_packet(ends(5, 0), true), None);
        // Off the belt, or timing not to be trusted: no telling
        assert_eq!(transmission.garbled_packet(ends(4, 30_000), true), None);
        assert_eq!(transmission.garbled_packet(ends(4, 0), false), None);
        // Once a repeater's copy is heard, its odd bins hold the last copies
        transmission.heard(arrival(ends(7, 0), 1, 0, true));
        assert_eq!(transmission.garbled_packet(ends(8, 0), true), None);
        assert_eq!(transmission.garbled_packet(ends(9, 0), true), Some(4));
    }

    #[test]
    fn garbled_packets_on_the_belt_keep_a_transmission_going() {
        let mut transmission = Transmission::start(42, CHANNELS, arrival(FIRST_END, 0, 0, true)).0;
        assert!(transmission.heard_garbled(ends(2, 3_000), true));
        assert!(!transmission.heard_garbled(ends(4, 30_000), true)); // between bins: noise
        assert_eq!(transmission.last_bin, 2);
    }

    #[test]
    fn good_packets_count_even_off_the_belt() {
        let mut transmission = Transmission::start(42, CHANNELS, arrival(FIRST_END, 0, 0, true)).0;
        let landing = transmission.heard(arrival(ends(6, 25_000), 0, 0, true));
        assert!(!landing.on_belt());
        assert_eq!(transmission.last_bin, 6);
        assert_eq!(transmission.tally().off_belt, 1);
    }

    #[test]
    fn a_late_copy_doesnt_wind_the_belt_back() {
        let mut transmission = Transmission::start(42, CHANNELS, arrival(FIRST_END, 0, 0, true)).0;
        transmission.heard(arrival(ends(8, 0), 0, 0, true));
        transmission.heard(arrival(ends(5, 0), 1, 1, true));
        assert_eq!(transmission.last_bin, 8);
    }

    #[test]
    fn moving_between_copies_never_plays_out_of_order() {
        let mut transmission = Transmission::start(42, CHANNELS, arrival(ends(1, 0), 1, 1, true)).0;
        let mut taken = Vec::new();
        let mut take = |transmission: &mut Transmission, bin: i64, hops: u8| {
            let landing = transmission.heard(arrival(ends(bin, 0), hops, hops, true));
            if transmission.first_copy(landing.packet.unwrap()) {
                taken.push(landing.packet.unwrap());
            }
        };
        // On the relay (hops 1): packets 1 and 2
        take(&mut transmission, 3, 1);
        take(&mut transmission, 5, 1);
        // Moved one hop back, to the talker: packet 3 a bin early, then its
        // relay is a copy
        take(&mut transmission, 6, 0);
        take(&mut transmission, 7, 1);
        // Moved one hop on again: packet 4's talker copy missed, its relay
        // a bin later; then a late talker copy of 4 turns up
        take(&mut transmission, 9, 1);
        take(&mut transmission, 8, 0);
        take(&mut transmission, 11, 1);
        assert_eq!(taken, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn only_the_first_copy_of_a_packet_is_taken() {
        let mut transmission = Transmission::start(42, CHANNELS, arrival(FIRST_END, 0, 0, true)).0;
        assert!(transmission.first_copy(3));
        assert!(!transmission.first_copy(3)); // its relay
        assert!(transmission.first_copy(5)); // 4 lost
        assert!(!transmission.first_copy(4)); // late: 5 is already taken
    }

    #[test]
    fn over_after_twelve_empty_bins() {
        let mut transmission = Transmission::start(42, CHANNELS, arrival(FIRST_END, 0, 0, true)).0;
        transmission.heard(arrival(ends(10, 0), 0, 0, true));
        // A packet in bin 22 would have ended by ends(22, 0), give or take
        // the tolerance; until then bin 22 might still bring one
        assert!(!transmission.over(ends(22, TOLERANCE)));
        assert!(transmission.over(ends(22, TOLERANCE + 1)));
        // Another packet pushes the end out again
        transmission.heard_garbled(ends(14, 0), true);
        assert!(!transmission.over(ends(22, TOLERANCE + 1)));
    }

    #[test]
    fn the_tally_keeps_the_worst_landing_per_path() {
        let mut transmission = Transmission::start(42, CHANNELS, arrival(FIRST_END, 0, 0, true)).0;
        transmission.heard(arrival(ends(1, -4_000), 1, 1, true));
        transmission.heard(arrival(ends(2, 3_000), 0, 0, true));
        transmission.heard(arrival(ends(3, -6_000), 1, 1, true));
        transmission.heard(arrival(ends(4, -1_000), 0, 0, true));
        let tally = transmission.tally();
        assert_eq!((tally.direct, tally.relayed), (3, 2));
        assert_eq!(
            (tally.worst_direct_us, tally.worst_relayed_us),
            (3_000, -6_000)
        );
    }
}
