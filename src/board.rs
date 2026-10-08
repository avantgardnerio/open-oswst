//! Pin map for the open-oswst PCB (v2) on a Heltec WiFi LoRa 32 V4.
//! The one place pins are assigned — every app gets its peripherals from here.

use esp_idf_svc::hal::gpio::{AnyIOPin, Output, PinDriver};
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::hal::peripherals::Peripherals;
use std::thread;
use std::time::Duration;

use crate::devices::{battery, encoder, fem, gps, irq_pin, mic, radio, screen, speaker};

pub struct Board {
    pub fem: fem::Peripherals,
    pub gps: gps::Peripherals,
    pub radio: radio::Peripherals,
    pub speaker: speaker::Peripherals,
    pub screen: screen::Peripherals,
    pub mic: mic::Peripherals,
    pub battery: battery::Peripherals,
    pub vol: encoder::Peripherals,
    pub ptt: AnyIOPin<'static>,
    /// WiFi (net.rs)
    pub modem: Modem<'static>,
    // Vext powers the OLED — must stay alive or power turns off
    _vext: PinDriver<'static, Output>,
}

/// Take the chip peripherals, power on Vext, install the GPIO interrupt
/// service, and hand out each subsystem's pins.
pub fn take() -> Board {
    let p = Peripherals::take().unwrap();

    // Vext power on (GPIO36 LOW)
    let mut vext = PinDriver::output(p.pins.gpio36).unwrap();
    vext.set_low().unwrap();
    thread::sleep(Duration::from_millis(50));

    // Before any pin waits on an interrupt (devices::irq_pin)
    irq_pin::install();

    Board {
        fem: fem::Peripherals {
            power: p.pins.gpio7.into(),
            csd: p.pins.gpio2.into(),
        },
        // Heltec GNSS connector. Note RX/TX: Meshtastic's variant.h comments
        // have them the other way round, and following those gives silence
        gps: gps::Peripherals {
            uart: p.uart1,
            tx: p.pins.gpio38.into(),
            rx: p.pins.gpio39.into(),
            power: p.pins.gpio34.into(),
            reset: p.pins.gpio42.into(),
            wake: p.pins.gpio40.into(),
        },
        radio: radio::Peripherals {
            spi: p.spi2,
            sck: p.pins.gpio9.into(),
            mosi: p.pins.gpio10.into(),
            miso: p.pins.gpio11.into(),
            nss: p.pins.gpio8.into(),
            reset: p.pins.gpio12.into(),
            dio1: p.pins.gpio14.into(),
            busy: p.pins.gpio13.into(),
            rf_switch_tx: Some(p.pins.gpio5.into()), // FEM CTX
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
        // The Heltec's own battery divider (needs PCB rev 4468800: GPIO1 free)
        battery: battery::Peripherals {
            pin: p.pins.gpio1,
            enable: p.pins.gpio37.into(), // ADC_CTRL
        },
        vol: encoder::Peripherals {
            a: p.pins.gpio3.into(),
            b: p.pins.gpio6.into(),   // GPIO2 is the FEM's
            sw: p.pins.gpio45.into(), // GPIO1 is battery sense
        },
        ptt: p.pins.gpio0.into(),
        modem: p.modem,
        _vext: vext,
    }
}
