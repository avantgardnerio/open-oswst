//! The radio channel, one LoRa symbol at a time: was it demodulated right,
//! and if not, which wrong bin won?
//!
//! The receiver multiplies by a down-chirp and takes an FFT of 2^SF bins. The
//! sent symbol turns into a peak in one bin; white noise turns into
//! independent complex Gaussian noise in every bin. The receiver picks the
//! biggest bin. So we skip the samples and the FFT and draw what they would
//! give, which is exact for white noise and perfect timing:
//!
//! - the right bin: signal plus noise, |sqrt(2^SF x SNR) + z|^2
//! - the best of the 2^SF - 1 wrong bins: the largest of that many unit
//!   exponentials, drawn in one go from its inverse CDF
//!
//! If the best wrong bin beats the right one, the symbol is lost to a random
//! wrong bin (every wrong bin is equally likely in white noise).
//!
//! Bursts: on top of the steady noise, a Gilbert-Elliott model switches the
//! noise up by `extra_noise_db` for random stretches. The field walks found
//! our range edges are set by noise, not signal, and FEC has to survive that.

pub struct Burst {
    /// Average time between bursts
    pub mean_gap_ms: f64,
    /// Average burst length
    pub mean_len_ms: f64,
    /// How much worse the noise gets during one
    pub extra_noise_db: f64,
}

pub struct Channel {
    rng: Rng,
    bins: u32,
    /// Signal amplitude in the right bin, noise bins having unit power
    amplitude: f64,
    amplitude_in_burst: f64,
    /// Per-symbol chances of a burst starting and ending
    burst_start: f64,
    burst_end: f64,
    in_burst: bool,
}

impl Channel {
    /// `snr_db` is what the radio would report: signal over the noise in its
    /// own bandwidth. `symbol_ms` converts the burst timing into symbols
    pub fn new(seed: u64, sf: u32, snr_db: f64, symbol_ms: f64, burst: Option<&Burst>) -> Self {
        let bins = 1 << sf;
        let amplitude = |snr_db: f64| (bins as f64 * 10f64.powf(snr_db / 10.0)).sqrt();
        let (burst_start, burst_end, extra) = match burst {
            Some(b) => (
                symbol_ms / b.mean_gap_ms,
                symbol_ms / b.mean_len_ms,
                b.extra_noise_db,
            ),
            None => (0.0, 1.0, 0.0),
        };
        Channel {
            rng: Rng(seed | 1),
            bins,
            amplitude: amplitude(snr_db),
            amplitude_in_burst: amplitude(snr_db - extra),
            burst_start,
            burst_end,
            in_burst: false,
        }
    }

    /// The bin the receiver picks when `sent` was transmitted
    pub fn demodulate(&mut self, sent: u32) -> u32 {
        self.in_burst = if self.in_burst {
            self.rng.uniform() >= self.burst_end
        } else {
            self.rng.uniform() < self.burst_start
        };
        let amplitude = if self.in_burst {
            self.amplitude_in_burst
        } else {
            self.amplitude
        };

        // Complex unit-power noise: each part has variance 1/2
        let (re, im) = self.rng.gaussian_pair();
        let right = (amplitude + re * 0.5f64.sqrt()).powi(2) + (im * 0.5f64.sqrt()).powi(2);
        let wrong_bins = (self.bins - 1) as f64;
        let best_wrong = -(1.0 - self.rng.uniform().powf(1.0 / wrong_bins)).ln();

        if best_wrong <= right {
            return sent;
        }
        // Any bin but the right one
        let offset = 1 + (self.rng.next() % (self.bins as u64 - 1)) as u32;
        (sent + offset) % self.bins
    }
}

/// xorshift64*: fast and plenty random for Monte Carlo, and no crates
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// In (0, 1]: never 0, so ln() is safe
    pub fn uniform(&mut self) -> f64 {
        ((self.next() >> 11) + 1) as f64 / (1u64 << 53) as f64
    }

    /// Two independent standard normals (Box-Muller)
    fn gaussian_pair(&mut self) -> (f64, f64) {
        let radius = (-2.0 * self.uniform().ln()).sqrt();
        let angle = 2.0 * std::f64::consts::PI * self.uniform();
        (radius * angle.cos(), radius * angle.sin())
    }
}
