//! Radio turnaround bench: where do the ~45ms per TX go, beyond the 61.7ms of
//! air? The app measured ~1.5ms per SPI command, where the wire time is
//! ~40µs. This finds out which part of the interrupt → wake → poll chain
//! costs it, and whether thread placement changes it.
//!
//! Sends a 26B packet every 160ms, going back to RX after each, like a relay.
//! The packet type is one nobody uses, so the other radios ignore it.
//! The radio runs in its own thread, placed by RADIO_CORE and RADIO_PRIORITY
//! (core 0 / prio 1 is where the app's radio runs today: the main task).
//! It cycles through three loads, PACKETS packets each, forever:
//!   - idle: nothing else running
//!   - codec: the real Codec2, encode + decode every 160ms. Lots of code and
//!     tables, all executed and read from flash through the shared cache
//!   - spin: a tiny loop, CPU-busy for as long as the codec took, every
//!     160ms. It fits in the cache, so it competes for the CPU like the
//!     codec but puts no pressure on the cache
//!
//! If SPI slows under codec but not under spin, it's the flash cache, not
//! scheduling. Both loads are spawned like the app's codec: no core
//! affinity, prio 5.
//!
//! Each run logs each TX step (median / max), and inside them how long SPI
//! transfers and BUSY waits took. "armed" counts BUSY waits where the pin was
//! still high, so the hal had to arm the interrupt and sleep.
//!
//! The radio has to be created on the thread that uses it: ESP-IDF routes a
//! GPIO interrupt to the core that enables it, and the hal installs the GPIO
//! ISR service on the core that first asks. Moved to another core, the radio
//! never sees BUSY or DIO1 interrupts and hangs.
//!
//! Build & flash: cargo build --bin radio_timing && espflash flash -p <PORT> --partition-table target/xtensa-esp32s3-espidf/debug/partition-table.bin target/xtensa-esp32s3-espidf/debug/radio_timing

use embassy_time::{Duration, Ticker};
use embedded_hal_async::digital::Wait;
use embedded_hal_async::spi::{Operation, SpiDevice};
use esp_idf_svc::hal::cpu::{self, Core};
use esp_idf_svc::hal::gpio::{Input, PinDriver, Pull};
use esp_idf_svc::hal::spi::config::Config as SpiConfig;
use esp_idf_svc::hal::spi::{SpiDeviceDriver, SpiDriver, SpiDriverConfig};
use esp_idf_svc::hal::task::block_on;
use esp_idf_svc::hal::task::thread::ThreadSpawnConfiguration;
use esp_idf_svc::hal::units::Hertz;
use lora_phy::iv::GenericSx126xInterfaceVariant;
use lora_phy::mod_params::*;
use lora_phy::sx126x::{self, Sx1262, Sx126x, TcxoCtrlVoltage};
use lora_phy::LoRa;
use open_oswst::board;
use open_oswst::devices::{fem, radio};
use open_oswst_core::codec::{self, CodecRequest, CODEC2_FRAME_SAMPLES, CODEC_REPLY};
use open_oswst_core::codec::{FRAMES_PER_PACKET, PAYLOAD_BYTES};
use open_oswst_core::packet;
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::time::Instant;

/// Where the radio thread runs
const RADIO_CORE: Core = Core::Core0;
const RADIO_PRIORITY: u8 = 1;
/// Packets per run: ~6.4s each
const PACKETS: usize = 40;
/// A packet type nobody handles, so the app on the other radios drops it
const TYPE_BENCH: u8 = 0x1F;

type Iv =
    GenericSx126xInterfaceVariant<PinDriver<'static, esp_idf_svc::hal::gpio::Output>, TimedPin>;
type Radio = LoRa<
    Sx126x<TimedSpi<SpiDeviceDriver<'static, SpiDriver<'static>>>, Iv, Sx1262>,
    embassy_time::Delay,
>;

/// What else is running: one of the LOAD_ values
static LOAD: AtomicU8 = AtomicU8::new(LOAD_IDLE);
const LOAD_IDLE: u8 = 0;
const LOAD_CODEC: u8 = 1;
const LOAD_SPIN: u8 = 2;
const LOADS: [(u8, &str); 3] = [
    (LOAD_IDLE, "idle"),
    (LOAD_CODEC, "codec"),
    (LOAD_SPIN, "spin"),
];
/// How long one encode + decode took, last time: the spin load copies it
static CODEC_US: AtomicU32 = AtomicU32::new(0);

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("radio_timing starting");

    let board = board::take();
    let _fem = fem::init(board.fem);

    // The codec, spawned like the app spawns it (before any spawn config is set)
    let (codec_tx, codec_rx) = std::sync::mpsc::sync_channel::<CodecRequest>(2);
    std::thread::Builder::new()
        .name("codec".into())
        .stack_size(32768)
        .spawn(move || codec::run(codec_rx))
        .unwrap();
    std::thread::Builder::new()
        .name("feeder".into())
        .stack_size(8192)
        .spawn(move || feed_codec(codec_tx))
        .unwrap();
    std::thread::Builder::new()
        .name("spin".into())
        .stack_size(4096)
        .spawn(spin)
        .unwrap();

    ThreadSpawnConfiguration {
        name: Some(c"radio"),
        priority: RADIO_PRIORITY,
        pin_to_core: Some(RADIO_CORE),
        ..Default::default()
    }
    .set()
    .unwrap();
    // std's stack size wins over the spawn config's, so it's set here. The
    // lora-phy futures are big: the app runs them on the 64KB main stack
    let radio_pins = board.radio;
    std::thread::Builder::new()
        .stack_size(32768)
        .spawn(move || block_on(run(radio_pins)))
        .unwrap();
    ThreadSpawnConfiguration::default().set().unwrap();

    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

/// The radio thread: a run under each load in turn, forever.
async fn run(radio_pins: radio::Peripherals) {
    let mut lora = new_radio(radio_pins).await;
    log::info!(
        "LoRa radio initialized, thread on core {:?} prio {}",
        cpu::core(),
        RADIO_PRIORITY
    );
    loop {
        for (load, name) in LOADS {
            LOAD.store(load, Ordering::Relaxed);
            let rows = bench(&mut lora).await;
            report(name, &rows);
        }
    }
}

/// Same radio setup as devices::radio, but with the SPI device and the BUSY
/// pin wrapped so each transfer and wait is timed.
async fn new_radio(p: radio::Peripherals) -> Radio {
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
    let dio1 = TimedPin {
        pin: PinDriver::input(p.dio1, Pull::Floating).unwrap(),
        is_busy: false,
    };
    let busy = TimedPin {
        pin: PinDriver::input(p.busy, Pull::Floating).unwrap(),
        is_busy: true,
    };
    let rf_switch_tx = p.rf_switch_tx.map(|pin| PinDriver::output(pin).unwrap());
    let iv = GenericSx126xInterfaceVariant::new(reset, dio1, busy, None, rf_switch_tx).unwrap();

    let config = sx126x::Config {
        chip: Sx1262,
        tcxo_ctrl: Some(TcxoCtrlVoltage::Ctrl1V7),
        use_dcdc: true,
        rx_boost: false,
    };
    LoRa::new(
        Sx126x::new(TimedSpi(spi), iv, config),
        false,
        embassy_time::Delay,
    )
    .await
    .unwrap()
}

/// Keep the codec as busy as a talker plus a listener: one packet encoded and
/// one decoded every 160ms, while the load is the codec.
fn feed_codec(codec_tx: std::sync::mpsc::SyncSender<CodecRequest>) {
    let pcm: Vec<i16> = (0..FRAMES_PER_PACKET * CODEC2_FRAME_SAMPLES)
        .map(|i| ((i * 7919) % 8000) as i16 - 4000) // any speech-band mush will do
        .collect();
    loop {
        if LOAD.load(Ordering::Relaxed) == LOAD_CODEC {
            let start = Instant::now();
            codec_tx
                .send(CodecRequest::encode([0; 2], pcm.clone().into()))
                .unwrap();
            block_on(CODEC_REPLY.receive());
            codec_tx
                .send(CodecRequest::decode(0, 0, [0x55; PAYLOAD_BYTES]))
                .unwrap();
            block_on(CODEC_REPLY.receive());
            CODEC_US.store(start.elapsed().as_micros() as u32, Ordering::Relaxed);
        }
        std::thread::sleep(std::time::Duration::from_millis(160));
    }
}

/// Keep a CPU as busy as the codec does, every 160ms, while the load is spin,
/// without touching more code or data than fits in the cache.
fn spin() {
    loop {
        if LOAD.load(Ordering::Relaxed) == LOAD_SPIN {
            let busy_us = CODEC_US.load(Ordering::Relaxed) as u128;
            let start = Instant::now();
            let mut x = 0u32;
            while start.elapsed().as_micros() < busy_us {
                x = std::hint::black_box(x.wrapping_mul(1_664_525).wrapping_add(1));
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(160));
    }
}

/// One TX and the return to RX, timed step by step
struct Row {
    standby_us: u32,
    prep_us: u32,
    tx_us: u32,
    back_to_rx_us: u32,
    spi: Counts,
    busy: Counts,
    busy_armed: u32,
}

async fn bench(lora: &mut Radio) -> Vec<Row> {
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

    lora.prepare_for_rx(RxMode::Continuous, &mdltn, &rx_params)
        .await
        .unwrap();
    lora.start_rx().await.unwrap();

    let txid = (unsafe { esp_idf_svc::sys::esp_random() } & 0x7F) as u8;
    let mut rows = Vec::with_capacity(PACKETS);
    let mut ticker = Ticker::every(Duration::from_millis(160));
    for n in 0..PACKETS {
        ticker.next().await;
        let mut data = heapless::Vec::<u8, 26>::new();
        let _ = data.extend_from_slice(&packet::pack(TYPE_BENCH, txid, n as u8 & 0x0F));
        let _ = data.extend_from_slice(&[0xA5; PAYLOAD_BYTES]);

        SPI.reset();
        BUSY.reset();
        BUSY_ARMED.store(0, Ordering::Relaxed);
        let start = Instant::now();
        lora.enter_standby().await.unwrap();
        let standby = Instant::now();
        lora.prepare_for_tx(&mdltn, &mut tx_params, radio::TX_POWER_DBM, &data)
            .await
            .unwrap();
        let prepared = Instant::now();
        lora.tx().await.unwrap();
        let sent = Instant::now();
        lora.prepare_for_rx(RxMode::Continuous, &mdltn, &rx_params)
            .await
            .unwrap();
        lora.start_rx().await.unwrap();
        let listening = Instant::now();

        rows.push(Row {
            standby_us: (standby - start).as_micros() as u32,
            prep_us: (prepared - standby).as_micros() as u32,
            tx_us: (sent - prepared).as_micros() as u32,
            back_to_rx_us: (listening - sent).as_micros() as u32,
            spi: SPI.take(),
            busy: BUSY.take(),
            busy_armed: BUSY_ARMED.load(Ordering::Relaxed),
        });
    }
    rows
}

fn report(load: &str, rows: &[Row]) {
    log::info!(
        "=== core {:?} prio {}, load {} (codec takes {}us), {} packets ===",
        cpu::core(),
        RADIO_PRIORITY,
        load,
        CODEC_US.load(Ordering::Relaxed),
        rows.len()
    );
    let step = |label: &str, us: &dyn Fn(&Row) -> u32| {
        let mut v: Vec<u32> = rows.iter().map(us).collect();
        v.sort_unstable();
        log::info!(
            "  {:10} median {:6}us  max {:6}us",
            label,
            v[v.len() / 2],
            v[v.len() - 1]
        );
    };
    step("standby", &|r| r.standby_us);
    step("prep", &|r| r.prep_us);
    step("tx", &|r| r.tx_us);
    step("back_to_rx", &|r| r.back_to_rx_us);

    let calls = |c: &dyn Fn(&Row) -> Counts| {
        let (n, sum, max) = rows.iter().map(c).fold((0, 0u64, 0), |(n, s, m), c| {
            (n + c.n, s + c.sum_us as u64, m.max(c.max_us))
        });
        (n, sum / n.max(1) as u64, max)
    };
    let (n, avg, max) = calls(&|r| r.spi);
    log::info!(
        "  SPI transfers: {} per TX, avg {}us, max {}us",
        n / rows.len() as u32,
        avg,
        max
    );
    let (n, avg, max) = calls(&|r| r.busy);
    let armed: u32 = rows.iter().map(|r| r.busy_armed).sum();
    log::info!(
        "  BUSY waits:    {} per TX ({} armed), avg {}us, max {}us",
        n / rows.len() as u32,
        armed / rows.len() as u32,
        avg,
        max
    );
}

/// Count, total and worst time of one kind of operation
#[derive(Clone, Copy)]
struct Counts {
    n: u32,
    sum_us: u32,
    max_us: u32,
}

struct Counter {
    n: AtomicU32,
    sum_us: AtomicU32,
    max_us: AtomicU32,
}

impl Counter {
    const fn new() -> Self {
        Counter {
            n: AtomicU32::new(0),
            sum_us: AtomicU32::new(0),
            max_us: AtomicU32::new(0),
        }
    }

    fn record(&self, since: Instant) {
        let us = since.elapsed().as_micros() as u32;
        self.n.fetch_add(1, Ordering::Relaxed);
        self.sum_us.fetch_add(us, Ordering::Relaxed);
        self.max_us.fetch_max(us, Ordering::Relaxed);
    }

    fn reset(&self) {
        self.take();
    }

    fn take(&self) -> Counts {
        Counts {
            n: self.n.swap(0, Ordering::Relaxed),
            sum_us: self.sum_us.swap(0, Ordering::Relaxed),
            max_us: self.max_us.swap(0, Ordering::Relaxed),
        }
    }
}

static SPI: Counter = Counter::new();
static BUSY: Counter = Counter::new();
/// BUSY waits where the pin was still high, so the hal armed the interrupt
static BUSY_ARMED: AtomicU32 = AtomicU32::new(0);

/// The SPI device, timing each transfer from the call to the wake-up after it
struct TimedSpi<S>(S);

impl<S: embedded_hal::spi::ErrorType> embedded_hal::spi::ErrorType for TimedSpi<S> {
    type Error = S::Error;
}

impl<S: SpiDevice> SpiDevice for TimedSpi<S> {
    async fn transaction(&mut self, ops: &mut [Operation<'_, u8>]) -> Result<(), S::Error> {
        let start = Instant::now();
        let result = self.0.transaction(ops).await;
        SPI.record(start);
        result
    }
}

/// DIO1 or BUSY. Only BUSY waits are timed: DIO1 waits are the packet itself.
struct TimedPin {
    pin: PinDriver<'static, Input>,
    is_busy: bool,
}

impl embedded_hal::digital::ErrorType for TimedPin {
    type Error = <PinDriver<'static, Input> as embedded_hal::digital::ErrorType>::Error;
}

impl Wait for TimedPin {
    async fn wait_for_low(&mut self) -> Result<(), Self::Error> {
        if !self.is_busy {
            return Wait::wait_for_low(&mut self.pin).await;
        }
        if self.pin.is_high() {
            BUSY_ARMED.fetch_add(1, Ordering::Relaxed);
        }
        let start = Instant::now();
        let result = Wait::wait_for_low(&mut self.pin).await;
        BUSY.record(start);
        result
    }

    async fn wait_for_high(&mut self) -> Result<(), Self::Error> {
        Wait::wait_for_high(&mut self.pin).await
    }

    async fn wait_for_rising_edge(&mut self) -> Result<(), Self::Error> {
        Wait::wait_for_rising_edge(&mut self.pin).await
    }

    async fn wait_for_falling_edge(&mut self) -> Result<(), Self::Error> {
        Wait::wait_for_falling_edge(&mut self.pin).await
    }

    async fn wait_for_any_edge(&mut self) -> Result<(), Self::Error> {
        Wait::wait_for_any_edge(&mut self.pin).await
    }
}
