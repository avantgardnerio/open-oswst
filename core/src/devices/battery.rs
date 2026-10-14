//! The battery: its voltage, read now and then, and what that voltage says
//! about the charge left. Knows nothing else.

#[allow(async_fn_in_trait)] // used by one task at a time; Send never needed
pub trait Battery {
    /// The battery's voltage in mV; None if it can't be read right now
    async fn millivolts(&mut self) -> Option<u32>;
}

/// Charge left, 0-100 %, from the voltage under the radio's idle load.
///
/// A skewed logistic fitted to one full discharge: a 2000 mAh LiPo on the
/// breadboard radio, idle (RX, WiFi, GPS, screen on), 4255 mV to cutoff at
/// 3031 mV in 18.4 h. At a steady load the charge left falls straight with
/// time, so each reading's percent is the time still to go. The fit is
/// within 3.7 % rms of that run; the worst (9 %) is on the curve's two flat
/// stretches, around 4.10 and 3.92 V, where the ADC's +-20 mV is worth
/// many percent. Under heavier load (talking) the voltage sags, so the
/// percent reads low.
///
/// Log and plots: logs/F85B1BA2C848-2026-10-14T130431-battery-dead/
pub fn percent(millivolts: u32) -> u8 {
    // Where the curve is steepest, mV; how wide its bend, mV; its skew
    const MIDDLE: f32 = 4095.0;
    const WIDTH: f32 = 74.2;
    const SKEW: f32 = 0.289;
    let rise = (-(millivolts as f32 - MIDDLE) / WIDTH).exp();
    let percent = 100.0 / (1.0 + rise).powf(SKEW);
    percent.round().clamp(0.0, 100.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_follows_the_discharge() {
        // Points from the run the curve was fitted to (hourly averages)
        assert_eq!(percent(4255), 97); // off the charger
        assert_eq!(percent(3933), 52); // hour 8-9 of 18.4: 54 % left
        assert_eq!(percent(3559), 12); // hour 16-17: 11 % left
        assert_eq!(percent(3031), 2); // cutoff
        assert_eq!(percent(0), 0);
        assert_eq!(percent(5000), 100);
        // Never rises as the voltage falls
        let mut last = 100;
        for millivolts in (2500..=4500).rev() {
            let now = percent(millivolts);
            assert!(now <= last, "{millivolts} mV: {now} % after {last} %");
            last = now;
        }
    }
}
