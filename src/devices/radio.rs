use embassy_futures::select::{select, select3, Either, Either3};
use esp_idf_svc::hal::gpio::{AnyIOPin, AnyInputPin, AnyOutputPin};
use esp_idf_svc::hal::gpio::{Input, Output, Pin, PinDriver, Pull};
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

// The queues the app talks to; this driver connects them to the SX1262
pub use open_oswst_core::devices::radio::{RxPacket, TxRequest, RX_CHAN, TX_CHAN};

type Iv<'a> = GenericSx126xInterfaceVariant<PinDriver<'a, Output>, PinDriver<'a, Input>>;
type Radio<'a> =
    LoRa<Sx126x<SpiDeviceDriver<'a, SpiDriver<'a>>, Iv<'a>, Sx1262>, embassy_time::Delay>;

/// SX1262 output power. The FEM adds ~13 dB, so this gives ~19 dBm into the
/// Air Buddy amp (max input 20 dBm). At its max 11 dB gain that's ~30 dBm out,
/// ~35 dBm EIRP on a 5 dBi antenna: under the FCC's 36 dBm.
const TX_POWER_DBM: i32 = 6;

/// How often to log the receiver's state. Matches the GPS log, so each check
/// lines up with a position.
const RX_STATE_EVERY_SECS: u64 = 10;

pub struct Peripherals {
    pub spi: SPI2<'static>,
    pub sck: AnyIOPin<'static>,
    pub mosi: AnyIOPin<'static>,
    pub miso: AnyIOPin<'static>,
    pub nss: AnyIOPin<'static>,
    pub reset: AnyOutputPin<'static>,
    pub dio1: AnyInputPin<'static>,
    pub busy: AnyInputPin<'static>,
    /// Front-end TX switch: HIGH while transmitting, LOW otherwise. The
    /// SX1262's DIO2 does the rest of the TX/RX switching on its own.
    pub rf_switch_tx: Option<AnyOutputPin<'static>>,
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

    // Remember the CTX pin's number so the RX state check can read its pad.
    // An output pad reads as 0 unless its input buffer is on, so turn that on.
    let ctx_gpio = p.rf_switch_tx.as_ref().map(|pin| pin.pin() as i32);
    let rf_switch_tx = p.rf_switch_tx.map(|pin| PinDriver::output(pin).unwrap());
    if let Some(gpio) = ctx_gpio {
        unsafe { esp_idf_svc::sys::gpio_input_enable(gpio) };
    }

    let iv = GenericSx126xInterfaceVariant::new(reset, dio1, busy, None, rf_switch_tx).unwrap();

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
        radio_loop(&mut lora, &mdltn, &mut tx_params, &rx_params, ctx_gpio).await;
    }
}

/// Logs what the receiver hears with nobody transmitting (the noise floor),
/// and the level on the FEM's CTX pad (must be 0 to receive through the LNA).
///
/// This is for a fault where a handheld heard ~8 dB worse until power-cycled,
/// with the other direction unchanged. A dead LNA drops the noise floor by
/// about its gain. Interference raises it. CTX stuck high sends RX through the
/// PA side.
async fn log_rx_state(lora: &mut Radio<'_>, ctx_gpio: Option<i32>) {
    // One reading jumps around by a few dB: take several, 1ms apart
    const SAMPLES: i32 = 8;
    let mut sum = 0;
    let mut max = i16::MIN;
    for _ in 0..SAMPLES {
        match lora.get_rssi().await {
            Ok(rssi) => {
                sum += rssi as i32;
                max = max.max(rssi);
            }
            Err(e) => {
                log::error!("RX state: RSSI read failed: {:?}", e);
                return;
            }
        }
        embassy_time::Timer::after_millis(1).await;
    }
    let ctx = match ctx_gpio {
        Some(gpio) => unsafe { esp_idf_svc::sys::gpio_get_level(gpio) },
        None => -1,
    };
    log::info!(
        "RX state: noise avg={} max={} dBm ctx={}",
        sum / SAMPLES,
        max,
        ctx
    );
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
    ctx_gpio: Option<i32>,
) {
    let mut rx_buf = [0u8; 255];
    let mut busy_since: Option<Instant> = None;

    enter_rx(lora, mdltn, rx_params).await;
    log_rx_state(lora, ctx_gpio).await;
    // A fixed deadline, not a fresh 10s timer each pass: busy RX would keep
    // resetting that one and the check would never run
    let mut next_rx_state =
        embassy_time::Instant::now() + embassy_time::Duration::from_secs(RX_STATE_EVERY_SECS);

    loop {
        match select3(
            lora.wait_for_irq(),
            TX_CHAN.receive(),
            embassy_time::Timer::at(next_rx_state),
        )
        .await
        {
            Either3::Third(()) => {
                next_rx_state += embassy_time::Duration::from_secs(RX_STATE_EVERY_SECS);
                // Mid-packet the reading is the packet, not the noise floor.
                // Only a recent preamble counts, as in the CSMA check below:
                // one with no end IRQ would otherwise stop these logs for good
                let mid_packet = busy_since.is_some_and(|t| t.elapsed().as_millis() < 200);
                if !mid_packet {
                    log_rx_state(lora, ctx_gpio).await;
                }
            }
            Either3::First(irq_result) => {
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
                                    len,
                                    rx_ms,
                                    status.rssi,
                                    status.snr
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
            Either3::Second(tx_req) => {
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
                lora.prepare_for_tx(mdltn, tx_params, TX_POWER_DBM, &tx_req.data)
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
