//! High-pass filter for the mic audio, just before the Codec2 encoder.
//!
//! The mic reaches the encoder with two things below the voice band that
//! wreck Codec2 1200 (bench, 2026-11-05):
//! - a DC offset of ~-10000: the MAX9814 output idles at 1.23 V, well below
//!   the ADC's midpoint
//! - with a headset like the Disco32, a slow wander after PTT: 82% of the
//!   keyed audio's power was below 50 Hz, and its speech came out of Codec2
//!   unintelligible. Through a 150 Hz high-pass first, it was intelligible
//!
//! Codec2 has no use for anything below ~150 Hz, so every mic gets this.
//! A 2nd-order Butterworth biquad (RBJ cookbook), transposed direct form II.
//! Its state carries over from one packet to the next.

use std::f32::consts::PI;

pub const SAMPLE_RATE_HZ: f32 = 8000.0;
pub const CUTOFF_HZ: f32 = 150.0;

pub struct Highpass {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    state1: f32,
    state2: f32,
}

impl Highpass {
    pub fn new(cutoff_hz: f32, sample_rate_hz: f32) -> Self {
        let omega = 2.0 * PI * cutoff_hz / sample_rate_hz;
        // Butterworth: Q = 1/sqrt(2)
        let alpha = omega.sin() / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let cos = omega.cos();
        let a0 = 1.0 + alpha;
        Self {
            b0: (1.0 + cos) / 2.0 / a0,
            b1: -(1.0 + cos) / a0,
            b2: (1.0 + cos) / 2.0 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
            state1: 0.0,
            state2: 0.0,
        }
    }

    /// Filters `samples` in place
    pub fn run(&mut self, samples: &mut [i16]) {
        for sample in samples {
            let input = *sample as f32;
            let output = self.b0 * input + self.state1;
            self.state1 = self.b1 * input - self.a1 * output + self.state2;
            self.state2 = self.b2 * input - self.a2 * output;
            *sample = output.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(frequency_hz: f32, amplitude: f32, offset: f32, samples: usize) -> Vec<i16> {
        (0..samples)
            .map(|index| {
                let time = index as f32 / SAMPLE_RATE_HZ;
                (offset + amplitude * (2.0 * PI * frequency_hz * time).sin()) as i16
            })
            .collect()
    }

    fn rms(samples: &[i16]) -> f32 {
        let sum: f32 = samples.iter().map(|&sample| (sample as f32).powi(2)).sum();
        (sum / samples.len() as f32).sqrt()
    }

    /// Filtered in packet-sized pieces, as the codec thread does, with the
    /// first second dropped (the filter settling)
    fn filtered_rms(mut samples: Vec<i16>) -> f32 {
        let mut filter = Highpass::new(CUTOFF_HZ, SAMPLE_RATE_HZ);
        for packet in samples.chunks_mut(1280) {
            filter.run(packet);
        }
        rms(&samples[8000..])
    }

    #[test]
    fn removes_the_dc_offset() {
        let silence_at_offset = vec![-10_000i16; 16_000];
        assert!(filtered_rms(silence_at_offset) < 1.0);
    }

    #[test]
    fn cuts_a_slow_wander() {
        let wander = tone(20.0, 8000.0, -10_000.0, 16_000);
        // 2nd order, 150 Hz: ~35 dB down at 20 Hz
        assert!(filtered_rms(wander) < 8000.0 * 0.707 * 0.03);
    }

    #[test]
    fn passes_the_voice_band() {
        for frequency_hz in [300.0, 1000.0, 3000.0] {
            let voice = tone(frequency_hz, 8000.0, -10_000.0, 16_000);
            let ratio = filtered_rms(voice) / (8000.0 * 0.707);
            assert!(
                ratio > 0.85 && ratio < 1.05,
                "{} Hz: {}",
                frequency_hz,
                ratio
            );
        }
    }
}
