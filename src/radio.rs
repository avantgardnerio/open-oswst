use embassy_futures::select::{select, Either};
use esp_idf_svc::hal::gpio::{AnyIOPin, AnyInputPin, AnyOutputPin};
use esp_idf_svc::hal::gpio::{Input, Output, PinDriver, Pull};
use esp_idf_svc::hal::spi::config::Config as SpiConfig;
use esp_idf_svc::hal::spi::{SpiDeviceDriver, SpiDriver, SpiDriverConfig, SPI2};
use esp_idf_svc::hal::units::Hertz;
use lora_phy::iv::GenericSx126xInterfaceVariant;
use lora_phy::mod_params::*;
use lora_phy::mod_traits::IrqState;
use lora_phy::sx126x::{self, Sx1262, Sx126x, TcxoCtrlVoltage};
use lora_phy::LoRa;
use std::future::Future;
use std::time::Instant;

use crate::{RxPacket, RX_CHAN, TX_CHAN};

type Iv<'a> = GenericSx126xInterfaceVariant<PinDriver<'a, Output>, PinDriver<'a, Input>>;
type Radio<'a> =
    LoRa<Sx126x<SpiDeviceDriver<'a, SpiDriver<'a>>, Iv<'a>, Sx1262>, embassy_time::Delay>;

pub struct Peripherals {
    pub spi: SPI2<'static>,
    pub sck: AnyIOPin<'static>,
    pub mosi: AnyIOPin<'static>,
    pub miso: AnyIOPin<'static>,
    pub nss: AnyIOPin<'static>,
    pub reset: AnyOutputPin<'static>,
    pub dio1: AnyInputPin<'static>,
    pub busy: AnyInputPin<'static>,
}

pub async fn init(p: Peripherals) -> impl Future<Output = ()> {
    let spi = SpiDeviceDriver::new_single(
        p.spi,
        p.sck,
        p.mosi,
        Some(p.miso),
        Some(p.nss),
        &SpiDriverConfig::new(),
        &SpiConfig::new().baudrate(Hertz(2_000_000)),
    )
    .unwrap();

    let reset = PinDriver::output(p.reset).unwrap();
    let dio1 = PinDriver::input(p.dio1, Pull::Floating).unwrap();
    let busy = PinDriver::input(p.busy, Pull::Floating).unwrap();

    let iv = GenericSx126xInterfaceVariant::new(reset, dio1, busy, None, None).unwrap();

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

    async move {
        radio_loop(&mut lora, &mdltn, &mut tx_params, &rx_params).await;
    }
}

async fn enter_rx(lora: &mut Radio<'_>, mdltn: &ModulationParams, rx_params: &PacketParams) {
    lora.prepare_for_rx(RxMode::Continuous, mdltn, rx_params)
        .await
        .unwrap();
    lora.start_rx().await.unwrap();
}

async fn radio_loop(
    lora: &mut Radio<'_>,
    mdltn: &ModulationParams,
    tx_params: &mut PacketParams,
    rx_params: &PacketParams,
) {
    let mut rx_buf = [0u8; 255];
    let mut busy_since: Option<Instant> = None;

    enter_rx(lora, mdltn, rx_params).await;

    loop {
        match select(lora.wait_for_irq(), TX_CHAN.receive()).await {
            Either::First(irq_result) => {
                if let Err(e) = irq_result {
                    log::error!("IRQ error: {:?}", e);
                    continue;
                }
                match lora.get_irq_state().await {
                    Ok(Some(IrqState::PreambleReceived)) => {
                        log::info!("RX preamble");
                        busy_since = Some(Instant::now());
                    }
                    Ok(Some(IrqState::Done)) => {
                        let rx_ms = busy_since.map(|t| t.elapsed().as_millis()).unwrap_or(0);
                        busy_since = None;
                        match lora.get_rx_result(rx_params, &mut rx_buf).await {
                            Ok((len, status)) => {
                                log::info!(
                                    "RX end [{}B] {}ms rssi={} snr={}",
                                    len, rx_ms, status.rssi, status.snr
                                );

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
                                log::error!("RX error [{}ms]: {:?}", rx_ms, e);
                            }
                        }
                    }
                    Ok(None) => {
                        let rx_ms = busy_since.map(|t| t.elapsed().as_millis()).unwrap_or(0);
                        log::warn!("RX CRC/header error [{}ms]", rx_ms);
                        busy_since = None;
                    }
                    Err(e) => {
                        log::error!("IRQ state error: {:?}", e);
                        busy_since = None;
                    }
                }
                lora.clear_irq_status().await.unwrap();
                // RX continuous keeps running — no re-setup needed
            }
            Either::Second(tx_req) => {
                // CSMA: wait for channel clear, then random jitter while listening
                loop {
                    // If channel busy, wait for in-progress RX to finish
                    if let Some(t) = busy_since {
                        if t.elapsed().as_millis() < 200 {
                            log::info!("TX waiting: channel busy");
                            loop {
                                lora.wait_for_irq().await.unwrap();
                                let state = lora.get_irq_state().await;
                                lora.clear_irq_status().await.unwrap();
                                match state {
                                    Ok(Some(IrqState::PreambleReceived)) => continue,
                                    _ => break,
                                }
                            }
                        }
                        busy_since = None;
                    }

                    // Random jitter — listen during wait to detect new transmissions
                    let jitter_ms = (unsafe { esp_idf_svc::sys::esp_random() } % 20) as u64;
                    match select(
                        lora.wait_for_irq(),
                        embassy_time::Timer::after_millis(jitter_ms),
                    )
                    .await
                    {
                        Either::First(_) => {
                            // Someone started TX during our jitter — handle and retry
                            if let Ok(Some(IrqState::PreambleReceived)) = lora.get_irq_state().await
                            {
                                busy_since = Some(Instant::now());
                            }
                            lora.clear_irq_status().await.unwrap();
                            continue;
                        }
                        Either::Second(_) => break, // Channel stayed clear, TX now
                    }
                }

                // actually transmit
                log::info!("TX start [{}B]", tx_req.data.len());
                let tx_start = Instant::now();
                lora.enter_standby().await.unwrap();
                lora.prepare_for_tx(mdltn, tx_params, 22, &tx_req.data)
                    .await
                    .unwrap();
                lora.tx().await.unwrap();
                log::info!(
                    "TX end [{}B] {}ms",
                    tx_req.data.len(),
                    tx_start.elapsed().as_millis()
                );

                // Back to RX continuous
                enter_rx(lora, mdltn, rx_params).await;
            }
        }
    }
}
