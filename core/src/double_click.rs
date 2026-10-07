//! A quick double click on a push switch: press, release, press, release.
//! Opens the menu, so a single stray click (a knob bumped in a pocket)
//! doesn't.

use std::time::{Duration, Instant};

/// How quick: each press released within this, and the second press within
/// this of the first release
pub const QUICK: Duration = Duration::from_millis(400);

/// Where we are in a double click
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Idle,
    FirstDown(Instant),  // the first press, since then
    OneClick(Instant),   // one quick click done, released then
    SecondDown(Instant), // the second press, since then
}

pub struct DoubleClick {
    stage: Stage,
}

impl Default for DoubleClick {
    fn default() -> Self {
        DoubleClick { stage: Stage::Idle }
    }
}

impl DoubleClick {
    pub fn press(&mut self, at: Instant) {
        self.stage = match self.stage {
            Stage::OneClick(released) if at - released <= QUICK => Stage::SecondDown(at),
            _ => Stage::FirstDown(at),
        };
    }

    /// True when this release finishes a quick double click
    pub fn release(&mut self, at: Instant) -> bool {
        let (next, done) = match self.stage {
            Stage::FirstDown(pressed) if at - pressed <= QUICK => (Stage::OneClick(at), false),
            Stage::SecondDown(pressed) if at - pressed <= QUICK => (Stage::Idle, true),
            // Held too long, or a release with no press seen (e.g. the one
            // after the click that closed the menu)
            _ => (Stage::Idle, false),
        };
        self.stage = next;
        done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    /// Press/release times in ms from a start; true if the last release
    /// finished a double click
    fn clicks(times: &[(u64, u64)]) -> bool {
        let start = Instant::now();
        let mut double_click = DoubleClick::default();
        let mut done = false;
        for &(press, release) in times {
            double_click.press(start + ms(press));
            done = double_click.release(start + ms(release));
        }
        done
    }

    #[test]
    fn a_quick_double_click_counts() {
        assert!(clicks(&[(0, 100), (250, 350)]));
    }

    #[test]
    fn one_click_does_not() {
        assert!(!clicks(&[(0, 100)]));
    }

    #[test]
    fn too_long_between_clicks_does_not() {
        assert!(!clicks(&[(0, 100), (600, 700)]));
    }

    #[test]
    fn a_held_press_does_not() {
        assert!(!clicks(&[(0, 500), (600, 700)]));
        assert!(!clicks(&[(0, 100), (200, 700)]));
    }

    #[test]
    fn a_late_second_click_can_start_a_new_double_click() {
        assert!(clicks(&[(0, 100), (600, 700), (800, 900)]));
    }

    #[test]
    fn a_stray_release_is_ignored() {
        let start = Instant::now();
        let mut double_click = DoubleClick::default();
        assert!(!double_click.release(start));
        double_click.press(start + ms(100));
        assert!(!double_click.release(start + ms(200)));
        double_click.press(start + ms(300));
        assert!(double_click.release(start + ms(400)));
    }
}
