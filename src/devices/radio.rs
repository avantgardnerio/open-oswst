use core::fmt::Write as _;
use embassy_futures::select::{select, select4, Either, Either4};
use esp_idf_svc::hal::gpio::{AnyIOPin, AnyInputPin, AnyOutputPin, Pin};
use esp_idf_svc::hal::spi::SPI2;
use lora_phy::mod_params::*;
use lora_phy::mod_traits::IrqState;
use lora_phy::sx126x::{self, CADSymbols, Sx1262, Sx126x, TcxoCtrlVoltage};
use lora_phy::LoRa;
use std::future::Future;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use super::radio_bus::{self, RadioSpi};

// The queues the app talks to; this driver connects them to the SX1262
use open_oswst_core::air;
use open_oswst_core::codec::PACKET_BYTES;
use open_oswst_core::config;
use open_oswst_core::crc;
pub use open_oswst_core::devices::radio::{
    Listen, Rotation, RxPacket, TxRequest, LISTEN, RX_CHAN, TX_CHAN,
};

type Radio = LoRa<Sx126x<RadioSpi, radio_bus::Interface, Sx1262>, embassy_time::Delay>;

/// The most SX1262 output power we ever ask for: every TX here is capped at
/// it, whatever the tx_power_dbm setting says. The FEM adds ~13 dB, so this
/// gives ~19 dBm into the Air Buddy amp (max input 20 dBm: more damages it).
/// At its max 11 dB gain that's ~30 dBm out, ~35 dBm EIRP on a 5 dBi
/// antenna: under the FCC's 36 dBm.
pub const MAX_TX_POWER_DBM: i32 = 6;

/// Random wait (0..this ms) before each TX, so repeaters that heard the same
/// packet don't all relay at once. OFF (0) for now: with only 3 radios built
/// it just complicates testing. It comes back if N repeaters need to take
/// turns; not if they all relay at the same instant (QMesh-style).
const TX_JITTER_MAX_MS: u32 = 0;

/// A packet whose header-to-end time is off a normal one's by more than this
/// was decoded off its channel: its end time is skewed (normal: within ~1 ms)
const SKEWED_AFTER_US: i64 = 3_000;

/// A timed send starts getting ready (standby, then the packet and its
/// settings into the chip: ~2.5 ms measured) this long before its send time,
/// listening until then
const PREPARE_US: i64 = 5_000;
/// Getting a packet ready takes ~2.4 ms: with less than this left before the
/// latest it may start (its send time plus the guard), it's dropped
const PREPARE_AT_LEAST_US: i64 = 3_000;

/// Preamble length in symbols, TX and RX alike: why 12 is in air.rs
const PREAMBLE_SYMBOLS: u16 = air::PREAMBLE_SYMBOLS;

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

/// How often to log the receiver's state (or, sweeping, the sweep's)
const RX_STATE_EVERY_SECS: u64 = 10;

/// Sweeping: CAD for 2 symbols, detection peak 22, minimum 10. The fastest
/// setting with no false alarms on the desk (src/bin/cad_test.rs)
const CAD_SETTINGS: (CADSymbols, u8, u8) = (CADSymbols::_2, 22, 10);

/// Sweeping: how long the TCXO gets to start. Every CAD ends with the chip
/// in STDBY_RC, which stops the TCXO, so each channel waits this. lora-phy's
/// default is 10ms; 2ms is the TCXO's datasheet maximum (KDS DSB321SDN)
const TCXO_WAKEUP_US: u32 = 2_000;

/// Sweeping: a hit with no header within this was a false alarm (or a
/// voice packet's short preamble, caught too late to receive it): back to
/// sweeping. A slot and a packet, so the next packet gets its chance
fn false_alarm_wait() -> Duration {
    Duration::from_micros((air::slot_us() + air::packet_us(PREAMBLE_SYMBOLS, PACKET_BYTES)) as u64)
}

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

/// `frequency_hz`: the channel to send on, and to listen on unless
/// `sweep_hz` lists channels to sweep while idle
pub async fn init(
    p: Peripherals,
    frequency_hz: u32,
    sweep_hz: Vec<u32>,
) -> impl Future<Output = ()> {
    // Pin numbers first: radio_bus takes the pins. DIO1 and BUSY for the
    // stuck-TX log, which reads their pads
    let dio1_gpio = p.dio1.pin() as i32;
    let busy_gpio = p.busy.pin() as i32;
    watch_for_stuck_tx(dio1_gpio, busy_gpio);

    // Remember the CTX pin's number so the RX state check can read its pad.
    // An output pad reads as 0 unless its input buffer is on, so turn that on
    // (after radio_bus has made it an output).
    let ctx_gpio = p.rf_switch_tx.as_ref().map(|pin| pin.pin() as i32);

    // Our own SPI bus and pin interrupts, not esp-idf-hal's: every command
    // and pin wait there costs ~300us of CPU relaying the interrupt through
    // the hal's reactor task, which starves the codec while sweeping
    let (spi, iv) = radio_bus::take(p);
    if let Some(gpio) = ctx_gpio {
        unsafe { esp_idf_svc::sys::gpio_input_enable(gpio) };
    }

    let config = sx126x::Config {
        chip: Sx1262,
        tcxo_ctrl: Some(TcxoCtrlVoltage::Ctrl1V7),
        use_dcdc: true,
        rx_boost: false,
    };

    // Keep the TCXO running between TX and RX: otherwise each TX waits ~10ms
    // for it to start, and so does listening again after it. And don't
    // rewrite settings the chip already has: each costs ~0.5ms of SPI
    let mut radio = Sx126x::new(spi, iv, config)
        .with_oscillator_kept_on()
        .with_unchanged_settings_skipped();
    if !sweep_hz.is_empty() {
        radio.set_tcxo_wakeup_us(TCXO_WAKEUP_US);
        let (symbols, det_peak, det_min) = CAD_SETTINGS;
        radio.set_cad_params(symbols, det_peak, det_min);
    }
    let mut lora = LoRa::new(radio, false, embassy_time::Delay).await.unwrap();
    log::info!("LoRa radio initialized");

    // Must match air.rs's SPREADING_FACTOR, BANDWIDTH_KHZ and CODING_RATE
    let mdltn = lora
        .create_modulation_params(
            SpreadingFactor::_7,
            Bandwidth::_125KHz,
            CodingRate::_4_5,
            frequency_hz,
        )
        .unwrap();

    let tx_params = lora
        // LoRa's CRC off: we send and check our own (open_oswst_core::crc)
        .create_tx_packet_params(PREAMBLE_SYMBOLS, false, false, false, &mdltn)
        .unwrap();

    let rx_params = lora
        .create_rx_packet_params(PREAMBLE_SYMBOLS, false, 255, false, false, &mdltn)
        .unwrap();

    let sweep: Vec<ModulationParams> = sweep_hz
        .iter()
        .map(|&hz| {
            lora.create_modulation_params(
                SpreadingFactor::_7,
                Bandwidth::_125KHz,
                CodingRate::_4_5,
                hz,
            )
            .unwrap()
        })
        .collect();
    let state = if sweep.is_empty() {
        State::Fixed
    } else {
        State::Sweeping {
            next: 0,
            ready: false,
        }
    };

    let mut driver = Driver {
        lora,
        mdltn,
        tx_params,
        rx_params,
        ctx_gpio,
        dio1_gpio,
        rx_buf: [0; 255],
        tx_buf: [0; 255],
        air: Air::Clear,
        header_us: None,
        sweep,
        state,
        sweep_stats: SweepStats::default(),
    };
    async move { driver.run().await }
}

/// The radio, and what the event handlers share
struct Driver {
    lora: Radio,
    mdltn: ModulationParams,
    tx_params: PacketParams,
    rx_params: PacketParams,
    ctx_gpio: Option<i32>, // FEM CTX pad, read by the RX state check
    dio1_gpio: i32,        // DIO1 pad, read by the IRQ stall log
    rx_buf: [u8; 255],
    tx_buf: [u8; 255], // a packet to send, with our CRC after it
    air: Air,
    /// When the packet arriving now raised its header IRQ (µs, as the DIO1
    /// interrupt noted it): its end comes a fixed time later
    header_us: Option<i64>,
    sweep: Vec<ModulationParams>, // channels to sweep while idle; empty: don't
    state: State,
    sweep_stats: SweepStats,
}

/// How the radio listens. With sweep channels, the app moves it between
/// sweeping and locked (LISTEN); a CAD hit locks it on its own, and a hit
/// that comes to nothing (no header) unlocks it
#[derive(Clone, Copy)]
enum State {
    /// RX on our one channel, always (no sweep channels)
    Fixed,
    /// CAD on each sweep channel in turn. `ready`: the chip is set up for
    /// CAD (any TX or RX in between undoes that)
    Sweeping { next: usize, ready: bool },
    /// RX on sweep channel `channel`: a CAD hit there at `hit_at`, or the
    /// app said Hold (channel 0, the start slot) or gave us a rotation.
    /// `heard`: a packet handed to the app since, or the app's word: then it
    /// stays until the app says Sweep. `rotation`: the app's schedule of
    /// channels, bin by bin (`channel` is whichever we're on now)
    Locked {
        channel: usize,
        hit_at: Instant,
        heard: bool,
        rotation: Option<Rotation>,
    },
}

/// The sweep since the last log line
#[derive(Default)]
struct SweepStats {
    checks: u32,
    check_us: u64,
    max_check_us: u32,
    hits: u32,
    false_alarms: u32,
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
    /// packet to send, or the periodic RX state check. Sweeping, it checks
    /// one channel per pass instead.
    async fn run(&mut self) {
        self.resume_listening().await;
        self.on_rx_state_due().await;
        // A fixed deadline, not a fresh 10s timer each pass: busy RX would keep
        // resetting that one and the check would never run
        let every = embassy_time::Duration::from_secs(RX_STATE_EVERY_SECS);
        let mut next_rx_state = embassy_time::Instant::now() + every;

        loop {
            if let State::Sweeping { next, ready } = self.state {
                if embassy_time::Instant::now() >= next_rx_state {
                    next_rx_state += every;
                    self.on_rx_state_due().await;
                }
                self.sweep_step(next, ready).await;
                continue;
            }
            // Locked: the timer also wakes us for a false alarm, or for the
            // rotation's next turn
            let mut wake = next_rx_state;
            for at in [self.false_alarm_at().map(deadline), self.turn_at()]
                .into_iter()
                .flatten()
            {
                wake = wake.min(at);
            }
            match select4(
                self.lora.wait_for_irq(),
                TX_CHAN.receive(),
                LISTEN.wait(),
                embassy_time::Timer::at(wake),
            )
            .await
            {
                Either4::First(irq) => self.on_irq(irq).await,
                Either4::Second(tx_req) => self.on_tx(tx_req).await,
                Either4::Third(listen) => self.on_listen(listen).await,
                Either4::Fourth(()) => {
                    if self.false_alarm_at().is_some_and(|at| Instant::now() >= at) {
                        self.on_false_alarm();
                    }
                    if self
                        .turn_at()
                        .is_some_and(|at| embassy_time::Instant::now() >= at)
                    {
                        self.on_turn().await;
                    }
                    if embassy_time::Instant::now() >= next_rx_state {
                        next_rx_state += every;
                        self.on_rx_state_due().await;
                    }
                }
            }
        }
    }

    /// One channel of the sweep: CAD on it, and lock on if it's busy. The
    /// app's word on how to listen, then a packet waiting to be sent, go
    /// first. Never inside a select: lora-phy says a CAD mustn't be cancelled
    async fn sweep_step(&mut self, next: usize, ready: bool) {
        if let Some(listen) = LISTEN.try_take() {
            self.on_listen(listen).await;
            return;
        }
        if let Ok(tx_req) = TX_CHAN.try_receive() {
            self.on_tx(tx_req).await;
            return;
        }
        let channel = &self.sweep[next];
        if !ready {
            self.lora.prepare_for_cad(channel).await.unwrap();
        }
        let started = Instant::now();
        self.lora
            .retune_for_cad(channel.frequency_in_hz)
            .await
            .unwrap();
        let hit = self.lora.cad(channel).await.unwrap();
        self.sweep_stats.record(hit, started.elapsed());
        if !hit {
            let next = (next + 1) % self.sweep.len();
            self.state = State::Sweeping { next, ready: true };
            return;
        }
        log::info!(
            "SWEEP hit on channel {} ({} Hz): locked",
            next,
            channel.frequency_in_hz
        );
        self.state = State::Locked {
            channel: next,
            hit_at: Instant::now(),
            heard: false,
            rotation: None,
        };
        self.resume_listening().await;
    }

    /// The app's word on how to listen. Without sweep channels, nothing to do
    async fn on_listen(&mut self, listen: Listen) {
        match (listen, self.state) {
            (_, State::Fixed) => {}
            (Listen::Hold, _) => {
                log::info!("SWEEP hold: the start slot");
                self.state = State::Locked {
                    channel: 0,
                    hit_at: Instant::now(),
                    heard: true,
                    rotation: None,
                };
                self.resume_listening().await;
            }
            (Listen::Rotation(rotation), _)
                if rotation
                    .channels
                    .iter()
                    .any(|&channel| channel as usize >= self.sweep.len()) =>
            {
                log::warn!(
                    "LISTEN rotation on channels {:?}: we only have {}, ignored",
                    rotation.channels,
                    self.sweep.len()
                );
            }
            (Listen::Rotation(rotation), state) => {
                log::info!(
                    "LISTEN rotation: channels {} and {}, turns of {}us from {}us",
                    rotation.channels[0],
                    rotation.channels[1],
                    rotation.every_us,
                    rotation.from_us
                );
                // Already listening: stay on this channel until the air's
                // free, then the turn moves us (turn_at). Sweeping: start
                // straight on the rotation's channel
                let on = match state {
                    State::Locked { channel, .. } => Some(channel),
                    _ => None,
                };
                let channel = rotation.channel_at(uptime_us()) as usize;
                self.state = State::Locked {
                    channel: on.unwrap_or(channel),
                    hit_at: Instant::now(),
                    heard: true,
                    rotation: Some(rotation),
                };
                if on.is_none() {
                    self.air = Air::Clear;
                    self.resume_listening().await;
                }
            }
            (Listen::Sweep, State::Sweeping { .. }) => {}
            (Listen::Sweep, State::Locked { channel, .. }) => {
                log::info!("SWEEP unlock channel {}: quiet", channel);
                self.air = Air::Clear;
                self.state = State::Sweeping {
                    next: 0,
                    ready: false,
                };
            }
        }
    }

    /// Locked after a hit with nothing heard since: when it's a false alarm.
    /// Never while a packet is arriving (a preamble or header, until its own
    /// deadline): on a walk the timer ran out in the same 10ms as a header
    /// and threw the packet away
    fn false_alarm_at(&self) -> Option<Instant> {
        match self.state {
            State::Locked {
                hit_at,
                heard: false,
                ..
            } => {
                let at = hit_at + false_alarm_wait();
                Some(self.air.busy_until().map_or(at, |busy| at.max(busy)))
            }
            _ => None,
        }
    }

    /// A hit, but no header followed: back to sweeping, from the next channel
    fn on_false_alarm(&mut self) {
        let State::Locked { channel, .. } = self.state else {
            return;
        };
        self.sweep_stats.false_alarms += 1;
        log::info!("SWEEP unlock channel {}: no header after the hit", channel);
        self.air = Air::Clear;
        self.state = State::Sweeping {
            next: (channel + 1) % self.sweep.len(),
            ready: false,
        };
    }

    /// The radio raised an IRQ while listening.
    async fn on_irq(&mut self, irq: Result<(), RadioError>) {
        // When the radio raised the IRQ, as the DIO1 interrupt noted it,
        // before this task ran. If DIO1 was already high when we started
        // waiting, no interrupt fired: then this task's own time, later. The
        // relay metric in scripts/relay-test.py times from here
        let task_us = uptime_us();
        let isr_us = radio_bus::fired_at_us(self.dio1_gpio, task_us);
        let irq_us = isr_us.unwrap_or(task_us);
        // How long after the interrupt this task got here (measuring)
        let task_late_us = isr_us.map(|isr_us| task_us - isr_us);
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
            Ok(Some(IrqState::PreambleReceived)) => {
                self.header_us = None;
                log::info!("RX preamble");
            }
            Ok(Some(IrqState::HeaderValid)) => {
                self.header_us = Some(irq_us);
                log::info!("RX header");
            }
            Ok(Some(IrqState::Done)) => {
                // Locked: only a good packet keeps us on the channel. The app
                // then says Sweep once it goes quiet. A header alone, or a
                // garbled packet, might not be ours (receive says why), and
                // then nothing would ever unlock us: leave those to the false
                // alarm timer
                let header_us = self.header_us.take();
                let good_packet = self
                    .receive(header_at, header_us, irq_us, task_late_us)
                    .await;
                if good_packet {
                    if let State::Locked { heard, .. } = &mut self.state {
                        *heard = true;
                    }
                }
            }
            // A bad PHY header. A bad CRC is ours to find (receive)
            Ok(None) => log::warn!("RX header error"),
            Err(e) => log::error!("IRQ state error: {:?}", e),
        }
        // Only the flags just handled: one raised since (e.g. the header,
        // right after the preamble) stays set and fires again. RX continuous
        // keeps running, so there's nothing to set up again.
        self.lora.clear_irq_flags_read().await.unwrap();
    }

    /// On a rotation, when to turn: at the next bin's edge, or now if we're
    /// not on the channel it says (just given it, or back from sending).
    /// Never while a packet is arriving: it finishes first. Senders put
    /// packets in the middle of their bins, so one ends a guard (7.1 ms)
    /// before the edge
    fn turn_at(&self) -> Option<embassy_time::Instant> {
        let State::Locked {
            channel,
            rotation: Some(rotation),
            ..
        } = self.state
        else {
            return None;
        };
        let now_us = uptime_us();
        let at_us = if rotation.channel_at(now_us) as usize != channel {
            now_us
        } else if rotation.stays() {
            return None;
        } else {
            rotation.next_turn_us(now_us)
        };
        let at = embassy_time::Instant::from_micros(at_us.max(0) as u64);
        Some(match self.air.busy_until() {
            Some(busy) => at.max(deadline(busy)),
            None => at,
        })
    }

    /// The rotation's turn: on to the channel it says for now
    async fn on_turn(&mut self) {
        let State::Locked {
            channel: on,
            rotation: Some(rotation),
            ..
        } = &mut self.state
        else {
            return;
        };
        let channel = rotation.channel_at(uptime_us()) as usize;
        if channel != *on {
            *on = channel;
            self.air = Air::Clear;
            self.resume_listening().await;
        }
    }

    /// The hop channel we're receiving on: the one locked onto, or the start
    /// slot (0) for a radio that doesn't sweep
    fn listening_on(&self) -> u8 {
        match self.state {
            State::Locked { channel, .. } => channel as u8,
            _ => 0,
        }
    }

    /// A packet arrived: read it out and hand it to the app, flagged if its
    /// CRC failed. True only for a good packet: only a good packet keeps a
    /// lock (a garbled one might not be ours: at point-blank range a
    /// repeater's relay on the next channel arrived garbled but of our
    /// length, bench 2026-10-05)
    /// `task_late_us`: how long after the interrupt the radio task got to it,
    /// None if no interrupt fired (and `irq_us` is the task's own time)
    /// `header_us`: when its header IRQ fired, if we saw it: the time from
    /// there to the end says whether the end time can be trusted
    async fn receive(
        &mut self,
        header_at: Option<Instant>,
        header_us: Option<i64>,
        irq_us: i64,
        task_late_us: Option<i64>,
    ) -> bool {
        let rx_ms = header_at.map_or(0, |at| at.elapsed().as_millis());
        let (len, status) = match self
            .lora
            .get_rx_result(&self.rx_params, &mut self.rx_buf)
            .await
        {
            Ok(result) => result,
            Err(e) => {
                log::error!("RX error [{}ms after header]: {:?}", rx_ms, e);
                return false;
            }
        };
        // Our CRC, not LoRa's: a header corrupted into "no CRC" still gets
        // checked here. A failed packet goes to the app too, flagged: its
        // bytes are garbage, but it was on the air, and the app uses that
        // for timing
        let (packet, crc_ok) = crc::split(&self.rx_buf[..len as usize]);
        // Decoded off its channel, a packet ends at a skewed time (in LoRa a
        // frequency offset looks like a time offset): its header-to-end time
        // gives it away. Without the header's time, we can't tell
        let timing_ok = header_us.is_none_or(|header_us| {
            let expected_us = air::after_header_us(packet.len()) as i64;
            (irq_us - header_us - expected_us).abs() <= SKEWED_AFTER_US
        });
        if crc_ok {
            log::info!(
                "RX end [{}B] {}ms after header rssi={} snr={} at={}us {}{}",
                packet.len(),
                rx_ms,
                status.rssi,
                status.snr,
                irq_us,
                TaskLate(task_late_us),
                if timing_ok { "" } else { " SKEWED" }
            );
        } else {
            log::warn!(
                "RX CRC error [{}B] {}ms after header rssi={} snr={} at={}us {}{}",
                len,
                rx_ms,
                status.rssi,
                status.snr,
                irq_us,
                TaskLate(task_late_us),
                if timing_ok { "" } else { " SKEWED" }
            );
        }

        let mut data = heapless::Vec::new();
        let _ = data.extend_from_slice(packet);
        RX_CHAN
            .send(RxPacket {
                data,
                rssi: status.rssi,
                snr: status.snr,
                channel: self.listening_on(),
                crc_ok,
                end_us: irq_us,
                timing_ok,
            })
            .await;
        crc_ok
    }

    /// The app wants a packet sent: wait for clear air, then send it.
    async fn on_tx(&mut self, tx_req: TxRequest) {
        if tx_req.clear_air_first {
            self.wait_for_clear_air().await;
        }
        self.transmit(
            &tx_req.data,
            tx_req.preamble,
            tx_req.channel,
            tx_req.send_at_us,
        )
        .await;
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
    /// `preamble`: symbols, if not PREAMBLE_SYMBOLS. `send_at_us`: when to
    /// start (µs since boot): we keep listening until just before it, get
    /// the packet ready, then send at that microsecond. So late it would
    /// leave its bin, it's dropped: a packet out of its bin is worse than none
    async fn transmit(
        &mut self,
        data: &[u8],
        preamble: Option<u16>,
        channel: u8,
        send_at_us: Option<i64>,
    ) {
        // Listening right up to the moment we must start getting ready.
        // Nothing is logged from here until the packet is on the air: a log
        // line costs ~1.4 ms, enough to make a send miss its time
        let mut woke_late_us = 0;
        if let Some(at_us) = send_at_us {
            until_us(at_us - PREPARE_US).await;
            woke_late_us = uptime_us() - (at_us - PREPARE_US);
            // Up to the guard late it still fits its bin, so the latest it may
            // start is its send time plus the guard. The wait above can wake a
            // few ms late (the radio task gets the CPU late now and then)
            let left_us = at_us + air::guard_us() as i64 - uptime_us();
            if left_us < PREPARE_AT_LEAST_US {
                log::warn!(
                    "TX [{}B] dropped: {}us before the latest it may start, too late to get ready (woke {}us late)",
                    data.len(),
                    left_us,
                    woke_late_us
                );
                return;
            }
        }
        // A sweeping radio can send on any of its channels; one that doesn't
        // sweep has only the start slot. Off the channel we listen on, this
        // costs one frequency command now and one going back to listening
        let mdltn = self.sweep.get(channel as usize).unwrap_or(&self.mdltn);
        let mut other_params;
        let params = match preamble {
            Some(symbols) => {
                other_params = self
                    .lora
                    .create_tx_packet_params(symbols, false, false, false, mdltn)
                    .unwrap();
                &mut other_params
            }
            None => &mut self.tx_params,
        };
        // Our CRC after the data (LoRa's is off: see receive)
        let framed = data.len() + crc::CRC_BYTES;
        if framed > self.tx_buf.len() {
            log::error!(
                "TX [{}B] too long for a packet and its CRC: dropped",
                data.len()
            );
            return;
        }
        self.tx_buf[..data.len()].copy_from_slice(data);
        self.tx_buf[data.len()..framed].copy_from_slice(&crc::crc16(data).to_be_bytes());
        let tx_power_dbm = config::TX_POWER_DBM.get().min(MAX_TX_POWER_DBM);
        TX_SINCE_MS.store(uptime_ms(), Ordering::Relaxed);
        let start = Instant::now();
        self.lora.enter_standby().await.unwrap();
        let standby_us = start.elapsed().as_micros();
        self.lora
            .prepare_for_tx(mdltn, params, tx_power_dbm, &self.tx_buf[..framed])
            .await
            .unwrap();
        let prepared_us = start.elapsed().as_micros();
        // Ready: wait for the send time itself
        let late_us = match send_at_us {
            Some(at_us) => {
                until_us(at_us).await;
                uptime_us() - at_us
            }
            None => 0,
        };
        // Past its send time by more than the guard (air::guard_us, 7.1 ms):
        // dropped rather than sent out of its bin. Up to the guard late, it
        // still fits inside its bin: write tight, but don't throw away what
        // fits
        if late_us > air::guard_us() as i64 {
            log::warn!(
                "TX [{}B] dropped: would start {}us after its send time, past the guard (ready in time: woke {}us late to get ready, standby={}us prep={}us; the wait for the send time woke late)",
                data.len(),
                late_us,
                woke_late_us,
                standby_us,
                prepared_us - standby_us
            );
            self.resume_listening().await;
            TX_SINCE_MS.store(0, Ordering::Relaxed);
            return;
        }
        let waited_us = start.elapsed().as_micros() - prepared_us;
        // SetTx → TxDone: air time plus the PA ramp (and the TCXO wake-up, if off)
        self.lora.tx().await.unwrap();
        let sent_us = start.elapsed().as_micros();
        self.resume_listening().await;
        let rx_us = start.elapsed().as_micros();
        log::info!(
            "TX end [{}B] {}ms: standby={}us prep={}us tx={}us back_to_rx={}us at={}us wait={}us late={}us{}{}",
            data.len(),
            rx_us / 1000,
            standby_us,
            prepared_us - standby_us,
            sent_us - prepared_us - waited_us,
            rx_us - sent_us,
            uptime_us(),
            waited_us,
            late_us,
            Preamble(preamble),
            Channel(channel)
        );
        TX_SINCE_MS.store(0, Ordering::Relaxed);
    }

    /// Time for the periodic RX state log. Mid-packet the reading would be
    /// the packet, not the noise floor, so skip it then. Sweeping, the chip
    /// isn't receiving: the sweep's numbers instead.
    async fn on_rx_state_due(&mut self) {
        if let State::Sweeping { .. } = self.state {
            self.sweep_stats.log_and_reset(self.sweep.len());
        } else if self.air.busy_until().is_none() {
            self.log_rx_state().await;
        }
    }

    /// Back to listening after a TX or a lock: RX on our channel, or on the
    /// one locked onto (on a rotation, the one it says for now). Sweeping
    /// needs nothing now; the next CAD sets up.
    async fn resume_listening(&mut self) {
        if let State::Locked {
            channel,
            rotation: Some(rotation),
            ..
        } = &mut self.state
        {
            *channel = rotation.channel_at(uptime_us()) as usize;
        }
        let channel = match self.state {
            State::Fixed => &self.mdltn,
            State::Locked { channel, .. } => &self.sweep[channel],
            State::Sweeping { next, .. } => {
                self.state = State::Sweeping { next, ready: false };
                return;
            }
        };
        self.lora
            .prepare_for_rx(RxMode::Continuous, channel, &self.rx_params)
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

impl SweepStats {
    fn record(&mut self, hit: bool, took: Duration) {
        let us = took.as_micros() as u32;
        self.checks += 1;
        self.check_us += us as u64;
        self.max_check_us = self.max_check_us.max(us);
        self.hits += hit as u32;
    }

    /// e.g. `SWEEP: 1370 checks, 7.3ms avg (max 8.1) = 36ms per 5 channels,
    /// 2 hits, 0 false alarms`
    fn log_and_reset(&mut self, channels: usize) {
        let avg_us = self.check_us / self.checks.max(1) as u64;
        log::info!(
            "SWEEP: {} checks, {:.1}ms avg (max {:.1}) = {}ms per {} channels, {} hits, {} false alarms",
            self.checks,
            avg_us as f32 / 1000.0,
            self.max_check_us as f32 / 1000.0,
            avg_us * channels as u64 / 1000,
            channels,
            self.hits,
            self.false_alarms
        );
        *self = SweepStats::default();
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

/// The echo station's replays sometimes stop dead: a transmit with no `TX
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
/// For the TX log: the preamble, if not the usual one (a wake-up's)
struct Preamble(Option<u16>);

impl core::fmt::Display for Preamble {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self.0 {
            Some(symbols) => write!(f, " preamble={}", symbols),
            None => Ok(()),
        }
    }
}

/// For the TX log: the channel, if not the start slot
struct Channel(u8);

impl core::fmt::Display for Channel {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self.0 {
            0 => Ok(()),
            channel => write!(f, " ch={}", channel),
        }
    }
}

/// For the RX log: how long after the DIO1 interrupt the radio task got to
/// it, or that no interrupt fired (DIO1 was already high)
struct TaskLate(Option<i64>);

impl core::fmt::Display for TaskLate {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self.0 {
            Some(late_us) => write!(f, "task+{}us", late_us),
            None => write!(f, "no-isr"),
        }
    }
}

/// Wait until `at_us` (µs since boot); at once if it's past. embassy-time
/// counts esp_timer's µs too, so this is the same clock
async fn until_us(at_us: i64) {
    embassy_time::Timer::at(embassy_time::Instant::from_micros(at_us.max(0) as u64)).await;
}

fn uptime_us() -> i64 {
    unsafe { esp_idf_svc::sys::esp_timer_get_time() }
}

/// An embassy timer deadline for a std Instant
fn deadline(at: Instant) -> embassy_time::Instant {
    let left = at.saturating_duration_since(Instant::now());
    embassy_time::Instant::now() + embassy_time::Duration::from_micros(left.as_micros() as u64)
}
