//! CAD bench, for the frequency-hopping design: an idle radio finds a
//! transmission by sweeping 50 channels with channel activity detection
//! (CAD). A sweep must fit inside a wake-up preamble, which the 0.4s dwell
//! limit caps, so one channel check has to cost under ~8ms.
//!
//! Found so far (lora-rs 43e89d5): CAD sees preambles only, never payload;
//! 2 symbols at peak 22 is the fastest setting with no false alarms. A
//! retune is one command (retune_for_cad, ~0.6ms), but every CAD ends with
//! the chip in STDBY_RC, which stops the TCXO, so the next CAD waits for it
//! to start again: lora-phy's default wait is 10ms.
//!
//! This run: how short can the TCXO wait be? For each of TCXO_WAKEUPS, the
//! radio is re-initialized with it, then checks channels for REPORT_EVERY.
//! The time per check shows what it saves; the hit rate on the busy channel
//! shows whether the radio still detects reliably (a TCXO that hasn't
//! settled would cost detections).
//!
//! Two roles, by MAC:
//! - TRANSMITTER sends tiny packets back to back on BUSY_HZ: about half of
//!   each is preamble, so CAD has plenty to find
//! - every other board alternates CAD checks on BUSY_HZ and on EMPTY_HZ
//!   (nobody transmits there: false alarms)
//!
//! Desk only: the boards are close, so the signal is strong. How CAD does
//! near the edge (low SNR) needs distance or attenuators.
//!
//! Build & flash (all boards, the role comes from the MAC):
//!   cargo build --bin cad_test
//!   espflash flash -p <PORT> --bootloader <ours> --partition-table ... target/xtensa-esp32s3-espidf/debug/cad_test
//! Watch: python3 scripts/boot-log.py <PORT> 60

use esp_idf_svc::hal::cpu::Core;
use esp_idf_svc::hal::task::block_on;
use esp_idf_svc::hal::task::thread::ThreadSpawnConfiguration;
use lora_phy::mod_params::*;
use lora_phy::sx126x::{self, CADSymbols, Sx1262, Sx126x, TcxoCtrlVoltage};
use lora_phy::LoRa;
use open_oswst::board;
use open_oswst::devices::radio_bus::{self, RadioSpi};
use open_oswst::devices::{fem, radio};
use std::time::{Duration, Instant};

/// The board that transmits: the echo station. Every other one scans
const TRANSMITTER: [u8; 6] = [0xF8, 0x5B, 0x1B, 0xA7, 0x38, 0x90];

/// The busy channel and the empty one. Away from 915 MHz (our app) and 916
/// (the house's Z-Wave)
const BUSY_HZ: u32 = 912_000_000;
const EMPTY_HZ: u32 = 921_000_000;

/// As in the app (devices::radio)
const PREAMBLE_SYMBOLS: u16 = 12;
/// The lowest the SX1262 allows. The FEM adds ~13dB: plenty across a desk
const TX_POWER_DBM: i32 = -9;
/// Tiny packets: ~26ms on air, ~12ms of it preamble
const PAYLOAD_BYTES: usize = 2;

const REPORT_EVERY: Duration = Duration::from_secs(5);

/// CAD: 2 symbols, detection peak 22, minimum 10. From a sweep of settings
/// (lora-rs 43e89d5): half the time of lora-phy's default (8, 20, 10), no
/// false alarms. No setting saw payload, only preambles
const CAD_SETTINGS: (CADSymbols, u8, u8) = (CADSymbols::_2, 22, 10);

/// TCXO start-up waits to try, us. lora-phy's default is 10ms; Semtech's
/// reference design uses 5
const TCXO_WAKEUPS: [u32; 3] = [10_000, 2_000, 1_000];

/// On our own SPI bus and pin interrupts (devices::radio_bus), not
/// esp-idf-hal's
type Radio = LoRa<Sx126x<RadioSpi, radio_bus::Interface, Sx1262>, embassy_time::Delay>;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let board = board::take();
    let _fem = fem::init(board.fem);
    let transmitter = mac() == TRANSMITTER;
    log::info!(
        "cad_test starting as the {}",
        if transmitter {
            "transmitter"
        } else {
            "scanner"
        }
    );

    // The radio gets its own thread on core 1 like the app's, created there:
    // created elsewhere and moved, it hung (see radio_timing.rs)
    ThreadSpawnConfiguration {
        name: Some(c"radio"),
        priority: 10,
        pin_to_core: Some(Core::Core1),
        ..Default::default()
    }
    .set()
    .unwrap();
    let radio_pins = board.radio;
    std::thread::Builder::new()
        .stack_size(32768)
        .spawn(move || {
            block_on(async {
                let mut lora = new_radio(radio_pins).await;
                if transmitter {
                    transmit(&mut lora).await
                } else {
                    scan(&mut lora).await
                }
            })
        })
        .unwrap();
    ThreadSpawnConfiguration::default().set().unwrap();

    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Long packets back to back on BUSY_HZ, forever
async fn transmit(lora: &mut Radio) {
    let mdltn = modulation(lora, BUSY_HZ);
    let mut tx_params = lora
        .create_tx_packet_params(PREAMBLE_SYMBOLS, false, true, false, &mdltn)
        .unwrap();
    let payload = [0x55u8; PAYLOAD_BYTES];
    let mut sent = 0u32;
    let mut since = Instant::now();
    loop {
        lora.prepare_for_tx(&mdltn, &mut tx_params, TX_POWER_DBM, &payload)
            .await
            .unwrap();
        lora.tx().await.unwrap();
        sent += 1;
        if since.elapsed() >= REPORT_EVERY {
            log::info!(
                "TX: {} packets of {}B on {} Hz",
                sent,
                PAYLOAD_BYTES,
                BUSY_HZ
            );
            sent = 0;
            since = Instant::now();
        }
    }
}

/// For each TCXO wait in turn: re-initialize the radio with it, then CAD on
/// the busy channel and the empty one, over and over, for REPORT_EVERY
async fn scan(lora: &mut Radio) {
    let channels = [
        ("busy ", modulation(lora, BUSY_HZ)),
        ("empty", modulation(lora, EMPTY_HZ)),
    ];
    let mut stats = [Stats::new(), Stats::new()];
    loop {
        for wakeup_us in TCXO_WAKEUPS {
            lora.radio_kind_mut().set_tcxo_wakeup_us(wakeup_us);
            lora.init().await.unwrap();
            let (symbols, det_peak, det_min) = CAD_SETTINGS;
            lora.radio_kind_mut()
                .set_cad_params(symbols, det_peak, det_min);
            lora.prepare_for_cad(&channels[0].1).await.unwrap();
            log::info!("TCXO wake-up: {}us", wakeup_us);

            let since = Instant::now();
            while since.elapsed() < REPORT_EVERY {
                for ((_, mdltn), stats) in channels.iter().zip(stats.iter_mut()) {
                    let started = Instant::now();
                    lora.retune_for_cad(mdltn.frequency_in_hz).await.unwrap();
                    let retuned = Instant::now();
                    let hit = lora.cad(mdltn).await.unwrap();
                    stats.record(hit, retuned - started, retuned.elapsed());
                }
            }
            for ((name, _), stats) in channels.iter().zip(stats.iter_mut()) {
                stats.log_and_reset(name);
            }
        }
    }
}

/// Hits and timings for one channel over REPORT_EVERY
struct Stats {
    hits: u32,
    retune_us: Vec<u32>,
    cad_us: Vec<u32>,
}

impl Stats {
    fn new() -> Self {
        // ~10s of checks at ~10ms each, with room to spare: no allocation later
        Stats {
            hits: 0,
            retune_us: Vec::with_capacity(4096),
            cad_us: Vec::with_capacity(4096),
        }
    }

    fn record(&mut self, hit: bool, retune: Duration, cad: Duration) {
        self.hits += hit as u32;
        if self.retune_us.len() < self.retune_us.capacity() {
            self.retune_us.push(retune.as_micros() as u32);
            self.cad_us.push(cad.as_micros() as u32);
        }
    }

    /// e.g. `CAD busy : 480 checks, 96.9% hit | retune p50/max 2.1/2.6ms,
    /// cad p50/max 9.4/10.1ms`
    fn log_and_reset(&mut self, name: &str) {
        let checks = self.retune_us.len();
        let ms = |us: u32| us as f32 / 1000.0;
        let (retune_p50, retune_max) = p50_max(&mut self.retune_us);
        let (cad_p50, cad_max) = p50_max(&mut self.cad_us);
        log::info!(
            "CAD {}: {} checks, {:.1}% hit | retune p50/max {:.1}/{:.1}ms, cad p50/max {:.1}/{:.1}ms",
            name,
            checks,
            self.hits as f32 * 100.0 / checks.max(1) as f32,
            ms(retune_p50),
            ms(retune_max),
            ms(cad_p50),
            ms(cad_max)
        );
        self.hits = 0;
        self.retune_us.clear();
        self.cad_us.clear();
    }
}

fn p50_max(values: &mut [u32]) -> (u32, u32) {
    if values.is_empty() {
        return (0, 0);
    }
    values.sort_unstable();
    (values[values.len() / 2], values[values.len() - 1])
}

fn modulation(lora: &mut Radio, hz: u32) -> ModulationParams {
    lora.create_modulation_params(
        SpreadingFactor::_7,
        Bandwidth::_125KHz,
        CodingRate::_4_5,
        hz,
    )
    .unwrap()
}

/// The same radio setup as devices::radio, on devices::radio_bus
async fn new_radio(p: radio::Peripherals) -> Radio {
    let (spi, iv) = radio_bus::take(p);
    let config = sx126x::Config {
        chip: Sx1262,
        tcxo_ctrl: Some(TcxoCtrlVoltage::Ctrl1V7),
        use_dcdc: true,
        rx_boost: false,
    };
    let radio = Sx126x::new(spi, iv, config)
        .with_oscillator_kept_on()
        .with_unchanged_settings_skipped();
    LoRa::new(radio, false, embassy_time::Delay).await.unwrap()
}

/// This board's WiFi station MAC: the one the logs and /status show
fn mac() -> [u8; 6] {
    let mut mac = [0u8; 6];
    unsafe {
        esp_idf_svc::sys::esp_read_mac(
            mac.as_mut_ptr(),
            esp_idf_svc::sys::esp_mac_type_t_ESP_MAC_WIFI_STA,
        );
    }
    mac
}
