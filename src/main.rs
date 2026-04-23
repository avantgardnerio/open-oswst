use embassy_futures::join::join;
use embassy_futures::select::{select, Either};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embedded_graphics::mono_font::ascii::FONT_6X10;
use embedded_graphics::mono_font::MonoTextStyleBuilder;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use esp_idf_svc::hal::adc::continuous::config::Config as AdcContConfig;
use esp_idf_svc::hal::adc::continuous::{AdcDriver as AdcContDriver, AdcMeasurement, Attenuated};
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

// --- Channel message types ---

struct RxPacket {
    data: heapless::Vec<u8, 255>,
    rssi: i16,
    snr: i16,
}

struct TxRequest {
    data: heapless::Vec<u8, 255>,
}

// --- Channel instances (static, ISR-safe) ---

static RX_CHAN: Channel<CriticalSectionRawMutex, RxPacket, 2> = Channel::new();
static TX_CHAN: Channel<CriticalSectionRawMutex, TxRequest, 4> = Channel::new();

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
    let mut button = PinDriver::input(peripherals.pins.gpio0, Pull::Up).unwrap();

    // Continuous ADC for mic on GPIO7 (ADC1_CH6) — DMA at 8kHz
    let adc_config = AdcContConfig::new()
        .sample_freq(Hertz(8000))
        .frame_measurements(320) // 320 samples = 40ms at 8kHz (one Codec2 frame)
        .frames_count(2); // double buffer

    let mut adc = AdcContDriver::new(
        peripherals.adc1,
        &adc_config,
        Attenuated::db12(peripherals.pins.gpio7),
    )
    .unwrap();

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

        // Start continuous ADC (DMA) — buffers accumulate, we read them during TX
        adc.start().unwrap();
        log::info!("ADC DMA started at 8kHz");

        // --- Radio task: exclusively owns lora, mdltn, tx_params, rx_params ---
        let radio = async {
            let mut rx_buf = [0u8; 255];

            loop {
                lora.prepare_for_rx(RxMode::Continuous, &mdltn, &rx_params)
                    .await
                    .unwrap();

                match select(lora.rx(&rx_params, &mut rx_buf), TX_CHAN.receive()).await {
                    Either::First(rx_result) => match rx_result {
                        Ok((len, status)) => {
                            let mut data = heapless::Vec::new();
                            let _ = data.extend_from_slice(&rx_buf[..len as usize]);
                            RX_CHAN
                                .send(RxPacket {
                                    data,
                                    rssi: status.rssi,
                                    snr: status.snr,
                                })
                                .await;
                        }
                        Err(e) => {
                            log::error!("RX error: {:?}", e);
                        }
                    },
                    Either::Second(tx_req) => {
                        // TX request received — cancel RX, transmit, loop back to RX
                        lora.enter_standby().await.unwrap();
                        lora.prepare_for_tx(&mdltn, &mut tx_params, 22, &tx_req.data)
                            .await
                            .unwrap();
                        lora.tx().await.unwrap();
                    }
                }
            }
        };

        // --- App task: owns button, adc, display ---
        let app = async {
            let mut tx_count: u32 = 0;
            let mut line_buf = heapless::String::<64>::new();
            let mut mic_buf = [AdcMeasurement::new(); 320];

            // Show initial RX state
            display.clear_buffer();
            Text::new(&mac_str, Point::new(1, 10), style)
                .draw(&mut display)
                .unwrap();
            Text::new("RX Listening", Point::new(16, 36), style)
                .draw(&mut display)
                .unwrap();
            display.flush().unwrap();

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
                        Text::new(&mac_str, Point::new(1, 10), style)
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
                            Text::new(&mac_str, Point::new(1, 10), style)
                                .draw(&mut display)
                                .unwrap();

                            line_buf.clear();
                            let _ =
                                core::fmt::write(&mut line_buf, format_args!("TX #{}", tx_count));
                            Text::new(&line_buf, Point::new(30, 36), style)
                                .draw(&mut display)
                                .unwrap();
                            display.flush().unwrap();

                            tx_count += 1;
                        }

                        log::info!("PTT released — back to RX");

                        // Redraw RX screen
                        display.clear_buffer();
                        Text::new(&mac_str, Point::new(1, 10), style)
                            .draw(&mut display)
                            .unwrap();
                        Text::new("RX Listening", Point::new(16, 36), style)
                            .draw(&mut display)
                            .unwrap();
                        display.flush().unwrap();
                    }
                }
            }
        };

        // Run radio and app tasks concurrently
        join(radio, app).await;
    });
}
