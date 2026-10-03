//! How close each received transmission came to an audio gap. Logged once
//! per transmission (at its EOT or RX timeout), to compare thread placements
//! and priorities: WiFi, the codec and the app all compete for the CPU, and
//! the speaker underruns when the app falls behind.
//!
//! Per packet:
//! - codec: the codec thread's own time decoding it (CPU, plus any preemption)
//! - wait: how long the app waited for the decoded audio, from asking to
//!   getting it (the codec time plus however long the codec thread took to
//!   get scheduled)
//! - queued: speaker frames still waiting when the decoded packet arrived,
//!   i.e. how much audio was left before a gap. 0 = the speaker was about to
//!   run dry (or already had)
//!
//! Underruns are counted by the speaker driver.

use core::fmt;

use crate::devices::speaker;

/// Packets per transmission we keep numbers for: 40 s of audio
const MAX_PACKETS: usize = 256;

pub struct PlaybackTiming {
    codec_us: [u32; MAX_PACKETS],
    wait_us: [u32; MAX_PACKETS],
    queued: [u32; MAX_PACKETS],
    packets: usize,
}

impl Default for PlaybackTiming {
    fn default() -> Self {
        PlaybackTiming {
            codec_us: [0; MAX_PACKETS],
            wait_us: [0; MAX_PACKETS],
            queued: [0; MAX_PACKETS],
            packets: 0,
        }
    }
}

impl PlaybackTiming {
    pub fn record(&mut self, codec_us: u32, wait_us: u32, queued: usize) {
        if self.packets < MAX_PACKETS {
            self.codec_us[self.packets] = codec_us;
            self.wait_us[self.packets] = wait_us;
            self.queued[self.packets] = queued as u32;
        }
        self.packets += 1;
    }

    /// The transmission ended: log its line (if it had any audio) and start over
    pub fn log_and_reset(&mut self) {
        let (underruns, underrun_ms) = speaker::take_underruns();
        if self.packets > 0 {
            log::info!("{}", self.summary(underruns, underrun_ms));
        }
        self.packets = 0;
    }

    fn summary(&mut self, underruns: u32, underrun_ms: u32) -> Summary {
        let kept = self.packets.min(MAX_PACKETS);
        Summary {
            packets: self.packets,
            codec_us: p50_max(&mut self.codec_us[..kept]),
            wait_us: p50_max(&mut self.wait_us[..kept]),
            // The first packet always finds an empty queue: playback starts with it
            queued_min: self.queued[1.min(kept)..kept]
                .iter()
                .copied()
                .min()
                .unwrap_or(0),
            queued_p50: p50_max(&mut self.queued[1.min(kept)..kept]).0,
            underruns,
            underrun_ms,
        }
    }
}

struct Summary {
    packets: usize,
    codec_us: (u32, u32),
    wait_us: (u32, u32),
    queued_min: u32,
    queued_p50: u32,
    underruns: u32,
    underrun_ms: u32,
}

/// e.g. `Playback: 42 packets, codec p50/max 31.2/48.0ms, wait p50/max
/// 33.0/120.4ms, queued min/p50 0/3 frames, underruns 2 (138ms)`
impl fmt::Display for Summary {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let ms = |us: u32| us as f32 / 1000.0;
        write!(
            f,
            "Playback: {} packets, codec p50/max {:.1}/{:.1}ms, wait p50/max {:.1}/{:.1}ms, \
             queued min/p50 {}/{} frames, underruns {} ({}ms)",
            self.packets,
            ms(self.codec_us.0),
            ms(self.codec_us.1),
            ms(self.wait_us.0),
            ms(self.wait_us.1),
            self.queued_min,
            self.queued_p50,
            self.underruns,
            self.underrun_ms
        )
    }
}

/// Median and maximum, sorting in place (no allocation)
fn p50_max(values: &mut [u32]) -> (u32, u32) {
    if values.is_empty() {
        return (0, 0);
    }
    values.sort_unstable();
    (values[values.len() / 2], values[values.len() - 1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_line() {
        let mut timing = PlaybackTiming::default();
        timing.record(30_000, 31_000, 0); // the first packet: empty queue, ignored
        timing.record(32_000, 120_400, 1);
        timing.record(48_000, 33_000, 3);
        timing.record(31_200, 32_000, 4);
        assert_eq!(
            timing.summary(2, 138).to_string(),
            "Playback: 4 packets, codec p50/max 32.0/48.0ms, wait p50/max 33.0/120.4ms, \
             queued min/p50 1/3 frames, underruns 2 (138ms)"
        );
    }

    #[test]
    fn more_packets_than_kept() {
        let mut timing = PlaybackTiming::default();
        for _ in 0..MAX_PACKETS + 10 {
            timing.record(1000, 2000, 2);
        }
        let line = timing.summary(0, 0).to_string();
        assert!(line.starts_with("Playback: 266 packets"), "{line}");
    }
}
