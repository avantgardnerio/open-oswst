//! RF front-end module on the Heltec V4.3: a KCT8103L PA + LNA between the
//! SX1262 and the antenna. Unpowered, it's a dead chip in the signal path.
//!
//! This powers the FEM and holds it enabled. Switching it between TX and RX
//! is the radio's job: the SX1262's DIO2 drives the FEM's CPS pin, and the
//! radio drives CTX (HIGH for TX, LOW for RX through the LNA).
//!
//! Pins per Meshtastic's heltec_v4 variant.h. Assumes a V4.3 or later; a V4.2
//! (GC1109) uses a different TX pin and gain.

use esp_idf_svc::hal::gpio::{AnyOutputPin, Output, PinDriver};
use std::thread;
use std::time::Duration;

pub struct Peripherals {
    /// LDO enable for the FEM's supply (GPIO7)
    pub power: AnyOutputPin<'static>,
    /// CSD chip enable (GPIO2)
    pub csd: AnyOutputPin<'static>,
}

/// Holds the FEM powered and enabled. Dropping it turns the FEM off.
pub struct Fem {
    _power: PinDriver<'static, Output>,
    _csd: PinDriver<'static, Output>,
}

pub fn init(p: Peripherals) -> Fem {
    let mut power = PinDriver::output(p.power).unwrap();
    power.set_high().unwrap();
    thread::sleep(Duration::from_millis(1)); // LDO settle, as Meshtastic does

    let mut csd = PinDriver::output(p.csd).unwrap();
    csd.set_high().unwrap();
    log::info!("FEM powered and enabled");

    Fem {
        _power: power,
        _csd: csd,
    }
}
