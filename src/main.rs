use esp_idf_svc::hal::gpio::PinDriver;
use esp_idf_svc::hal::peripherals::Peripherals;
use std::thread;
use std::time::Duration;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("Hello from loraudio!");

    let peripherals = Peripherals::take().unwrap();
    let mut led = PinDriver::output(peripherals.pins.gpio35).unwrap();

    loop {
        led.set_high().unwrap();
        log::info!("LED on");
        thread::sleep(Duration::from_millis(500));

        led.set_low().unwrap();
        log::info!("LED off");
        thread::sleep(Duration::from_millis(500));
    }
}
