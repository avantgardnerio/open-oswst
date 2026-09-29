//! Battery-sense + GPIO45 probe, to qualify freeing GPIO1 (VOL_SW today) for
//! battery sense in the next PCB rev. Unplug the VOL harness first.
//!
//! Once a second:
//! - Reads GPIO1 through the ADC with ADC_CTRL (GPIO37) LOW, then HIGH. Per
//!   Meshtastic's heltec_v4 variant, HIGH switches the battery divider onto
//!   GPIO1: Vbat = mV x 4.9 x 1.045. Expect ~3.3-4.2 V with a LiPo attached
//!   (on USB alone it reads the charger). LOW should read near 0.
//! - Reads GPIO45 with its internal pull-up. Jumper it to GND: it should read 0.
//!   Then power-cycle with the jumper still on: if this log appears, grounding
//!   GPIO45 at boot is safe.
//!
//! Build & flash: cargo build --bin batt_probe && espflash flash -p <PORT> --partition-table target/xtensa-esp32s3-espidf/debug/partition-table.bin target/xtensa-esp32s3-espidf/debug/batt_probe

use esp_idf_svc::hal::adc::attenuation::adc_atten_t_ADC_ATTEN_DB_2_5;
use esp_idf_svc::hal::adc::oneshot::config::{AdcChannelConfig, Calibration};
use esp_idf_svc::hal::adc::oneshot::{AdcChannelDriver, AdcDriver};
use esp_idf_svc::hal::gpio::{PinDriver, Pull};
use esp_idf_svc::hal::peripherals::Peripherals;
use std::thread;
use std::time::Duration;

/// Meshtastic's heltec_v4 ADC_MULTIPLIER: divider ratio plus a trim
const VBAT_MULTIPLIER: f32 = 4.9 * 1.045;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let p = Peripherals::take().unwrap();

    // Read GPIO45 before touching its pull: shows what the strap circuit sees.
    let gpio45 = PinDriver::input(p.pins.gpio45, Pull::Floating).unwrap();
    log::info!(
        "batt_probe booted. GPIO45 floating at boot = {}",
        gpio45.is_high() as u8
    );
    drop(gpio45);
    // SAFETY: the floating driver above was dropped; this is the only one now.
    let gpio45 =
        PinDriver::input(unsafe { esp_idf_svc::hal::gpio::Gpio45::steal() }, Pull::Up).unwrap();

    let mut adc_ctrl = PinDriver::output(p.pins.gpio37).unwrap();
    adc_ctrl.set_low().unwrap();

    let adc = AdcDriver::new(p.adc1).unwrap();
    let config = AdcChannelConfig {
        attenuation: adc_atten_t_ADC_ATTEN_DB_2_5, // ~0-1 V; a 4.2 V LiPo lands at ~0.86 V
        calibration: Calibration::Curve,
        ..Default::default()
    };
    let mut vbat = AdcChannelDriver::new(&adc, p.pins.gpio1, &config).unwrap();

    loop {
        adc_ctrl.set_low().unwrap();
        thread::sleep(Duration::from_millis(10));
        let off_mv = vbat.read().unwrap();

        adc_ctrl.set_high().unwrap();
        thread::sleep(Duration::from_millis(10)); // divider settle
        let on_mv = vbat.read().unwrap();
        adc_ctrl.set_low().unwrap();

        log::info!(
            "GPIO1: ADC_CTRL off {} mV | on {} mV -> Vbat {:.2} V | GPIO45 (pull-up) = {}",
            off_mv,
            on_mv,
            on_mv as f32 * VBAT_MULTIPLIER / 1000.0,
            gpio45.is_high() as u8
        );
        thread::sleep(Duration::from_secs(1));
    }
}
