mod app;
mod radio;

use embassy_futures::join::join;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use esp_idf_svc::hal::gpio::PinDriver;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::task::block_on;
use std::thread;
use std::time::Duration;

// --- Channel message types ---

pub(crate) struct RxPacket {
    pub data: heapless::Vec<u8, 255>,
    pub rssi: i16,
    pub snr: i16,
}

pub(crate) struct TxRequest {
    pub data: heapless::Vec<u8, 255>,
}

// --- Channel instances (static, ISR-safe) ---

pub(crate) static RX_CHAN: Channel<CriticalSectionRawMutex, RxPacket, 2> = Channel::new();
pub(crate) static TX_CHAN: Channel<CriticalSectionRawMutex, TxRequest, 4> = Channel::new();

/// Read the base MAC address from eFuse
fn get_mac() -> [u8; 6] {
    let mut mac = [0u8; 6];
    unsafe {
        esp_idf_svc::sys::esp_read_mac(
            mac.as_mut_ptr(),
            esp_idf_svc::sys::esp_mac_type_t_ESP_MAC_WIFI_STA,
        );
    }
    mac
}

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("Loraudio starting...");

    let peripherals = Peripherals::take().unwrap();

    // Enable Vext power (GPIO36 LOW = on) — must keep _vext alive or power turns off
    let mut _vext = PinDriver::output(peripherals.pins.gpio36).unwrap();
    _vext.set_low().unwrap();
    thread::sleep(Duration::from_millis(50));

    // Get MAC for display
    let mac = get_mac();
    let mut mac_str = heapless::String::<18>::new();
    let _ = core::fmt::write(
        &mut mac_str,
        format_args!(
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        ),
    );

    block_on(async {
        let radio_fut = radio::init(radio::Peripherals {
            spi: peripherals.spi2,
            sck: peripherals.pins.gpio9.into(),
            mosi: peripherals.pins.gpio10.into(),
            miso: peripherals.pins.gpio11.into(),
            nss: peripherals.pins.gpio8.into(),
            reset: peripherals.pins.gpio12.into(),
            dio1: peripherals.pins.gpio14.into(),
            busy: peripherals.pins.gpio13.into(),
        })
        .await;

        let app_fut = app::init(
            app::Peripherals {
                ptt: peripherals.pins.gpio0.into(),
                audio_in: peripherals.pins.gpio7,
                adc: peripherals.adc1,
                i2c: peripherals.i2c0,
                oled_sda: peripherals.pins.gpio17.into(),
                oled_scl: peripherals.pins.gpio18.into(),
                oled_rst: peripherals.pins.gpio21.into(),
            },
            mac_str,
        )
        .await;

        log::info!("All systems ready");
        join(radio_fut, app_fut).await;
    });
}
