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
use std::sync::atomic::Ordering;
use std::time::Instant;

use crate::{RxPacket, CHAN_USE_PCT, RX_CHAN, TX_CHAN};

/// Compute LoRa time-on-air in milliseconds using the standard formula.
/// Pure function — no hardware access. Uses f32 only for T_sym and fractional preamble.
fn lora_toa_ms(
    sf: u32,
    bw_hz: u32,
    cr_denom: u32,
    preamble: u32,
    payload_bytes: u32,
    explicit_header: bool,
    crc: bool,
) -> u32 {
    let t_sym = (1u32 << sf) as f32 / bw_hz as f32 * 1000.0; // ms
    let t_preamble = (preamble as f32 + 4.25) * t_sym;

    // Low data-rate optimization: enabled when symbol time > 16ms
    let ldro = t_sym > 16.0;
    let de = if ldro { 2 } else { 0 };
    let ih = if explicit_header { 0i32 } else { 1 };
    let crc_bits = if crc { 16i32 } else { 0 };

    let numerator = 8 * payload_bytes as i32 - 4 * sf as i32 + 28 + crc_bits - 20 * ih;
    let denom = 4 * (sf as i32 - de);
    let payload_symbols = if numerator <= 0 {
        8
    } else {
        8 + ((numerator + denom - 1) / denom) * (cr_denom as i32)
    };

    let t_payload = payload_symbols as f32 * t_sym;
    (t_preamble + t_payload).ceil() as u32
}

/// EMA channel occupancy tracker.
/// decay ≈ 0.93 gives ~5s effective window at ~6 packets/sec.
const EMA_DECAY: f32 = 0.93;

struct ChannelTracker {
    avg_air_ms: f32,
    last_update: Instant,
}

impl ChannelTracker {
    fn new() -> Self {
        Self {
            avg_air_ms: 0.0,
            last_update: Instant::now(),
        }
    }

    fn record(&mut self, air_ms: u32) {
        let interval_ms = self.last_update.elapsed().as_millis() as f32;
        self.last_update = Instant::now();
        // air_ms per interval_ms → percentage
        let sample = if interval_ms > 0.0 {
            air_ms as f32 / interval_ms * 100.0
        } else {
            0.0
        };
        self.avg_air_ms = self.avg_air_ms * EMA_DECAY + sample * (1.0 - EMA_DECAY);
        let pct = (self.avg_air_ms as u8).min(100);
        CHAN_USE_PCT.store(pct, Ordering::Relaxed);
    }
}

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

/// Our radio config constants for ToA calculation
const OUR_SF: u32 = 7;
const OUR_BW_HZ: u32 = 125_000;
const OUR_CR_DENOM: u32 = 5;
const OUR_PREAMBLE: u32 = 8;
const OUR_EXPLICIT_HEADER: bool = true;
const OUR_CRC: bool = true;

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
    let mut tracker = ChannelTracker::new();
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
                        busy_since = Some(Instant::now());
                    }
                    Ok(Some(IrqState::Done)) => {
                        busy_since = None;
                        match lora.get_rx_result(rx_params, &mut rx_buf).await {
                            Ok((len, status)) => {
                                let air_ms = lora_toa_ms(
                                    OUR_SF,
                                    OUR_BW_HZ,
                                    OUR_CR_DENOM,
                                    OUR_PREAMBLE,
                                    len as u32,
                                    OUR_EXPLICIT_HEADER,
                                    OUR_CRC,
                                );
                                tracker.record(air_ms);
                                let pct = CHAN_USE_PCT.load(Ordering::Relaxed);
                                log::info!("RX [{}B] air={}ms chan={}%", len, air_ms, pct);

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
                                log::error!("RX result error: {:?}", e);
                            }
                        }
                    }
                    Ok(None) => {
                        // CRC/header error — channel is free, no good packet
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
                if let Some(t) = busy_since {
                    if t.elapsed().as_millis() < 200 {
                        log::info!("TX waiting: channel busy");
                        // Wait for current RX to finish before TX
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

                lora.enter_standby().await.unwrap();
                lora.prepare_for_tx(mdltn, tx_params, 22, &tx_req.data)
                    .await
                    .unwrap();
                lora.tx().await.unwrap();

                let tx_len = tx_req.data.len() as u32;
                let air_ms = lora_toa_ms(
                    OUR_SF,
                    OUR_BW_HZ,
                    OUR_CR_DENOM,
                    OUR_PREAMBLE,
                    tx_len,
                    OUR_EXPLICIT_HEADER,
                    OUR_CRC,
                );
                tracker.record(air_ms);
                let pct = CHAN_USE_PCT.load(Ordering::Relaxed);
                log::info!("TX [{}B] air={}ms chan={}%", tx_len, air_ms, pct);

                // Back to RX continuous
                enter_rx(lora, mdltn, rx_params).await;
            }
        }
    }
}
