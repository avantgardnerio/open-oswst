//! A transmission's timing, as a conveyor belt of bins going by, one every
//! 80 ms (air::bin_us). The talker sends in the even bins, and a repeater
//! relays each packet in the odd bin after it.
//!
//! The belt starts from the first good packet heard, whichever it is: from
//! when that packet ended, back to when its bin started. Every later bin is
//! counted from that one moment, never from the last packet, so the small
//! errors in each packet's timestamp don't add up. A packet that lands near
//! the start of a bin rode the belt: it's part of this transmission, even
//! if its CRC failed. One that lands between bins is someone else's, or
//! noise.
//!
//! Pure logic on µs timestamps (`RxPacket::end_us`), so it's tested here
//! without a radio or a clock.

use crate::air;
use crate::codec::PACKET_BYTES;
use core::fmt;

/// How far from its bin's start a packet may land and still be on the belt.
/// Wide for now: packet times are read when the radio task wakes, not in its
/// interrupt, and a repeater starts its relay ~4 ms before its bin
const TOLERANCE_US: i64 = 10_000;

/// Empty bins in a row that end a transmission whose end packet we missed:
/// 12, ~1 s: six of the talker's bins and their relays
const EMPTY_BINS_TO_END: i64 = 12;

/// Which copy of a packet: the talker's own, or a repeater's relay of it
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Path {
    /// From the talker, in an even bin
    Direct,
    /// A repeater's relay, in the odd bin after the talker's
    Relayed,
}

/// Where a packet landed on the belt
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Landing {
    /// The nearest bin, counted from bin 0 (where the belt started)
    pub bin: i64,
    /// How far from that bin's start the packet started: + late, - early
    pub off_by_us: i64,
}

impl Landing {
    /// Even bins are the talker's, odd ones a repeater's
    pub fn path(&self) -> Path {
        if self.bin.rem_euclid(2) == 0 {
            Path::Direct
        } else {
            Path::Relayed
        }
    }

    /// Close enough to its bin's start to be part of this transmission
    pub fn on_belt(&self) -> bool {
        self.off_by_us.abs() <= TOLERANCE_US
    }

    /// Which of the talker's packets this is, counted from the belt's start:
    /// a packet and its relay (bins 2n and 2n + 1) are both packet n
    pub fn packet(&self) -> i64 {
        self.bin.div_euclid(2)
    }
}

pub struct Conveyor {
    /// When bin 0 started, in µs since boot: the talker's bin of the first
    /// good packet heard (the one before it, if that packet was a relay)
    bin_0_us: i64,
}

impl Conveyor {
    /// Start the belt from the first good packet heard, which ended at
    /// `end_us`. A direct packet is in bin 0; a relayed one in bin 1, after
    /// its talker's bin 0
    pub fn start(end_us: i64, path: Path) -> Conveyor {
        let bin = match path {
            Path::Direct => 0,
            Path::Relayed => 1,
        };
        Conveyor {
            bin_0_us: end_us - packet_air_us() - bin * bin_us(),
        }
    }

    /// Where a packet that ended at `end_us` landed: the nearest bin, and how
    /// far from that bin's start it started
    pub fn landing(&self, end_us: i64) -> Landing {
        let since_bin_0_us = end_us - packet_air_us() - self.bin_0_us;
        let bin = div_round(since_bin_0_us, bin_us());
        Landing {
            bin,
            off_by_us: since_bin_0_us - bin * bin_us(),
        }
    }

    /// When `bin` starts, in µs since boot: when its packet goes on the air
    pub fn bin_start(&self, bin: i64) -> i64 {
        self.bin_0_us + bin * bin_us()
    }
}

/// One transmission on its conveyor: whose it is, the last bin that brought
/// anything, and how its packets landed (for the log)
pub struct Transmission {
    pub txid: u8,
    conveyor: Conveyor,
    last_bin: i64,
    tally: Tally,
}

impl Transmission {
    /// The first good packet of a transmission: `txid`'s, ended at `end_us`
    pub fn start(txid: u8, end_us: i64, path: Path) -> Transmission {
        let conveyor = Conveyor::start(end_us, path);
        let landing = conveyor.landing(end_us);
        let mut transmission = Transmission {
            txid,
            conveyor,
            last_bin: landing.bin,
            tally: Tally::default(),
        };
        transmission.tally.good(landing);
        transmission
    }

    /// Where a packet that ended at `end_us` landed, without counting it
    pub fn landing(&self, end_us: i64) -> Landing {
        self.conveyor.landing(end_us)
    }

    /// A good packet of this transmission (its txid says it's ours, so it
    /// counts even off the belt): where it landed
    pub fn heard(&mut self, end_us: i64) -> Landing {
        let landing = self.conveyor.landing(end_us);
        self.last_bin = self.last_bin.max(landing.bin);
        self.tally.good(landing);
        landing
    }

    /// A packet whose CRC failed. Nothing in it can be trusted, so it's ours
    /// only if it rode the belt: then the transmission is still going. True
    /// if it did
    pub fn heard_garbled(&mut self, end_us: i64) -> bool {
        let landing = self.conveyor.landing(end_us);
        self.tally.garbled(landing);
        if landing.on_belt() {
            self.last_bin = self.last_bin.max(landing.bin);
        }
        landing.on_belt()
    }

    /// EMPTY_BINS_TO_END bins have gone by empty since the last one that
    /// brought anything: a packet in the last of them would have ended by now
    pub fn over(&self, now_us: i64) -> bool {
        let last_could_end_us =
            self.conveyor.bin_start(self.last_bin + EMPTY_BINS_TO_END) + packet_air_us();
        now_us > last_could_end_us + TOLERANCE_US
    }

    /// How its packets landed, for the log
    pub fn tally(&self) -> &Tally {
        &self.tally
    }
}

/// How a transmission's packets landed on the belt
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Tally {
    direct: u32,
    relayed: u32,
    /// Good packets more than TOLERANCE_US from their bin's start
    off_belt: u32,
    /// CRC failed, on the belt: counted as part of the transmission
    garbled_on_belt: u32,
    /// CRC failed, between bins: ignored
    garbled_off_belt: u32,
    /// The furthest any good packet landed from its bin's start, + or -
    worst_direct_us: i64,
    worst_relayed_us: i64,
}

impl Tally {
    fn good(&mut self, landing: Landing) {
        let worst = match landing.path() {
            Path::Direct => {
                self.direct += 1;
                &mut self.worst_direct_us
            }
            Path::Relayed => {
                self.relayed += 1;
                &mut self.worst_relayed_us
            }
        };
        if landing.off_by_us.abs() > worst.abs() {
            *worst = landing.off_by_us;
        }
        if !landing.on_belt() {
            self.off_belt += 1;
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
             garbled on the belt {}, garbled between bins {}",
            self.direct,
            self.worst_direct_us as f32 / 1000.0,
            self.relayed,
            self.worst_relayed_us as f32 / 1000.0,
            self.off_belt,
            self.garbled_on_belt,
            self.garbled_off_belt
        )
    }
}

/// One bin: 80 ms
fn bin_us() -> i64 {
    air::bin_us() as i64
}

/// A voice packet's air time, 65.8 ms. A wake-up packet takes the same (air.rs)
fn packet_air_us() -> i64 {
    air::packet_us(air::PREAMBLE_SYMBOLS, PACKET_BYTES) as i64
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
    /// When the talker's first packet (the one we hear first) ends
    const FIRST_END: i64 = 5_000_000;

    /// When a packet in `bin` ends, `late_us` after its bin's start
    fn ends(bin: i64, late_us: i64) -> i64 {
        FIRST_END + bin * BIN + late_us
    }

    #[test]
    fn a_relay_lands_in_the_odd_bin_after_its_talkers() {
        let belt = Conveyor::start(FIRST_END, Path::Direct);
        assert_eq!(
            belt.landing(ends(0, 0)),
            Landing {
                bin: 0,
                off_by_us: 0
            }
        );
        let relay = belt.landing(ends(1, -4_200)); // a repeater starts ~4 ms early
        assert_eq!(
            relay,
            Landing {
                bin: 1,
                off_by_us: -4_200
            }
        );
        assert_eq!(relay.path(), Path::Relayed);
        assert_eq!(relay.packet(), 0); // packet 0's relay
        assert!(relay.on_belt());
        let next = belt.landing(ends(2, 1_500));
        assert_eq!((next.bin, next.path(), next.packet()), (2, Path::Direct, 1));
    }

    #[test]
    fn starting_from_a_relay_gives_the_same_belt() {
        let from_direct = Conveyor::start(FIRST_END, Path::Direct);
        let from_relay = Conveyor::start(ends(1, 0), Path::Relayed);
        assert_eq!(from_direct.bin_start(7), from_relay.bin_start(7));
        assert_eq!(from_relay.landing(ends(40, 0)).bin, 40);
    }

    #[test]
    fn bins_count_on_past_the_seq_wrap() {
        // seq is 4 bits and wraps every 16 packets; bins don't
        let belt = Conveyor::start(FIRST_END, Path::Direct);
        let late = belt.landing(ends(2 * 40, 2_000));
        assert_eq!((late.bin, late.packet()), (80, 40));
    }

    #[test]
    fn between_bins_is_off_the_belt() {
        let belt = Conveyor::start(FIRST_END, Path::Direct);
        assert!(belt.landing(ends(3, TOLERANCE_US)).on_belt());
        let between = belt.landing(ends(3, TOLERANCE_US + 1));
        assert_eq!(between.bin, 3);
        assert!(!between.on_belt());
        let halfway = belt.landing(ends(3, BIN / 2 - 1));
        assert_eq!(halfway.bin, 3); // still nearest bin 3, just off the belt
    }

    #[test]
    fn bin_start_is_when_its_packet_goes_on_the_air() {
        let belt = Conveyor::start(FIRST_END, Path::Direct);
        assert_eq!(belt.bin_start(0), FIRST_END - AIR);
        assert_eq!(belt.bin_start(5), FIRST_END - AIR + 5 * BIN);
    }

    #[test]
    fn garbled_packets_on_the_belt_keep_a_transmission_going() {
        let mut transmission = Transmission::start(42, FIRST_END, Path::Direct);
        assert!(transmission.heard_garbled(ends(2, 3_000)));
        assert!(!transmission.heard_garbled(ends(4, 30_000))); // between bins: noise
        assert_eq!(transmission.last_bin, 2);
    }

    #[test]
    fn good_packets_count_even_off_the_belt() {
        let mut transmission = Transmission::start(42, FIRST_END, Path::Direct);
        let landing = transmission.heard(ends(6, 25_000));
        assert!(!landing.on_belt());
        assert_eq!(transmission.last_bin, 6);
        assert_eq!(transmission.tally().off_belt, 1);
    }

    #[test]
    fn a_late_copy_doesnt_wind_the_belt_back() {
        let mut transmission = Transmission::start(42, FIRST_END, Path::Direct);
        transmission.heard(ends(8, 0));
        transmission.heard(ends(5, 0));
        assert_eq!(transmission.last_bin, 8);
    }

    #[test]
    fn over_after_twelve_empty_bins() {
        let mut transmission = Transmission::start(42, FIRST_END, Path::Direct);
        transmission.heard(ends(10, 0));
        // A packet in bin 22 would have ended by ends(22, 0), give or take
        // the tolerance; until then bin 22 might still bring one
        assert!(!transmission.over(ends(22, TOLERANCE_US)));
        assert!(transmission.over(ends(22, TOLERANCE_US + 1)));
        // Another packet pushes the end out again
        transmission.heard_garbled(ends(14, 0));
        assert!(!transmission.over(ends(22, TOLERANCE_US + 1)));
    }

    #[test]
    fn the_tally_keeps_the_worst_landing_per_path() {
        let mut transmission = Transmission::start(42, FIRST_END, Path::Direct);
        transmission.heard(ends(1, -4_000));
        transmission.heard(ends(2, 3_000));
        transmission.heard(ends(3, -6_000));
        transmission.heard(ends(4, -1_000));
        let tally = transmission.tally();
        assert_eq!((tally.direct, tally.relayed), (3, 2));
        assert_eq!(
            (tally.worst_direct_us, tally.worst_relayed_us),
            (3_000, -6_000)
        );
    }
}
