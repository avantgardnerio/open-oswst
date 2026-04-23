use embassy_futures::select::{select, Either};
use embassy_time::Timer;
use embedded_graphics::mono_font::ascii::FONT_6X10;
use embedded_graphics::mono_font::MonoTextStyleBuilder;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use esp_idf_svc::hal::adc::attenuation::DB_12;
use esp_idf_svc::hal::adc::oneshot::config::AdcChannelConfig;
use esp_idf_svc::hal::adc::oneshot::*;
use esp_idf_svc::hal::gpio::{PinDriver, Pull};
use esp_idf_svc::hal::i2c::config::Config as I2cConfig;
use esp_idf_svc::hal::i2c::I2cDriver;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::spi::config::Config as SpiConfig;
use esp_idf_svc::hal::spi::{SpiDeviceDriver, SpiDriverConfig};
use esp_idf_svc::hal::task::block_on;
use esp_idf_svc::hal::units::Hertz;
use lora_phy::iv::GenericSx126xInterfaceVariant;
use lora_phy::mod_params::*;
use lora_phy::sx126x::{self, Sx1262, Sx126x, TcxoCtrlVoltage};
use lora_phy::LoRa;
use ssd1306::prelude::*;
use ssd1306::{I2CDisplayInterface, Ssd1306};
use std::thread;
use std::time::Duration;

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

    // PRG button on GPIO0 — active LOW with internal pull-up
    let button = PinDriver::input(peripherals.pins.gpio0, Pull::Up).unwrap();

    // ADC for mic on GPIO7 (ADC1_CH6)
    let adc = AdcDriver::new(peripherals.adc1).unwrap();
    let adc_config = AdcChannelConfig {
        attenuation: DB_12,
        ..Default::default()
    };
    let mut mic_pin = AdcChannelDriver::new(&adc, peripherals.pins.gpio7, &adc_config).unwrap();

    // Enable Vext power (GPIO36 LOW = on) — must keep _vext alive or power turns off
    let mut _vext = PinDriver::output(peripherals.pins.gpio36).unwrap();
    _vext.set_low().unwrap();
    thread::sleep(Duration::from_millis(50));

    // Reset OLED (GPIO21)
    let mut oled_rst = PinDriver::output(peripherals.pins.gpio21).unwrap();
    oled_rst.set_low().unwrap();
    thread::sleep(Duration::from_millis(50));
    oled_rst.set_high().unwrap();
    thread::sleep(Duration::from_millis(50));

    // I2C for OLED: SDA=GPIO17, SCL=GPIO18
    let i2c = I2cDriver::new(
        peripherals.i2c0,
        peripherals.pins.gpio17,
        peripherals.pins.gpio18,
        &I2cConfig::default(),
    )
    .unwrap();

    // SPI for LoRa: SCK=9, MOSI=10, MISO=11, NSS=8
    let spi = SpiDeviceDriver::new_single(
        peripherals.spi2,
        peripherals.pins.gpio9,
        peripherals.pins.gpio10,
        Some(peripherals.pins.gpio11),
        Some(peripherals.pins.gpio8),
        &SpiDriverConfig::new(),
        &SpiConfig::new().baudrate(Hertz(2_000_000)),
    )
    .unwrap();

    // LoRa control pins — degrade for type erasure (GenericSx126xInterfaceVariant needs uniform types)
    let lora_reset = PinDriver::output(peripherals.pins.gpio12.degrade_output()).unwrap();
    let lora_dio1 =
        PinDriver::input(peripherals.pins.gpio14.degrade_input(), Pull::Floating).unwrap();
    let lora_busy =
        PinDriver::input(peripherals.pins.gpio13.degrade_input(), Pull::Floating).unwrap();

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
        // OLED init
        let interface = I2CDisplayInterface::new(i2c);
        let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
            .into_buffered_graphics_mode();
        display.init().unwrap();
        display.set_brightness(Brightness::BRIGHTEST).unwrap();
        log::info!("OLED initialized, MAC: {}", mac_str);

        let style = MonoTextStyleBuilder::new()
            .font(&FONT_6X10)
            .text_color(BinaryColor::On)
            .build();

        // LoRa init
        let iv = GenericSx126xInterfaceVariant::new(lora_reset, lora_dio1, lora_busy, None, None)
            .unwrap();

        let config = sx126x::Config {
            chip: Sx1262,
            tcxo_ctrl: Some(TcxoCtrlVoltage::Ctrl1V7),
            use_dcdc: true,
            rx_boost: false,
        };

        let mut lora = LoRa::new(Sx126x::new(spi, iv, config), false, embassy_time::Delay)
            .await
            .unwrap();
        log::info!("LoRa radio initialized");

        let mdltn = lora
            .create_modulation_params(
                SpreadingFactor::_7,
                Bandwidth::_125KHz,
                CodingRate::_4_5,
                915_000_000,
            )
            .unwrap();

        let mut tx_params = lora
            .create_tx_packet_params(8, false, true, false, &mdltn)
            .unwrap();

        let rx_params = lora
            .create_rx_packet_params(8, false, 255, true, false, &mdltn)
            .unwrap();

        // PTT loop — default RX, TX while button held
        let mut tx_count: u32 = 0;
        let mut rx_buf = [0u8; 255];
        let mut line_buf = heapless::String::<64>::new();

        // Show initial RX state with MAC and VU meter
        display.clear_buffer();
        Text::new(&mac_str, Point::new(1, 10), style)
            .draw(&mut display)
            .unwrap();
        Text::new("RX Listening", Point::new(16, 36), style)
            .draw(&mut display)
            .unwrap();
        display.flush().unwrap();

        loop {
            // Sample mic and update VU meter
            let mut peak: u16 = 0;
            for _ in 0..64 {
                let sample = adc.read_raw(&mut mic_pin).unwrap_or(0);
                let deviation = (sample as i32 - 2048).unsigned_abs() as u16;
                if deviation > peak {
                    peak = deviation;
                }
            }
            let bar_width = ((peak as u32) * 128 / 2048).min(128) as u32;

            // Draw VU bar at bottom of screen
            Rectangle::new(Point::new(0, 56), Size::new(128, 8))
                .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
                .draw(&mut display)
                .unwrap();
            if bar_width > 0 {
                Rectangle::new(Point::new(0, 56), Size::new(bar_width, 8))
                    .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
                    .draw(&mut display)
                    .unwrap();
            }
            display.flush().unwrap();

            // Check PTT button (non-blocking)
            if button.is_low() {
                log::info!("PTT pressed — switching to TX");

                // TX while button held
                while button.is_low() {
                    let msg = format!("LORAUDIO #{}", tx_count);
                    log::info!("TX: {}", msg);

                    lora.prepare_for_tx(&mdltn, &mut tx_params, 22, msg.as_bytes())
                        .await
                        .unwrap();
                    lora.tx().await.unwrap();

                    // Update OLED
                    display.clear_buffer();
                    Text::new(&mac_str, Point::new(1, 10), style)
                        .draw(&mut display)
                        .unwrap();

                    line_buf.clear();
                    let _ = core::fmt::write(&mut line_buf, format_args!("TX #{}", tx_count));
                    Text::new(&line_buf, Point::new(30, 36), style)
                        .draw(&mut display)
                        .unwrap();
                    display.flush().unwrap();

                    tx_count += 1;
                    Timer::after_millis(200).await;
                }

                log::info!("PTT released — back to RX");
                lora.sleep(false).await.unwrap();

                // Redraw RX screen
                display.clear_buffer();
                Text::new(&mac_str, Point::new(1, 10), style)
                    .draw(&mut display)
                    .unwrap();
                Text::new("RX Listening", Point::new(16, 36), style)
                    .draw(&mut display)
                    .unwrap();
                display.flush().unwrap();
                continue;
            }

            // Short RX window — listen for a packet with timeout, then loop back for VU update
            lora.prepare_for_rx(RxMode::Continuous, &mdltn, &rx_params)
                .await
                .unwrap();

            match select(lora.rx(&rx_params, &mut rx_buf), Timer::after_millis(50)).await {
                Either::First(rx_result) => match rx_result {
                    Ok((len, status)) => {
                        let msg = core::str::from_utf8(&rx_buf[..len as usize]).unwrap_or("???");
                        log::info!(
                            "RX [{}B] rssi={}dBm snr={}dB: {}",
                            len,
                            status.rssi,
                            status.snr,
                            msg
                        );

                        display.clear_buffer();
                        Text::new(&mac_str, Point::new(1, 10), style)
                            .draw(&mut display)
                            .unwrap();
                        Text::new(msg, Point::new(0, 32), style)
                            .draw(&mut display)
                            .unwrap();

                        line_buf.clear();
                        let _ = core::fmt::write(
                            &mut line_buf,
                            format_args!("RSSI:{} SNR:{}", status.rssi, status.snr),
                        );
                        Text::new(&line_buf, Point::new(0, 48), style)
                            .draw(&mut display)
                            .unwrap();
                        display.flush().unwrap();
                    }
                    Err(e) => {
                        log::error!("RX error: {:?}", e);
                    }
                },
                Either::Second(_) => {
                    // Timeout — no packet, loop back to update VU meter
                    lora.enter_standby().await.unwrap();
                }
            }
        }
    });
}
