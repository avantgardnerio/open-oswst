//! Pin map for the open-oswst PCB (v2) on a Heltec WiFi LoRa 32 V4.
//! The one place pins are assigned — every app gets its peripherals from here.

use esp_idf_svc::hal::gpio::{AnyIOPin, Output, PinDriver};
use esp_idf_svc::hal::peripherals::Peripherals;
use std::thread;
use std::time::Duration;

use crate::devices::{mic, radio, screen, speaker};

pub struct Board {
    pub radio: radio::Peripherals,
    pub speaker: speaker::Peripherals,
    pub screen: screen::Peripherals,
    pub mic: mic::Peripherals,
    pub ptt: AnyIOPin<'static>,
    // Vext powers the OLED — must stay alive or power turns off
    _vext: PinDriver<'static, Output>,
}

/// Take the chip peripherals, power on Vext, and hand out each subsystem's pins.
pub fn take() -> Board {
    let p = Peripherals::take().unwrap();

    // Vext power on (GPIO36 LOW)
    let mut vext = PinDriver::output(p.pins.gpio36).unwrap();
    vext.set_low().unwrap();
    thread::sleep(Duration::from_millis(50));

    Board {
        radio: radio::Peripherals {
            spi: p.spi2,
            sck: p.pins.gpio9.into(),
            mosi: p.pins.gpio10.into(),
            miso: p.pins.gpio11.into(),
            nss: p.pins.gpio8.into(),
            reset: p.pins.gpio12.into(),
            dio1: p.pins.gpio14.into(),
            busy: p.pins.gpio13.into(),
        },
        speaker: speaker::Peripherals {
            i2s: p.i2s0,
            spk_bclk: p.pins.gpio47.into(),
            spk_din: p.pins.gpio33.into(),
            spk_ws: p.pins.gpio48.into(),
        },
        screen: screen::Peripherals {
            i2c: p.i2c0,
            sda: p.pins.gpio17.into(),
            scl: p.pins.gpio18.into(),
            rst: p.pins.gpio21.into(),
        },
        mic: mic::Peripherals {
            adc: p.adc1,
            pin: p.pins.gpio4,
        },
        ptt: p.pins.gpio0.into(),
        _vext: vext,
    }
}
