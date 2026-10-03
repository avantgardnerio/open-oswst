//! Bursts of interference on our channel: how long, how often, how strong.
//! The radio reads its RSSI every millisecond while the air is clear and
//! feeds it here; every 10 s it logs a summary and starts a new window.
//!
//! This is the input the FEC simulation needs (core/examples/fec_sim): short
//! bursts inside one packet are what FEC can fix, long ones are not.
//!
//! - The floor is the 10th percentile of the previous window's readings, so
//!   bursts don't drag it up. The first window only learns it.
//! - A burst is a run of readings at or above floor + threshold, at three
//!   thresholds. +6 dB already costs an edge packet; +20 dB kills any packet
//!   that isn't strong.
//! - A packet on the air reads as a burst too, until the radio detects its
//!   preamble (~8 ms in). The radio then calls `interrupted`, which drops the
//!   run in progress. It also drops a real burst that the radio took for a
//!   preamble: `dropped` counts them.

use core::fmt;

/// How often the radio reads its RSSI
pub const SAMPLE_EVERY_MS: u64 = 1;

/// dB over the floor that counts as a burst
const THRESHOLDS_DB: [i16; 3] = [6, 10, 20];

/// Burst lengths in ms: 1, 2-3, 4-7, 8-15, 16-31, 32-63, 64+
const LENGTH_BUCKETS: usize = 7;
/// Quiet gaps between bursts in ms: under 10, 10-99, 100-999, 1000+
const GAP_BUCKETS: usize = 4;

/// The SX1262 reads RSSI from 0 down to -127.5 dBm
const LOWEST_DBM: i16 = -128;
const LEVELS: usize = (-LOWEST_DBM) as usize + 1;

pub struct NoiseMeter {
    /// The previous window's 10th percentile. None until one window is in
    floor: Option<i16>,
    /// This window's readings, counted by dBm: the next window's floor
    levels: [u32; LEVELS],
    samples: u32,
    dropped: u32,
    bursts: [Bursts; THRESHOLDS_DB.len()],
}

/// Bursts at one threshold. A burst still going at the end of a window
/// carries over: its time is busy time in each window it covers, but it
/// counts (and has a length) in the window where it ends
#[derive(Default)]
struct Bursts {
    /// ms above the threshold so far; 0 = not in a burst
    run: u32,
    run_peak: i16,
    /// ms since the last burst ended. None before the first one, and after a
    /// packet (a gap with a packet in it isn't a quiet gap)
    quiet: Option<u32>,
    count: u32,
    busy_ms: u32,
    peak: Option<i16>,
    lengths: [u32; LENGTH_BUCKETS],
    gaps: [u32; GAP_BUCKETS],
}

impl Default for NoiseMeter {
    fn default() -> Self {
        NoiseMeter {
            floor: None,
            levels: [0; LEVELS],
            samples: 0,
            dropped: 0,
            bursts: Default::default(),
        }
    }
}

impl NoiseMeter {
    /// One RSSI reading, taken SAMPLE_EVERY_MS after the last
    pub fn sample(&mut self, dbm: i16) {
        let dbm = dbm.clamp(LOWEST_DBM, 0);
        self.levels[(dbm - LOWEST_DBM) as usize] += 1;
        self.samples += 1;
        let Some(floor) = self.floor else {
            return;
        };
        for (bursts, threshold) in self.bursts.iter_mut().zip(THRESHOLDS_DB) {
            bursts.step(dbm, dbm >= floor + threshold);
        }
    }

    /// A preamble was detected, or we transmitted: whatever burst is in
    /// progress was (probably) a packet, so it doesn't count
    pub fn interrupted(&mut self) {
        if self.bursts[0].run > 0 {
            self.dropped += 1;
        }
        for bursts in &mut self.bursts {
            // Take back its busy time (a run begun in the last window can't
            // be taken back from there: close enough)
            bursts.busy_ms = bursts.busy_ms.saturating_sub(bursts.run);
            bursts.run = 0;
            bursts.quiet = None;
        }
    }

    /// Close this window (after logging it): learn the floor from it, and
    /// start counting afresh
    pub fn next_window(&mut self) {
        if self.samples > 0 {
            self.floor = Some(self.percentile(10));
        }
        self.levels = [0; LEVELS];
        self.samples = 0;
        self.dropped = 0;
        for bursts in &mut self.bursts {
            *bursts = Bursts {
                run: bursts.run,
                run_peak: bursts.run_peak,
                quiet: bursts.quiet,
                ..Default::default()
            };
        }
    }

    fn percentile(&self, percent: u32) -> i16 {
        let target = (self.samples * percent).div_ceil(100).max(1);
        let mut seen = 0;
        for (i, &count) in self.levels.iter().enumerate() {
            seen += count;
            if seen >= target {
                return LOWEST_DBM + i as i16;
            }
        }
        0
    }
}

impl Bursts {
    fn step(&mut self, dbm: i16, above: bool) {
        if above {
            if self.run == 0 {
                // A burst starts: the quiet before it is a gap
                if let Some(quiet) = self.quiet {
                    self.gaps[gap_bucket(quiet)] += 1;
                }
                self.run_peak = dbm;
            }
            self.run += 1;
            self.busy_ms += 1;
            self.run_peak = self.run_peak.max(dbm);
        } else if self.run > 0 {
            // A burst ends
            self.count += 1;
            self.lengths[length_bucket(self.run)] += 1;
            self.peak = Some(self.peak.unwrap_or(i16::MIN).max(self.run_peak));
            self.run = 0;
            self.quiet = Some(1);
        } else if let Some(quiet) = &mut self.quiet {
            *quiet += 1;
        }
    }
}

/// 1, 2-3, 4-7 ... 64+ ms: a bucket per power of two
fn length_bucket(ms: u32) -> usize {
    (ms.ilog2() as usize).min(LENGTH_BUCKETS - 1)
}

/// Under 10, 10-99, 100-999, 1000+ ms
fn gap_bucket(ms: u32) -> usize {
    match ms {
        0..10 => 0,
        10..100 => 1,
        100..1000 => 2,
        _ => 3,
    }
}

/// The log line, e.g.
/// `floor=-71 samples=9876 dropped=2 | +6dB n=14 busy=1.2% peak=-38
///  len=5,4,3,1,1,0,0 gap=2,3,6,2 | +10dB ... | +20dB ...`
/// len and gap are the bucket counts above, shortest first
impl fmt::Display for NoiseMeter {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let Some(floor) = self.floor else {
            return write!(f, "learning the floor, samples={}", self.samples);
        };
        write!(
            f,
            "floor={} samples={} dropped={}",
            floor, self.samples, self.dropped
        )?;
        for (bursts, threshold) in self.bursts.iter().zip(THRESHOLDS_DB) {
            let busy = bursts.busy_ms as f32 * 100.0 / self.samples.max(1) as f32;
            write!(
                f,
                " | +{}dB n={} busy={:.1}%",
                threshold, bursts.count, busy
            )?;
            if let Some(peak) = bursts.peak {
                write!(f, " peak={}", peak)?;
            }
            write!(f, " len=")?;
            write_counts(f, &bursts.lengths)?;
            write!(f, " gap=")?;
            write_counts(f, &bursts.gaps)?;
        }
        Ok(())
    }
}

fn write_counts(f: &mut fmt::Formatter, counts: &[u32]) -> fmt::Result {
    for (i, count) in counts.iter().enumerate() {
        if i > 0 {
            write!(f, ",")?;
        }
        write!(f, "{}", count)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A meter that has learned a floor of -100 dBm
    fn meter() -> NoiseMeter {
        let mut meter = NoiseMeter::default();
        for _ in 0..100 {
            meter.sample(-100);
        }
        meter.next_window();
        meter
    }

    fn feed(meter: &mut NoiseMeter, dbm: i16, ms: u32) {
        for _ in 0..ms {
            meter.sample(dbm);
        }
    }

    #[test]
    fn learns_the_floor_first() {
        let mut meter = NoiseMeter::default();
        feed(&mut meter, -60, 5);
        assert_eq!(meter.to_string(), "learning the floor, samples=5");
        // 90 quiet readings and 10 loud ones: the 10th percentile is quiet
        let mut meter = NoiseMeter::default();
        feed(&mut meter, -100, 90);
        feed(&mut meter, -50, 10);
        meter.next_window();
        assert_eq!(meter.floor, Some(-100));
    }

    #[test]
    fn counts_bursts_by_threshold_and_length() {
        let mut meter = meter();
        feed(&mut meter, -98, 1); // +2: nothing
        feed(&mut meter, -100, 50);
        feed(&mut meter, -93, 3); // +7: a 3 ms burst at +6
        feed(&mut meter, -100, 200);
        feed(&mut meter, -75, 20); // +25: 20 ms at every threshold
        feed(&mut meter, -100, 10);
        assert_eq!(
            meter.to_string(),
            "floor=-100 samples=284 dropped=0 \
             | +6dB n=2 busy=8.1% peak=-75 len=0,1,0,0,1,0,0 gap=0,0,1,0 \
             | +10dB n=1 busy=7.0% peak=-75 len=0,0,0,0,1,0,0 gap=0,0,0,0 \
             | +20dB n=1 busy=7.0% peak=-75 len=0,0,0,0,1,0,0 gap=0,0,0,0"
        );
    }

    #[test]
    fn a_packet_is_not_a_burst() {
        let mut meter = meter();
        feed(&mut meter, -60, 8); // a packet, until its preamble is detected
        meter.interrupted();
        feed(&mut meter, -100, 5);
        let line = meter.to_string();
        assert!(line.contains("dropped=1"), "{line}");
        assert!(line.contains("+6dB n=0"), "{line}");
        // ...and the next burst has no gap before it: a packet was in it
        feed(&mut meter, -90, 2);
        feed(&mut meter, -100, 1);
        let line = meter.to_string();
        assert!(
            line.contains("+6dB n=1 busy=12.5% peak=-90 len=0,1,0,0,0,0,0 gap=0,0,0,0"),
            "{line}"
        );
    }

    #[test]
    fn a_burst_across_windows_counts_once_where_it_ends() {
        let mut meter = meter();
        feed(&mut meter, -100, 100); // mostly quiet, so the floor stays -100
        feed(&mut meter, -80, 30);
        meter.next_window();
        feed(&mut meter, -80, 40);
        feed(&mut meter, -100, 1);
        // 70 ms in all: 64+, counted here, but busy only for its 40 ms here
        let line = meter.to_string();
        assert!(
            line.contains("+6dB n=1 busy=97.6% peak=-80 len=0,0,0,0,0,0,1"),
            "{line}"
        );
    }

    #[test]
    fn buckets() {
        assert_eq!(
            [1, 2, 3, 4, 7, 8, 63, 64, 5000].map(length_bucket),
            [0, 1, 1, 2, 2, 3, 5, 6, 6]
        );
        assert_eq!(
            [1, 9, 10, 99, 100, 999, 1000].map(gap_bucket),
            [0, 0, 1, 1, 2, 2, 3]
        );
    }
}
