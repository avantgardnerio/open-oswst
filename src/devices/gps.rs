//! GPS: the L76K on the Heltec's GNSS connector. A thread reads its NMEA and
//! keeps the latest time and position; callers just ask for `latest()`.
//! Parsing and the `Fix` type live in the core crate's `devices::gps`.

use esp_idf_svc::hal::delay::TickType;
use esp_idf_svc::hal::gpio::{AnyIOPin, AnyOutputPin, PinDriver};
use esp_idf_svc::hal::uart::{config::Config, UartDriver, UART1};
use esp_idf_svc::hal::units::Hertz;
use open_oswst_core::agnss;
use open_oswst_core::config;
use open_oswst_core::devices::gps::{apply_nmea, Fix};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// No sentence for this long: treat the GPS as gone
const STALE: Duration = Duration::from_secs(3);
/// After power-on, before the GPS takes input (Bangle.js 2 waits 0.5s)
const BOOT_WAIT: Duration = Duration::from_millis(1000);
/// Our fix is kept for the next boot this often (and on the first)
const KEEP_FIX_EVERY: Duration = Duration::from_secs(30 * 60);

/// Assisted GPS: how often to ask the GPS what orbits it holds
/// (NAV-GPSINFO), before and after the first fix
const INFO_EVERY: Duration = Duration::from_secs(10);
const INFO_AFTER_FIX: Duration = Duration::from_secs(60);
/// The longest CASIC payload we collect: NAV-GPSINFO for 32 satellites
const MAX_FRAME_PAYLOAD: usize = 8 + 12 * 32;

/// A binary CASIC frame from the GPS: log what NAV-GPSINFO says (the
/// rest, ACKs to our CFG messages, needn't be)
fn on_frame(frame: &[u8]) {
    let payload = &frame[6..frame.len() - 4];
    if agnss::frame(frame[4], frame[5], payload) != frame {
        return; // bad checksum
    }
    if (frame[4], frame[5]) == (0x01, 0x20) {
        if let Some(info) = agnss::read_gps_info(payload) {
            log::info!(
                "GPS orbits: {} listed, {} from ephemeris, {} almanac, {} invalid; {} heard, {} used",
                info.listed,
                info.ephemeris,
                info.almanac,
                info.invalid,
                info.heard,
                info.used
            );
        }
    }
}

/// Bytes for the GPS (assisted-GPS messages): the GPS thread owns the UART
/// and writes them between NMEA lines
static OUTBOX: OnceLock<Mutex<Sender<Vec<u8>>>> = OnceLock::new();

/// Queue bytes for the GPS. Dropped if the GPS thread isn't running
pub fn send(bytes: Vec<u8>) {
    if let Some(outbox) = OUTBOX.get() {
        let _ = outbox.lock().unwrap().send(bytes);
    }
}

pub struct Peripherals {
    pub uart: UART1<'static>,
    pub tx: AnyIOPin<'static>,        // ESP -> GPS (GPIO38)
    pub rx: AnyIOPin<'static>,        // GPS -> ESP (GPIO39)
    pub power: AnyOutputPin<'static>, // VGNSS_Ctrl, active LOW (GPIO34)
    pub reset: AnyOutputPin<'static>, // active LOW (GPIO42)
    pub wake: AnyOutputPin<'static>,  // HIGH = stay awake (GPIO40)
}

pub struct Gps {
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    fix: Fix,
    updated: Option<Instant>,
}

impl open_oswst_core::devices::gps::Gps for Gps {
    fn latest(&self) -> Option<Fix> {
        let state = self.state.lock().unwrap();
        state
            .updated
            .filter(|at| at.elapsed() < STALE)
            .map(|_| state.fix)
    }
}

pub fn init(p: Peripherals) -> Gps {
    let state = Arc::new(Mutex::new(State::default()));
    let shared = state.clone();
    // 6KB: reading /data (assisted GPS) overflowed 4KB; ~2.9KB used at
    // worst with it (Stack free log, 2026-10-04). Stacks come out of the
    // heap, so no bigger than that. Gone once this thread becomes a task on
    // the main executor
    crate::thread::spawn(c"gps", 6144, None, None, move || run(p, shared));
    Gps { state }
}

fn run(p: Peripherals, state: Arc<Mutex<State>>) {
    let mut power = PinDriver::output(p.power).unwrap();
    power.set_low().unwrap();
    let mut reset = PinDriver::output(p.reset).unwrap();
    reset.set_high().unwrap();
    let mut wake = PinDriver::output(p.wake).unwrap();
    wake.set_high().unwrap();

    let uart = UartDriver::new(
        p.uart,
        p.tx,
        p.rx,
        Option::<AnyIOPin>::None,
        Option::<AnyIOPin>::None,
        &Config::new().baudrate(Hertz(9600)),
    )
    .unwrap();
    log::info!("GPS powered, reading NMEA");

    let (outbox, inbox): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = mpsc::channel();
    let _ = OUTBOX.set(Mutex::new(outbox));
    std::thread::sleep(BOOT_WAIT);
    if config::GPS_COLD_START.is_on() {
        let _ = uart.write(&agnss::nmea("PCAS10,2"));
        log::info!("GPS: cold start (gps_cold_start flag)");
        std::thread::sleep(BOOT_WAIT);
    }
    // Ask for its firmware version: a TXT reply proves our transmit line
    // reaches the GPS (nothing else we send gets an answer we can see)
    let _ = uart.write(&agnss::nmea("PCAS06,0"));
    let assisted = config::AGNSS.is_on();
    if assisted {
        crate::agnss::seed_at_boot();
    }
    let powered = Instant::now();
    let mut first_fix_logged = false;
    // Without assisted GPS there are no kept orbits to send
    let mut orbits_sent = !assisted;
    let mut fix_kept: Option<Instant> = None;

    let mut last_txt = String::with_capacity(96);
    let mut frame: Vec<u8> = Vec::with_capacity(6 + MAX_FRAME_PAYLOAD + 4);
    let mut info_polled: Option<Instant> = None;
    let mut line = Vec::with_capacity(96);
    let mut buf = [0u8; 64];
    loop {
        // Anything for the GPS goes out first (a whole file of orbits is
        // ~2.7KB: ~3s at 9600 baud, while NMEA waits in the RX buffer)
        while let Ok(bytes) = inbox.try_recv() {
            let mut sent = 0;
            while sent < bytes.len() {
                match uart.write(&bytes[sent..]) {
                    Ok(n) => sent += n,
                    Err(e) => {
                        log::warn!("GPS: write failed: {}", e);
                        break;
                    }
                }
            }
        }
        // The clock is set (from the GPS's own time, or NTP): kept orbits
        // can be judged by age now
        if !orbits_sent && crate::clock::is_set() {
            orbits_sent = true;
            crate::agnss::send_kept_orbits();
        }
        // Assisted GPS: ask what orbits it holds (NAV-GPSINFO), every
        // INFO_EVERY until the first fix, then every INFO_AFTER_FIX
        let every = if first_fix_logged {
            INFO_AFTER_FIX
        } else {
            INFO_EVERY
        };
        if assisted && info_polled.is_none_or(|at| at.elapsed() >= every) {
            info_polled = Some(Instant::now());
            let _ = uart.write(&agnss::poll_gps_info());
        }
        let n = uart
            .read(&mut buf, TickType::new_millis(200).ticks())
            .unwrap_or(0);
        for &byte in &buf[..n] {
            // A binary CASIC frame (BA CE, length, class, id, payload,
            // checksum) among the NMEA lines: collect it whole
            if !frame.is_empty() {
                frame.push(byte);
                if frame.len() == 2 && byte != 0xCE {
                    frame.clear(); // a stray 0xBA, not a frame
                } else if frame.len() >= 4 {
                    let len = u16::from_le_bytes([frame[2], frame[3]]) as usize;
                    if len > MAX_FRAME_PAYLOAD {
                        frame.clear();
                    } else if frame.len() == 6 + len + 4 {
                        on_frame(&frame);
                        frame.clear();
                    }
                }
                continue;
            }
            if byte == 0xBA {
                frame.push(byte);
                continue;
            }
            match byte {
                b'\n' => {
                    if let Ok(sentence) = std::str::from_utf8(&line) {
                        // The GPS's own text (firmware version, antenna state),
                        // once per change: it repeats ANTENNA OPEN every
                        // second (normal for our passive patch antenna)
                        if sentence.get(3..6) == Some("TXT") && sentence != last_txt {
                            log::info!("GPS says: {}", sentence.trim_end());
                            last_txt.clear();
                            last_txt.push_str(sentence);
                        }
                        let mut state = state.lock().unwrap();
                        let mut position = None;
                        if apply_nmea(&mut state.fix, sentence.trim_end()) {
                            state.updated = Some(Instant::now());
                            // The first UTC date and time sets the clock
                            if let (Some(date), Some(time)) = (state.fix.date, state.fix.time) {
                                if !crate::clock::is_set() {
                                    crate::clock::set_from_gps(date, time);
                                }
                            }
                            position = state.fix.position;
                        }
                        // Not while holding the fix: keeping it writes flash
                        drop(state);
                        if position.is_some() && !first_fix_logged {
                            first_fix_logged = true;
                            log::info!(
                                "GPS: first fix {} s after power-on (assisted {})",
                                powered.elapsed().as_secs(),
                                if assisted { "on" } else { "off" }
                            );
                        }
                        // A fix: keep it for the next boot, now and then
                        if let Some((lat, lon)) = position {
                            if fix_kept.is_none_or(|at| at.elapsed() > KEEP_FIX_EVERY) {
                                fix_kept = Some(Instant::now());
                                crate::agnss::keep_fix(lat, lon);
                            }
                        }
                    }
                    line.clear();
                }
                _ if line.len() < 96 => line.push(byte),
                _ => line.clear(), // garbage: too long to be NMEA
            }
        }
    }
}
