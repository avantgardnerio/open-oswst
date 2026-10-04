//! GPS: the L76K on the Heltec's GNSS connector. A thread reads its NMEA and
//! keeps the latest time and position; callers just ask for `latest()`.
//! Parsing and the `Fix` type live in the core crate's `devices::gps`.

use esp_idf_svc::hal::delay::TickType;
use esp_idf_svc::hal::gpio::{AnyIOPin, AnyOutputPin, PinDriver};
use esp_idf_svc::hal::uart::{config::Config, UartDriver, UART1};
use esp_idf_svc::hal::units::Hertz;
use open_oswst_core::devices::gps::{apply_nmea, command, Fix};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// No sentence for this long: treat the GPS as gone
const STALE: Duration = Duration::from_secs(3);

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
    // ~2.5KB used at worst (Stack free log, 2026-10-02)
    crate::thread::spawn(c"gps", 4096, None, None, move || run(p, shared));
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
    // Its firmware version comes back as TXT lines (logged below): proof
    // that what we send reaches it
    let _ = uart.write(&command("PCAS06,0"));
    let powered = Instant::now();
    let mut first_fix_logged = false;
    let mut last_txt = String::with_capacity(96);

    let mut line = Vec::with_capacity(96);
    let mut buf = [0u8; 64];
    loop {
        let n = uart
            .read(&mut buf, TickType::new_millis(1000).ticks())
            .unwrap_or(0);
        for &byte in &buf[..n] {
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
                        if apply_nmea(&mut state.fix, sentence.trim_end()) {
                            state.updated = Some(Instant::now());
                            // The first UTC date and time sets the clock
                            if let (Some(date), Some(time)) = (state.fix.date, state.fix.time) {
                                if !crate::clock::is_set() {
                                    crate::clock::set_from_gps(date, time);
                                }
                            }
                            if state.fix.position.is_some() && !first_fix_logged {
                                first_fix_logged = true;
                                log::info!(
                                    "GPS: first fix {} s after power-on",
                                    powered.elapsed().as_secs()
                                );
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
