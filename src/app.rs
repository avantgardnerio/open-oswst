use embassy_futures::select::{select, Either};
use embedded_graphics::mono_font::ascii::FONT_6X10;
use embedded_graphics::mono_font::MonoTextStyleBuilder;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use esp_idf_svc::hal::adc::continuous::config::Config as AdcContConfig;
use esp_idf_svc::hal::adc::continuous::{AdcDriver as AdcContDriver, AdcMeasurement, Attenuated};
use esp_idf_svc::hal::adc::ADC1;
use esp_idf_svc::hal::gpio::{Gpio0, Gpio17, Gpio18, Gpio21, Gpio7};
use esp_idf_svc::hal::gpio::{Input, PinDriver, Pull};
use esp_idf_svc::hal::i2c::config::Config as I2cConfig;
use esp_idf_svc::hal::i2c::{I2cDriver, I2C0};
use esp_idf_svc::hal::units::Hertz;
use ssd1306::mode::BufferedGraphicsMode;
use ssd1306::prelude::*;
use ssd1306::{I2CDisplayInterface, Ssd1306};
use std::future::Future;
use std::thread;
use std::time::Duration;

use crate::{TxRequest, RX_CHAN, TX_CHAN};

type Display<'a> = Ssd1306<
    I2CInterface<I2cDriver<'a>>,
    DisplaySize128x64,
    BufferedGraphicsMode<DisplaySize128x64>,
>;

pub struct Peripherals {
    pub ptt: Gpio0<'static>,
    pub audio_in: Gpio7<'static>,
    pub adc: ADC1<'static>,
    pub i2c: I2C0<'static>,
    pub oled_sda: Gpio17<'static>,
    pub oled_scl: Gpio18<'static>,
    pub oled_rst: Gpio21<'static>,
}

pub async fn init(p: Peripherals, mac_str: heapless::String<18>) -> impl Future<Output = ()> {
    // PRG button on GPIO0 — active LOW with internal pull-up
    let button = PinDriver::input(p.ptt, Pull::Up).unwrap();

    // Continuous ADC for mic on GPIO7 (ADC1_CH6) — DMA at 8kHz
    let adc_config = AdcContConfig::new()
        .sample_freq(Hertz(8000))
        .frame_measurements(320) // 320 samples = 40ms at 8kHz (one Codec2 frame)
        .frames_count(2); // double buffer

    let mut adc = AdcContDriver::new(p.adc, &adc_config, Attenuated::db12(p.audio_in)).unwrap();

    // Reset OLED
    let mut oled_rst = PinDriver::output(p.oled_rst).unwrap();
    oled_rst.set_low().unwrap();
    thread::sleep(Duration::from_millis(50));
    oled_rst.set_high().unwrap();
    thread::sleep(Duration::from_millis(50));

    // I2C for OLED
    let i2c = I2cDriver::new(p.i2c, p.oled_sda, p.oled_scl, &I2cConfig::default()).unwrap();

    // OLED init
    let interface = I2CDisplayInterface::new(i2c);
    let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();
    display.init().unwrap();
    display.set_brightness(Brightness::BRIGHTEST).unwrap();
    log::info!("OLED initialized, MAC: {}", mac_str);

    // Start continuous ADC (DMA)
    adc.start().unwrap();
    log::info!("ADC DMA started at 8kHz");

    async move {
        // Keep oled_rst alive so the pin doesn't float low (holding OLED in reset)
        let _oled_rst = oled_rst;
        app_loop(button, adc, display, &mac_str).await;
    }
}

async fn app_loop(
    mut button: PinDriver<'_, Input>,
    mut adc: AdcContDriver<'_>,
    mut display: Display<'_>,
    mac_str: &str,
) {
    let style = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(BinaryColor::On)
        .build();

    let mut tx_count: u32 = 0;
    let mut line_buf = heapless::String::<64>::new();
    let mut mic_buf = [AdcMeasurement::new(); 320];

    // Show initial RX state
    draw_rx_screen(&mut display, mac_str, &style);

    loop {
        match select(RX_CHAN.receive(), button.wait_for_low()).await {
            Either::First(rx_pkt) => {
                let msg = core::str::from_utf8(&rx_pkt.data).unwrap_or("???");
                log::info!(
                    "RX [{}B] rssi={}dBm snr={}dB: {}",
                    rx_pkt.data.len(),
                    rx_pkt.rssi,
                    rx_pkt.snr,
                    msg
                );

                display.clear_buffer();
                Text::new(mac_str, Point::new(1, 10), style)
                    .draw(&mut display)
                    .unwrap();
                Text::new(msg, Point::new(0, 32), style)
                    .draw(&mut display)
                    .unwrap();

                line_buf.clear();
                let _ = core::fmt::write(
                    &mut line_buf,
                    format_args!("RSSI:{} SNR:{}", rx_pkt.rssi, rx_pkt.snr),
                );
                Text::new(&line_buf, Point::new(0, 48), style)
                    .draw(&mut display)
                    .unwrap();
                display.flush().unwrap();
            }
            Either::Second(_) => {
                // PTT button pressed — enter TX mode
                log::info!("PTT pressed — switching to TX");

                // Drain any stale mic data
                let _ = adc.read(&mut mic_buf, 0);

                while button.is_low() {
                    // Wait for a fresh mic frame from DMA
                    let count = adc.read_async(&mut mic_buf).await.unwrap_or(0);

                    let msg = format!("TX #{} ({}samp)", tx_count, count);
                    log::info!("{}", msg);

                    // Send TX request to radio task
                    let mut data = heapless::Vec::new();
                    let _ = data.extend_from_slice(msg.as_bytes());
                    TX_CHAN.send(TxRequest { data }).await;

                    // Update display
                    display.clear_buffer();
                    Text::new(mac_str, Point::new(1, 10), style)
                        .draw(&mut display)
                        .unwrap();

                    line_buf.clear();
                    let _ = core::fmt::write(&mut line_buf, format_args!("TX #{}", tx_count));
                    Text::new(&line_buf, Point::new(30, 36), style)
                        .draw(&mut display)
                        .unwrap();
                    display.flush().unwrap();

                    tx_count += 1;
                }

                log::info!("PTT released — back to RX");

                // Redraw RX screen
                draw_rx_screen(&mut display, mac_str, &style);
            }
        }
    }
}

fn draw_rx_screen(
    display: &mut Display<'_>,
    mac_str: &str,
    style: &embedded_graphics::mono_font::MonoTextStyle<BinaryColor>,
) {
    display.clear_buffer();
    Text::new(mac_str, Point::new(1, 10), *style)
        .draw(display)
        .unwrap();
    Text::new("RX Listening", Point::new(16, 36), *style)
        .draw(display)
        .unwrap();
    display.flush().unwrap();
}
