//! The text of the radio screen's rows: 21 characters each (FONT_6X10 on
//! the 128px wide screen). Only the text: app.rs draws it.
//!
//! ```text
//! Cornelious   A2:C6:2C     us: our name, short MAC (the hard ID)
//! 40.54390,-105.09185       our position
//! Bob          1.35km W     the last transmission heard to its end: who,
//! -87dBm +13dB 1 hop          how far, how strong, through how many
//! 14:11    192.168.0.240      repeaters, and when; then WiFi (devices::network)
//! V7 RX 14:11     3.91V     volume, what we're doing, the time, battery
//!                           (with the mode first if not normal: RPT 3.9V)
//! ```
//!
//! Every field has a fixed widest form that fits: nothing comes and goes
//! with the values. The time is UTC, without a Z.

use core::fmt::Write as _;

use crate::devices::gps::{bearing_deg, compass_point, distance_m};
use crate::mode::Mode;
use crate::packet::Ident;

pub const WIDTH: usize = 21;

/// One row of text
pub type Row = heapless::String<32>;

/// What the radio is doing, for the bottom row
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Receiving,
    Transmitting,
    Repeating,
}

impl Activity {
    fn code(self) -> &'static str {
        match self {
            Activity::Idle => "  ",
            Activity::Receiving => "RX",
            Activity::Transmitting => "TX",
            Activity::Repeating => "RP",
        }
    }
}

/// The last transmission heard to its end (its end packet)
#[derive(Clone, Debug, PartialEq)]
pub struct Heard {
    pub txid: u8,
    pub ident: Ident,
    pub rssi: i16,
    /// Signal to noise, dB: below 0 is under the noise (LoRa still decodes
    /// to about -7.5 at SF7)
    pub snr: i16,
    /// Repeaters it came through (its header's hop count): 0 straight from
    /// the talker. Its RSSI is the last repeater's
    pub hops: u8,
    /// UTC hh:mm when it ended, if we knew the time
    pub at: Option<(u8, u8)>,
}

/// `left` at the left and `right` at the right of a row; `left` cut short
/// if both don't fit
pub fn spread(left: &str, right: &str) -> Row {
    if right.is_empty() {
        return left.chars().take(WIDTH).collect();
    }
    let room = WIDTH.saturating_sub(right.chars().count() + 1);
    let left: Row = left.chars().take(room).collect();
    let mut row = left.clone();
    for _ in 0..WIDTH - left.chars().count() - right.chars().count() {
        let _ = row.push(' ');
    }
    let _ = row.push_str(right);
    row
}

/// Row 1: our name and short MAC
pub fn us(name: &str, short_mac: &str) -> Row {
    spread(name, short_mac)
}

/// Rows 3 and 4: who we last heard, how far away and which way (if both of
/// us had a fix), then how strong and through how many repeaters. Widest:
/// "-120dBm -12dB 2 hops", 20
pub fn heard(heard: &Heard, our_position: Option<(f64, f64)>) -> (Row, Row) {
    // "10.5km NNW" at the widest: 10 of the row's 21, leaving the name 10
    let mut far = heapless::String::<12>::new();
    if let (Some(theirs), Some(ours)) = (heard.ident.position, our_position) {
        let m = distance_m(theirs, ours);
        let _ = match m {
            m if m < 1_000.0 => write!(far, "{:.0}m", m),
            m if m < 10_000.0 => write!(far, "{:.2}km", m / 1000.0),
            m => write!(far, "{:.1}km", m / 1000.0),
        };
        let _ = write!(far, " {}", compass_point(bearing_deg(ours, theirs)));
    }
    let first = spread(&heard.ident.name, &far);

    let mut second = Row::new();
    let _ = write!(second, "{}dBm {:+}dB", heard.rssi, heard.snr);
    let plural = if heard.hops == 1 { "" } else { "s" };
    let _ = write!(second, " {} hop{}", heard.hops, plural);
    (first, second)
}

/// Row 5: when we last heard someone (blank if no one yet), and the WiFi:
/// its address, or why there's none. Widest: "14:11 192.168.100.240", 21
pub fn heard_at_and_network(heard: Option<&Heard>, network: &str) -> Row {
    let mut when = Row::new();
    let _ = match heard.map(|heard| heard.at) {
        Some(Some((h, m))) => write!(when, "{:02}:{:02}", h, m),
        Some(None) => write!(when, "--:--"),
        None => Ok(()),
    };
    spread(&when, network)
}

/// Row 6: volume (* when locked), what we're doing, the UTC time, then the
/// mode, cut short if it doesn't fit (only "REPEATER" at volume 10 doesn't)
pub fn status(
    volume: u8,
    locked: bool,
    activity: Activity,
    time: Option<(u8, u8)>,
    mode: Mode,
    battery_mv: Option<u32>,
) -> Row {
    let mut left = Row::new();
    let lock = if locked { "*" } else { " " };
    let _ = write!(left, "V{}{}{}", volume, lock, activity.code());
    let _ = match time {
        Some((h, m)) => write!(left, " {:02}:{:02}", h, m),
        None => write!(left, " --:--"),
    };
    // The battery in volts: a LiPo's percent under load is a guess. Normal
    // mode isn't shown, so the volts get two decimals; another mode is worth
    // seeing at a glance, and takes one of them
    let mut right = Row::new();
    let _ = match (mode, battery_mv) {
        (Mode::Normal, Some(mv)) => write!(right, "{}.{:02}V", mv / 1000, mv % 1000 / 10),
        (Mode::Normal, None) => write!(right, "-.--V"),
        (Mode::Repeater, Some(mv)) => write!(right, "RPT {}.{}V", mv / 1000, mv % 1000 / 100),
        (Mode::Repeater, None) => write!(right, "RPT -.-V"),
        (Mode::Echo, Some(mv)) => write!(right, "ECH {}.{}V", mv / 1000, mv % 1000 / 100),
        (Mode::Echo, None) => write!(right, "ECH -.-V"),
    };
    spread(&left, &right)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bob(position: Option<(f64, f64)>, hops: u8) -> Heard {
        Heard {
            txid: 1,
            ident: Ident {
                name: "Bob".try_into().unwrap(),
                position,
            },
            rssi: -87,
            snr: 13,
            hops,
            at: Some((14, 11)),
        }
    }

    #[test]
    fn rows_fill_the_width() {
        assert_eq!(us("Cornelious", "A2:C6:2C"), "Cornelious   A2:C6:2C");
        assert_eq!(
            us("a-name-too-long-to-fit", "A2:C6:2C"),
            "a-name-too-l A2:C6:2C"
        );
        assert_eq!(
            status(
                7,
                false,
                Activity::Receiving,
                Some((14, 11)),
                Mode::Normal,
                Some(3_912)
            ),
            "V7 RX 14:11     3.91V"
        );
        assert_eq!(
            status(7, false, Activity::Idle, Some((14, 11)), Mode::Normal, None),
            "V7    14:11     -.--V"
        );
        assert_eq!(
            status(
                7,
                true,
                Activity::Repeating,
                None,
                Mode::Repeater,
                Some(4_187)
            ),
            "V7*RP --:--  RPT 4.1V"
        );
        assert_eq!(
            status(
                10,
                false,
                Activity::Repeating,
                Some((9, 5)),
                Mode::Echo,
                Some(3_650)
            ),
            "V10 RP 09:05 ECH 3.6V" // the widest
        );
        for row in [
            us("x", "y"),
            status(10, true, Activity::Idle, None, Mode::Echo, None),
        ] {
            assert_eq!(row.chars().count(), WIDTH);
        }
    }

    #[test]
    fn heard_shows_who_how_far_and_how() {
        let home = (40.543_9, -105.091_85);
        let far_end = (40.545_389, -105.107_722);
        let (first, second) = heard(&bob(Some(far_end), 1), Some(home));
        assert_eq!(first, "Bob          1.35km W");
        assert_eq!(second, "-87dBm +13dB 1 hop");
        let (first, second) = heard(&bob(Some((40.544_8, -105.091_85)), 0), Some(home));
        assert_eq!(first, "Bob            100m N");
        assert_eq!(second, "-87dBm +13dB 0 hops");
        // The widest it gets still fits
        let mut weak = bob(None, 2);
        weak.rssi = -120;
        weak.snr = -12;
        assert_eq!(heard(&weak, None).1, "-120dBm -12dB 2 hops");
        // The widest it gets still fits: a long name is cut short
        let mut far_away = bob(Some((0.088, -0.037)), 0);
        far_away.ident.name = "Longest-Name".try_into().unwrap();
        let (first, _) = heard(&far_away, Some((0.0, 0.0)));
        assert_eq!(first, "Longest-Na 10.6km NNW");
        assert_eq!(first.chars().count(), WIDTH);
        // Either side without a fix: no distance
        assert_eq!(heard(&bob(None, 0), Some(home)).0, "Bob");
    }

    #[test]
    fn when_heard_and_the_network_share_a_row() {
        let bob = bob(None, 0);
        assert_eq!(
            heard_at_and_network(Some(&bob), "192.168.0.240"),
            "14:11   192.168.0.240"
        );
        // The widest of both still fits
        assert_eq!(
            heard_at_and_network(Some(&bob), "192.168.100.240"),
            "14:11 192.168.100.240"
        );
        let mut no_clock = bob.clone();
        no_clock.at = None;
        assert_eq!(
            heard_at_and_network(Some(&no_clock), "WiFi off"),
            "--:--        WiFi off"
        );
        // No one heard yet: just the network
        assert_eq!(
            heard_at_and_network(None, "WiFi searching"),
            "       WiFi searching"
        );
    }
}
