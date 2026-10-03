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
use std::sync::atomic::{AtomicU32, Ordering};
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

/// Preamble length in symbols, TX and RX alike (every radio must agree). Was
/// 8. The 2026-10-03 walk lost its return packets to missed detections, not
/// corruption, with interference setting off the detector between real
/// packets. A longer preamble gives the detector more to lock onto, and a
/// second chance after a false alarm. 12, not 16: at 16 a packet (69.9ms)
/// plus a repeater's relay of it overran the 160ms slot on the desk, and
/// playback underran. 12 costs 4 symbols = 4.1ms per packet at SF7/125k
/// (61.7 -> 65.8ms) and leaves a single repeater ~17ms of slack.
const PREAMBLE_SYMBOLS: u16 = 12;

/// A detected preamble is only a maybe: if no valid header follows within
/// this, it wasn't a packet (e.g. we started listening mid-packet and the
/// detector locked onto the payload; then no further IRQ ever comes). The
/// header landed ~20ms after the detection with an 8-symbol preamble at
/// SF7/125k; the 4 extra symbols add ~4ms.
const HEADER_WAIT: Duration = Duration::from_millis(35);

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
    let busy_gpio = p.busy.pin() as i32;
    let dio1 = PinDriver::input(p.dio1, Pull::Floating).unwrap();
    let busy = PinDriver::input(p.busy, Pull::Floating).unwrap();
    watch_for_stuck_tx(dio1_gpio, busy_gpio);

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

    // Keep the TCXO running between TX and RX: otherwise each TX waits ~10ms
    // for it to start, and so does listening again after it. And don't
    // rewrite settings the chip already has: each costs ~0.5ms of SPI
    let radio = Sx126x::new(spi, iv, config)
        .with_oscillator_kept_on()
        .with_unchanged_settings_skipped();
    let mut lora = LoRa::new(radio, false, embassy_time::Delay).await.unwrap();
    log::info!("LoRa radio initialized");

    let mdltn = lora
        .create_modulation_params(
            SpreadingFactor::_7,
            Bandwidth::_125KHz,
            CodingRate::_4_5,
            915_000_000,
        )
        .unwrap();

    let tx_params = lora
        .create_tx_packet_params(PREAMBLE_SYMBOLS, false, true, false, &mdltn)
        .unwrap();

    let rx_params = lora
        .create_rx_packet_params(PREAMBLE_SYMBOLS, false, 255, true, false, &mdltn)
        .unwrap();

    let mut driver = Driver {
        lora,
        mdltn,
        tx_params,
        rx_params,
        ctx_gpio,
        dio1_gpio,
        rx_buf: [0; 255],
        air: Air::Clear,
    };
    async move { driver.run().await }
}

/// The radio, and what the event handlers share
struct Driver {
    lora: Radio<'static>,
    mdltn: ModulationParams,
    tx_params: PacketParams,
    rx_params: PacketParams,
    ctx_gpio: Option<i32>, // FEM CTX pad, read by the RX state check
    dio1_gpio: i32,        // DIO1 pad, read by the IRQ stall log
    rx_buf: [u8; 255],
    air: Air,
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

impl Driver {
    /// Listen, and handle whichever comes first: an IRQ from the radio, a
    /// packet to send, or the periodic RX state check.
    async fn run(&mut self) {
        self.enter_rx().await;
        self.log_rx_state().await;
        // A fixed deadline, not a fresh 10s timer each pass: busy RX would keep
        // resetting that one and the check would never run
        let every = embassy_time::Duration::from_secs(RX_STATE_EVERY_SECS);
        let mut next_rx_state = embassy_time::Instant::now() + every;

        loop {
            match select3(
                self.lora.wait_for_irq(),
                TX_CHAN.receive(),
                embassy_time::Timer::at(next_rx_state),
            )
            .await
            {
                Either3::First(irq) => self.on_irq(irq).await,
                Either3::Second(tx_req) => self.on_tx(tx_req).await,
                Either3::Third(()) => {
                    next_rx_state += every;
                    self.on_rx_state_due().await;
                }
            }
        }
    }

    /// The radio raised an IRQ while listening.
    async fn on_irq(&mut self, irq: Result<(), RadioError>) {
        // As close to the radio's IRQ as software gets: the relay metric in
        // scripts/relay-test.py times from here
        let irq_us = uptime_us();
        if let Err(e) = irq {
            log::error!("IRQ error: {:?}", e);
            return;
        }
        let header_at = match self.air {
            Air::Header(at) => Some(at),
            _ => None,
        };
        let state = self.lora.get_irq_state().await;
        self.air = Air::after(&state);
        match state {
            Ok(Some(IrqState::PreambleReceived)) => log::info!("RX preamble"),
            Ok(Some(IrqState::HeaderValid)) => log::info!("RX header"),
            Ok(Some(IrqState::Done)) => self.receive(header_at, irq_us).await,
            Ok(None) => log::warn!("RX CRC/header error"),
            Err(e) => log::error!("IRQ state error: {:?}", e),
        }
        // Only the flags just handled: one raised since (e.g. the header,
        // right after the preamble) stays set and fires again. RX continuous
        // keeps running, so there's nothing to set up again.
        self.lora.clear_irq_flags_read().await.unwrap();
    }

    /// A packet arrived: read it out and hand it to the app.
    async fn receive(&mut self, header_at: Option<Instant>, irq_us: i64) {
        let rx_ms = header_at.map_or(0, |at| at.elapsed().as_millis());
        let (len, status) = match self
            .lora
            .get_rx_result(&self.rx_params, &mut self.rx_buf)
            .await
        {
            Ok(result) => result,
            Err(e) => {
                log::error!("RX error [{}ms after header]: {:?}", rx_ms, e);
                return;
            }
        };
        log::info!(
            "RX end [{}B] {}ms after header rssi={} snr={} at={}us",
            len,
            rx_ms,
            status.rssi,
            status.snr,
            irq_us
        );

        let mut data = heapless::Vec::new();
        let _ = data.extend_from_slice(&self.rx_buf[..len as usize]);
        RX_CHAN
            .send(RxPacket {
                data,
                rssi: status.rssi,
                snr: status.snr,
            })
            .await;
    }

    /// The app wants a packet sent: wait for clear air, then send it.
    async fn on_tx(&mut self, tx_req: TxRequest) {
        self.wait_for_clear_air().await;
        self.transmit(&tx_req.data).await;
    }

    /// CSMA: wait out any packet on the air, then the random jitter (if on),
    /// starting over if someone starts sending during it.
    async fn wait_for_clear_air(&mut self) {
        loop {
            self.wait_out_packet().await;
            if self.jitter_stayed_clear().await {
                return;
            }
        }
    }

    /// If a packet may be on the air, wait for its end IRQ, or its deadline.
    async fn wait_out_packet(&mut self) {
        let Some(mut until) = self.air.busy_until() else {
            return;
        };
        log::info!("TX waiting: channel busy");
        // IRQs this wait handled, at ms since it started: for the stall log
        let waiting = Instant::now();
        let mut seen = heapless::String::<96>::new();
        loop {
            let woke = select(
                self.lora.wait_for_irq(),
                embassy_time::Timer::at(deadline(until)),
            )
            .await;
            if let Either::Second(()) = woke {
                self.on_air_deadline(&seen).await;
                return;
            }

            let state = self.lora.get_irq_state().await;
            self.lora.clear_irq_flags_read().await.unwrap();
            let _ = write!(
                seen,
                " {}@{}",
                irq_name(&state),
                waiting.elapsed().as_millis()
            );
            // TODO: on Done, the packet we waited for is dropped: never read
            // out or sent to the app. A repeater about to TX loses the
            // talker's packet.
            self.air = Air::after(&state);
            match self.air.busy_until() {
                Some(later) => until = later,
                None => return,
            }
        }
    }

    /// The air's deadline passed with no end IRQ.
    async fn on_air_deadline(&mut self, seen: &str) {
        match self.air {
            Air::Header(at) => self.log_irq_stall(at, seen).await,
            _ => log::info!("RX preamble without a header: not a packet"),
        }
        self.air = Air::Clear;
    }

    /// The random wait before a TX, listening. False if someone started
    /// sending during it.
    async fn jitter_stayed_clear(&mut self) -> bool {
        if TX_JITTER_MAX_MS == 0 {
            return true;
        }
        let jitter_ms = (unsafe { esp_idf_svc::sys::esp_random() } % TX_JITTER_MAX_MS) as u64;
        let woke = select(
            self.lora.wait_for_irq(),
            embassy_time::Timer::after_millis(jitter_ms),
        )
        .await;
        if let Either::Second(()) = woke {
            return true;
        }
        let state = self.lora.get_irq_state().await;
        self.lora.clear_irq_flags_read().await.unwrap();
        self.air = Air::after(&state);
        false
    }

    /// Send one packet, then go back to listening. Each step is timed: a
    /// relay has to fit in the talker's gap, so every ms of turnaround counts.
    async fn transmit(&mut self, data: &[u8]) {
        log::info!("TX start [{}B]", data.len());
        TX_SINCE_MS.store(uptime_ms(), Ordering::Relaxed);
        let start = Instant::now();
        self.lora.enter_standby().await.unwrap();
        let standby_us = start.elapsed().as_micros();
        self.lora
            .prepare_for_tx(&self.mdltn, &mut self.tx_params, TX_POWER_DBM, data)
            .await
            .unwrap();
        let prepared_us = start.elapsed().as_micros();
        // SetTx → TxDone: air time plus the PA ramp (and the TCXO wake-up, if off)
        self.lora.tx().await.unwrap();
        let sent_us = start.elapsed().as_micros();
        self.enter_rx().await;
        let rx_us = start.elapsed().as_micros();
        log::info!(
            "TX end [{}B] {}ms: standby={}us prep={}us tx={}us back_to_rx={}us at={}us",
            data.len(),
            rx_us / 1000,
            standby_us,
            prepared_us - standby_us,
            sent_us - prepared_us,
            rx_us - sent_us,
            uptime_us()
        );
        TX_SINCE_MS.store(0, Ordering::Relaxed);
    }

    /// Time for the periodic RX state log. Mid-packet the reading would be
    /// the packet, not the noise floor, so skip it then.
    async fn on_rx_state_due(&mut self) {
        if self.air.busy_until().is_none() {
            self.log_rx_state().await;
        }
    }

    async fn enter_rx(&mut self) {
        self.lora
            .prepare_for_rx(RxMode::Continuous, &self.mdltn, &self.rx_params)
            .await
            .unwrap();
        self.lora.start_rx().await.unwrap();
    }

    /// Logs what the receiver hears with nobody transmitting (the noise
    /// floor), and the level on the FEM's CTX pad (must be 0 to receive
    /// through the LNA).
    ///
    /// This is for a fault where a handheld heard ~8 dB worse until
    /// power-cycled, with the other direction unchanged. A dead LNA drops the
    /// noise floor by about its gain. Interference raises it. CTX stuck high
    /// sends RX through the PA side.
    async fn log_rx_state(&mut self) {
        // One reading jumps around by a few dB: take several, 1ms apart
        const SAMPLES: i32 = 8;
        let mut sum = 0;
        let mut max = i16::MIN;
        for _ in 0..SAMPLES {
            match self.lora.get_rssi().await {
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
        let ctx = match self.ctx_gpio {
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

    /// A packet's header arrived, but no end IRQ (done or error) within
    /// PACKET_END_WAIT. Logs the radio's state right then, to tell apart:
    /// - dio1=1: an IRQ is pending that the wait never saw (our bug)
    /// - dio1=0, still on air: a longer packet than ours
    /// - dio1=0, nothing on air: the chip never finished the packet
    async fn log_irq_stall(&mut self, header: Instant, seen: &str) {
        let dio1 = unsafe { esp_idf_svc::sys::gpio_get_level(self.dio1_gpio) };
        // Read only: no clear, so this doesn't disturb what it looks at
        let state = self.lora.get_irq_state().await;
        let rssi = self.lora.get_rssi().await;
        log::warn!(
            "RADIO IRQ STALL: no end IRQ {}ms after the header. dio1={} irq_state={} rssi={:?} handled:{}",
            header.elapsed().as_millis(),
            dio1,
            irq_name(&state),
            rssi,
            if seen.is_empty() { " none" } else { seen }
        );
    }
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

/// When the transmit in progress started (ms since boot), or 0. 32 bits:
/// Xtensa has no 64-bit atomics
static TX_SINCE_MS: AtomicU32 = AtomicU32::new(0);

/// A transmit takes ~70ms. Past this, it's stuck
const TX_STUCK_MS: u32 = 200;

/// The echo station's replays sometimes stop dead: a `TX start` with no `TX
/// end`, and the radio thread silent for 15-27s. lora-phy's tx() has no
/// timeout: it waits for TxDone, and loops while the IRQ it sees isn't one
/// it wants. This logs once per stuck transmit, with the pins that tell
/// those apart: DIO1 high = an IRQ is up but tx() isn't taking it; DIO1 low
/// = TxDone never came. Reads pins only, never the SPI bus, so it can't
/// disturb the radio (lora-phy's IRQ handling must never be interrupted).
fn watch_for_stuck_tx(dio1_gpio: i32, busy_gpio: i32) {
    crate::thread::spawn(
        c"tx_watch",
        3072,
        Some(2),
        Some(esp_idf_svc::hal::cpu::Core::Core0),
        move || {
            let mut logged = false;
            loop {
                std::thread::sleep(Duration::from_millis(100));
                let since = TX_SINCE_MS.load(Ordering::Relaxed);
                if since == 0 {
                    logged = false;
                } else if !logged && uptime_ms().wrapping_sub(since) > TX_STUCK_MS {
                    let (dio1, busy) = unsafe {
                        (
                            esp_idf_svc::sys::gpio_get_level(dio1_gpio),
                            esp_idf_svc::sys::gpio_get_level(busy_gpio),
                        )
                    };
                    log::warn!(
                        "RADIO TX STUCK: {}ms since TX start, dio1={} busy={}",
                        uptime_ms().wrapping_sub(since),
                        dio1,
                        busy
                    );
                    logged = true;
                }
            }
        },
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

/// Milliseconds since boot, for TX_SINCE_MS
fn uptime_ms() -> u32 {
    (uptime_us() / 1000) as u32
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
