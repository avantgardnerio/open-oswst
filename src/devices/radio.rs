use core::fmt::Write as _;
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
use std::time::{Duration, Instant};

// The queues the app talks to; this driver connects them to the SX1262
pub use open_oswst_core::devices::radio::{RxPacket, TxRequest, RX_CHAN, TX_CHAN};

type Iv<'a> = GenericSx126xInterfaceVariant<PinDriver<'a, Output>, PinDriver<'a, Input>>;
type Radio<'a> =
    LoRa<Sx126x<SpiDeviceDriver<'a, SpiDriver<'a>>, Iv<'a>, Sx1262>, embassy_time::Delay>;

/// SX1262 output power. The FEM adds ~13 dB, so this gives ~19 dBm into the
/// Air Buddy amp (max input 20 dBm). At its max 11 dB gain that's ~30 dBm out,
/// ~35 dBm EIRP on a 5 dBi antenna: under the FCC's 36 dBm.
pub const TX_POWER_DBM: i32 = 6;

/// Random wait (0..this ms) before each TX, so repeaters that heard the same
/// packet don't all relay at once. OFF (0) for now: with only 3 radios built
/// it just complicates testing. It comes back if N repeaters need to take
/// turns; not if they all relay at the same instant (QMesh-style).
const TX_JITTER_MAX_MS: u32 = 0;

/// A detected preamble is only a maybe: if no valid header follows within
/// this, it wasn't a packet (e.g. we started listening mid-packet and the
/// detector locked onto the payload; then no further IRQ ever comes). The
/// header lands ~20ms after the detection at SF7/125k.
const HEADER_WAIT: Duration = Duration::from_millis(30);

/// After a valid header, the longest until the packet's end IRQ. Our 26B
/// packets end ~41ms after their header at SF7/125k. A longer packet from
/// someone else would still be on the air when this runs out.
const PACKET_END_WAIT: Duration = Duration::from_millis(60);

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
    // DIO1's number too: a stall log reads its pad to see if an IRQ is pending
    let dio1_gpio = p.dio1.pin() as i32;
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
        radio_loop(
            &mut lora,
            &mdltn,
            &mut tx_params,
            &rx_params,
            ctx_gpio,
            dio1_gpio,
        )
        .await;
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

/// What's on the air, as far as this radio can tell
#[derive(Clone, Copy)]
enum Air {
    Clear,
    /// A preamble detected at this time: maybe a packet, see HEADER_WAIT
    Preamble(Instant),
    /// A valid header at this time: a packet really is arriving
    Header(Instant),
}

impl Air {
    /// What an IRQ from the radio tells us about the air
    fn after(state: &Result<Option<IrqState>, RadioError>) -> Air {
        match state {
            Ok(Some(IrqState::PreambleReceived)) => Air::Preamble(Instant::now()),
            Ok(Some(IrqState::HeaderValid)) => Air::Header(Instant::now()),
            // The packet ended: received, or a CRC/header error
            _ => Air::Clear,
        }
    }

    /// When a TX could go ahead, if the air is busy now
    fn busy_until(self) -> Option<Instant> {
        let until = match self {
            Air::Clear => return None,
            Air::Preamble(at) => at + HEADER_WAIT,
            Air::Header(at) => at + PACKET_END_WAIT,
        };
        (until > Instant::now()).then_some(until)
    }
}

/// A packet's header arrived, but no end IRQ (done or error) within
/// PACKET_END_WAIT. Logs the radio's state right then, to tell apart:
/// - dio1=1: an IRQ is pending that the wait never saw (our bug)
/// - dio1=0, still on air: a longer packet than ours
/// - dio1=0, nothing on air: the chip never finished the packet
async fn log_irq_stall(lora: &mut Radio<'_>, header: Instant, dio1_gpio: i32, seen: &str) {
    let dio1 = unsafe { esp_idf_svc::sys::gpio_get_level(dio1_gpio) };
    // Read only: no clear, so this doesn't disturb what it looks at
    let state = lora.get_irq_state().await;
    let rssi = lora.get_rssi().await;
    log::warn!(
        "RADIO IRQ STALL: no end IRQ {}ms after the header. dio1={} irq_state={} rssi={:?} handled:{}",
        header.elapsed().as_millis(),
        dio1,
        irq_name(&state),
        rssi,
        if seen.is_empty() { " none" } else { seen }
    );
}

/// Short name for what lora-phy made of the IRQ register
fn irq_name(state: &Result<Option<IrqState>, RadioError>) -> &'static str {
    match state {
        Ok(Some(IrqState::PreambleReceived)) => "preamble",
        Ok(Some(IrqState::HeaderValid)) => "header",
        Ok(Some(IrqState::Done)) => "done",
        Ok(None) => "none-or-header-error",
        Err(_) => "error",
    }
}

/// Microseconds since boot. The log's own timestamp moves in 10ms ticks.
fn uptime_us() -> i64 {
    unsafe { esp_idf_svc::sys::esp_timer_get_time() }
}

/// An embassy timer deadline for a std Instant
fn deadline(at: Instant) -> embassy_time::Instant {
    let left = at.saturating_duration_since(Instant::now());
    embassy_time::Instant::now() + embassy_time::Duration::from_micros(left.as_micros() as u64)
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
    dio1_gpio: i32,
) {
    let mut rx_buf = [0u8; 255];
    let mut air = Air::Clear;

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
                // Mid-packet the reading is the packet, not the noise floor
                if air.busy_until().is_none() {
                    log_rx_state(lora, ctx_gpio).await;
                }
            }
            Either3::First(irq_result) => {
                // As close to the radio's IRQ as software gets: the relay metric
                // in scripts/relay-test.py times from here
                let irq_us = uptime_us();
                if let Err(e) = irq_result {
                    log::error!("IRQ error: {:?}", e);
                    continue;
                }
                let header_at = match air {
                    Air::Header(at) => Some(at),
                    _ => None,
                };
                let state = lora.get_irq_state().await;
                air = Air::after(&state);
                match state {
                    Ok(Some(IrqState::PreambleReceived)) => log::info!("RX preamble"),
                    Ok(Some(IrqState::HeaderValid)) => log::info!("RX header"),
                    Ok(Some(IrqState::Done)) => {
                        let rx_ms = header_at.map_or(0, |at| at.elapsed().as_millis());
                        match lora.get_rx_result(rx_params, &mut rx_buf).await {
                            Ok((len, status)) => {
                                log::info!(
                                    "RX end [{}B] {}ms after header rssi={} snr={} at={}us",
                                    len,
                                    rx_ms,
                                    status.rssi,
                                    status.snr,
                                    irq_us
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
                                log::error!("RX error [{}ms after header]: {:?}", rx_ms, e);
                            }
                        }
                    }
                    Ok(None) => log::warn!("RX CRC/header error"),
                    Err(e) => log::error!("IRQ state error: {:?}", e),
                }
                // Only the flags just handled: one raised since (e.g. the
                // header, right after the preamble) stays set and fires again
                lora.clear_irq_flags_read().await.unwrap();
                // RX continuous keeps running — no re-setup needed
            }
            Either3::Second(tx_req) => {
                // CSMA: wait for channel clear, then random jitter while listening
                loop {
                    // If a packet may be on the air, wait for it to end
                    if let Some(until) = air.busy_until() {
                        log::info!("TX waiting: channel busy");
                        // IRQs this wait handled, at ms since it started
                        let waiting = Instant::now();
                        let mut seen = heapless::String::<96>::new();
                        let mut until = until;
                        loop {
                            match select(
                                lora.wait_for_irq(),
                                embassy_time::Timer::at(deadline(until)),
                            )
                            .await
                            {
                                Either::First(_) => {
                                    let state = lora.get_irq_state().await;
                                    lora.clear_irq_flags_read().await.unwrap();
                                    let _ = write!(
                                        seen,
                                        " {}@{}",
                                        irq_name(&state),
                                        waiting.elapsed().as_millis()
                                    );
                                    // TODO: on Done, the packet we waited for is
                                    // dropped: never read out or sent to the app. A
                                    // repeater about to TX loses the talker's packet.
                                    air = Air::after(&state);
                                    match air.busy_until() {
                                        Some(later) => until = later,
                                        None => break,
                                    }
                                }
                                Either::Second(()) => {
                                    match air {
                                        Air::Header(at) => {
                                            log_irq_stall(lora, at, dio1_gpio, &seen).await
                                        }
                                        _ => {
                                            log::info!("RX preamble without a header: not a packet")
                                        }
                                    }
                                    air = Air::Clear;
                                    break;
                                }
                            }
                        }
                    }

                    // Random jitter — listen during wait to detect new transmissions
                    if TX_JITTER_MAX_MS == 0 {
                        break;
                    }
                    let jitter_ms =
                        (unsafe { esp_idf_svc::sys::esp_random() } % TX_JITTER_MAX_MS) as u64;
                    match select(
                        lora.wait_for_irq(),
                        embassy_time::Timer::after_millis(jitter_ms),
                    )
                    .await
                    {
                        Either::First(_) => {
                            // Someone started TX during our jitter — handle and retry
                            let state = lora.get_irq_state().await;
                            lora.clear_irq_flags_read().await.unwrap();
                            air = Air::after(&state);
                            continue;
                        }
                        Either::Second(_) => break, // Channel stayed clear, TX now
                    }
                }

                // actually transmit, timing each step: a relay has to fit in
                // the talker's gap, so every ms of turnaround counts
                log::info!("TX start [{}B]", tx_req.data.len());
                let tx_start = Instant::now();
                lora.enter_standby().await.unwrap();
                let standby_us = tx_start.elapsed().as_micros();
                lora.prepare_for_tx(mdltn, tx_params, TX_POWER_DBM, &tx_req.data)
                    .await
                    .unwrap();
                let prepared_us = tx_start.elapsed().as_micros();
                // SetTx → TxDone: includes the TCXO wake-up and PA ramp, not just air
                lora.tx().await.unwrap();
                let sent_us = tx_start.elapsed().as_micros();

                // Back to RX continuous
                enter_rx(lora, mdltn, rx_params).await;
                let rx_us = tx_start.elapsed().as_micros();
                log::info!(
                    "TX end [{}B] {}ms: standby={}us prep={}us tx={}us back_to_rx={}us at={}us",
                    tx_req.data.len(),
                    rx_us / 1000,
                    standby_us,
                    prepared_us - standby_us,
                    sent_us - prepared_us,
                    rx_us - sent_us,
                    uptime_us()
                );
            }
        }
    }
}
