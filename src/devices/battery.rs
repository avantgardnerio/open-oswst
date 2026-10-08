//! Battery voltage, through the Heltec V4's own divider: GPIO1 (ADC1), which
//! ADC_CTRL (GPIO37) high switches onto the battery. Battery mV = pin mV x
//! 4.9 x 1.045 (Meshtastic's heltec_v4; batt_probe.rs checked it on the
//! bench, 2026-09-29: ~850 mV at the pin, 4.34-4.38 V on USB). Needs PCB rev
//! `4468800` or later: rev `dbc0ed0` has the knob's switch on GPIO1.
//!
//! ADC1 belongs to the mic while PTT is held (adc1.rs), so a reading is only
//! taken with the mic stopped.

use embassy_time::Timer;
use esp_idf_svc::hal::adc::attenuation::adc_atten_t_ADC_ATTEN_DB_2_5;
use esp_idf_svc::hal::adc::oneshot::config::{AdcChannelConfig, Calibration};
use esp_idf_svc::hal::adc::oneshot::{AdcChannelDriver, AdcDriver};
use esp_idf_svc::hal::adc::ADC1;
use esp_idf_svc::hal::gpio::{AnyIOPin, Gpio1, Output, PinDriver};

use super::adc1;

/// Meshtastic's heltec_v4 ADC_MULTIPLIER: the divider's ratio plus a trim
const MULTIPLIER: f32 = 4.9 * 1.045;

/// The divider settles this long after ADC_CTRL switches it on
const SETTLE_MS: u64 = 10;

/// Readings averaged per measurement, to quiet the ADC's noise
const SAMPLES: u32 = 8;

pub struct Peripherals {
    pub pin: Gpio1<'static>,
    pub enable: AnyIOPin<'static>, // ADC_CTRL
}

pub struct Battery {
    pin: Gpio1<'static>,
    enable: PinDriver<'static, Output>,
}

pub fn init(p: Peripherals) -> Battery {
    let mut enable = PinDriver::output(p.enable).unwrap();
    // Off between readings: the divider drains the battery while it's on
    enable.set_low().unwrap();
    Battery { pin: p.pin, enable }
}

impl Battery {
    /// The battery's voltage in mV; None if ADC1 was busy or failed
    pub async fn millivolts(&mut self) -> Option<u32> {
        self.enable.set_high().unwrap();
        Timer::after_millis(SETTLE_MS).await;
        let pin_mv = self.read_pin_mv();
        self.enable.set_low().unwrap();
        pin_mv.map(|mv| (mv as f32 * MULTIPLIER) as u32)
    }

    /// GPIO1's voltage, averaged. The ADC driver lives only for the reading,
    /// so the mic can have ADC1 the moment PTT is pressed
    fn read_pin_mv(&mut self) -> Option<u32> {
        let _lease = adc1::lease()?;
        // SAFETY: the lease makes this the only ADC1 driver until it drops
        let adc = AdcDriver::new(unsafe { ADC1::steal() }).ok()?;
        let config = AdcChannelConfig {
            attenuation: adc_atten_t_ADC_ATTEN_DB_2_5, // ~0-1 V; a 4.2 V LiPo is ~0.86 V here
            calibration: Calibration::Curve,
            ..Default::default()
        };
        // SAFETY: the channel driver is dropped before this returns
        let mut channel =
            AdcChannelDriver::new(&adc, unsafe { self.pin.reborrow() }, &config).ok()?;
        let mut total = 0u32;
        for _ in 0..SAMPLES {
            total += channel.read().ok()? as u32;
        }
        Some(total / SAMPLES)
    }
}

impl open_oswst_core::devices::battery::Battery for Battery {
    async fn millivolts(&mut self) -> Option<u32> {
        Battery::millivolts(self).await
    }
}
