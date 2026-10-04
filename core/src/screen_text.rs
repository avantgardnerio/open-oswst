//! The text of the radio screen's rows: 21 characters each (FONT_6X10 on
//! the 128px wide screen). Only the text: app.rs draws it.
//!
//! ```text
//! Cornelious   A2:C6:2C     us: our name, short MAC (the hard ID)
//! 40.54390,-105.09185       our position
//! Bob            1.24km     the last transmission heard to its end:
//! 14:11Z -87dBm via rpt       who, how far, when, how strong, how
//! koldendeco      .0.240    WiFi (devices::network)
//! V7 RX  14:11Z   NORMAL    volume, what we're doing, the time, mode
//! ```

use core::fmt::Write as _;

use crate::devices::gps::distance_m;
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
    /// Through a repeater: it came in on another channel than the start
    /// slot, where talkers send (tx_hops = 0). Its RSSI is the repeater's
    pub relayed: bool,
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

/// Rows 3 and 4: who we last heard, how far away (if both of us had a
/// fix), then when, how strong, and whether through a repeater
pub fn heard(heard: &Heard, our_position: Option<(f64, f64)>) -> (Row, Row) {
    let mut far = heapless::String::<8>::new();
    if let (Some(theirs), Some(ours)) = (heard.ident.position, our_position) {
        let m = distance_m(theirs, ours);
        let _ = match m {
            m if m < 1_000.0 => write!(far, "{:.0}m", m),
            m if m < 10_000.0 => write!(far, "{:.2}km", m / 1000.0),
            m => write!(far, "{:.1}km", m / 1000.0),
        };
    }
    let first = spread(&heard.ident.name, &far);

    let mut second = Row::new();
    let _ = match heard.at {
        Some((h, m)) => write!(second, "{:02}:{:02}Z", h, m),
        None => write!(second, "--:--Z"),
    };
    let _ = write!(second, " {}dBm", heard.rssi);
    if heard.relayed {
        let _ = write!(second, " via rpt");
    }
    (first, second)
}

/// Row 6: volume (* when locked), what we're doing, the UTC time, then the
/// mode, cut short if it doesn't fit (only "REPEATER" at volume 10 doesn't)
pub fn status(
    volume: u8,
    locked: bool,
    activity: Activity,
    time: Option<(u8, u8)>,
    mode: &str,
) -> Row {
    let mut left = Row::new();
    let lock = if locked { "*" } else { " " };
    let _ = write!(left, "V{}{}{}", volume, lock, activity.code());
    let _ = match time {
        Some((h, m)) => write!(left, " {:02}:{:02}Z", h, m),
        None => write!(left, " --:--Z"),
    };
    let room = WIDTH.saturating_sub(left.chars().count() + 1);
    let mode: heapless::String<12> = mode
        .chars()
        .take(room)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    spread(&left, &mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bob(position: Option<(f64, f64)>, relayed: bool) -> Heard {
        Heard {
            txid: 1,
            ident: Ident {
                name: "Bob".try_into().unwrap(),
                position,
            },
            rssi: -87,
            relayed,
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
            status(7, false, Activity::Receiving, Some((14, 11)), "normal"),
            "V7 RX 14:11Z   NORMAL"
        );
        assert_eq!(
            status(7, true, Activity::Repeating, None, "repeater"),
            "V7*RP --:--Z REPEATER"
        );
        assert_eq!(
            status(10, false, Activity::Repeating, Some((9, 5)), "repeater"),
            "V10 RP 09:05Z REPEATE"
        );
        for row in [us("x", "y"), status(10, true, Activity::Idle, None, "echo")] {
            assert_eq!(row.chars().count(), WIDTH);
        }
    }

    #[test]
    fn heard_shows_who_how_far_and_how() {
        let home = (40.543_9, -105.091_85);
        let far_end = (40.545_389, -105.107_722);
        let (first, second) = heard(&bob(Some(far_end), true), Some(home));
        assert_eq!(first, "Bob            1.35km");
        assert_eq!(second, "14:11Z -87dBm via rpt");
        let (first, second) = heard(&bob(Some((40.544_8, -105.091_85)), false), Some(home));
        assert_eq!(first, "Bob              100m");
        assert_eq!(second, "14:11Z -87dBm");
        // Either side without a fix: no distance
        assert_eq!(heard(&bob(None, false), Some(home)).0, "Bob");
    }
}
