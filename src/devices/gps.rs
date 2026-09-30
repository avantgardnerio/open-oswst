//! GPS: the L76K on the Heltec's GNSS connector. A thread reads its NMEA and
//! keeps the latest time and position; callers just ask for `latest()`.
//!
//! Only two sentences matter: RMC (UTC time, date, position, valid flag) and
//! GGA (satellites in use). The module usually knows the time well before it
//! has a position, so the two are reported separately.

use esp_idf_svc::hal::delay::TickType;
use esp_idf_svc::hal::gpio::{AnyIOPin, AnyOutputPin, PinDriver};
use esp_idf_svc::hal::uart::{config::Config, UartDriver, UART1};
use esp_idf_svc::hal::units::Hertz;
use std::sync::{Arc, Mutex};
use std::thread;
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

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Fix {
    pub time: Option<(u8, u8, u8)>,   // UTC hh, mm, ss
    pub date: Option<(u16, u8, u8)>,  // yyyy, mm, dd
    pub position: Option<(f64, f64)>, // lat, lon in degrees; None without a fix
    pub satellites: u8,
}

/// `2026-09-30T20:49:00Z 40.54235,-105.08326 sats=7`, with `no-fix` for the
/// position and `no-time` / `no-date` for anything the GPS doesn't know yet.
impl std::fmt::Display for Fix {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self.date {
            Some((y, mo, d)) => write!(f, "{:04}-{:02}-{:02}T", y, mo, d)?,
            None => write!(f, "no-date ")?,
        }
        match self.time {
            Some((h, m, s)) => write!(f, "{:02}:{:02}:{:02}Z", h, m, s)?,
            None => write!(f, "no-time")?,
        }
        match self.position {
            Some((lat, lon)) => write!(f, " {:.5},{:.5}", lat, lon)?,
            None => write!(f, " no-fix")?,
        }
        write!(f, " sats={}", self.satellites)
    }
}

pub struct Gps {
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    fix: Fix,
    updated: Option<Instant>,
}

impl Gps {
    /// Latest fix, or None if the GPS has gone quiet (unplugged, or never started).
    pub fn latest(&self) -> Option<Fix> {
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
    thread::Builder::new()
        .name("gps".into())
        .stack_size(6144)
        .spawn(move || run(p, shared))
        .unwrap();
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
                        let mut state = state.lock().unwrap();
                        if apply(&mut state.fix, sentence.trim_end()) {
                            state.updated = Some(Instant::now());
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

/// Fold one NMEA sentence into `fix`. Returns whether it was one we use.
fn apply(fix: &mut Fix, sentence: &str) -> bool {
    let Some(body) = checked_body(sentence) else {
        return false;
    };
    let fields: Vec<&str> = body.split(',').collect();
    match fields[0].get(2..) {
        // $GNRMC,hhmmss.sss,A|V,lat,N|S,lon,E|W,speed,course,ddmmyy,...
        Some("RMC") if fields.len() > 9 => {
            fix.time = parse_time(fields[1]);
            fix.date = parse_date(fields[9]);
            fix.position = if fields[2] == "A" {
                parse_coord(fields[3], fields[4]).zip(parse_coord(fields[5], fields[6]))
            } else {
                None
            };
            true
        }
        // $GNGGA,time,lat,N,lon,E,quality,satellites,...
        Some("GGA") if fields.len() > 7 => {
            fix.satellites = fields[7].parse().unwrap_or(0);
            true
        }
        _ => false,
    }
}

/// The part between `$` and `*`, if the checksum after `*` matches.
fn checked_body(sentence: &str) -> Option<&str> {
    let (body, checksum) = sentence.strip_prefix('$')?.split_once('*')?;
    let expected = u8::from_str_radix(checksum, 16).ok()?;
    let actual = body.bytes().fold(0u8, |sum, b| sum ^ b);
    (actual == expected).then_some(body)
}

/// "204900.000" -> (20, 49, 0)
fn parse_time(field: &str) -> Option<(u8, u8, u8)> {
    let digits = field.get(..6)?;
    Some((
        digits[0..2].parse().ok()?,
        digits[2..4].parse().ok()?,
        digits[4..6].parse().ok()?,
    ))
}

/// "300926" -> (2026, 9, 30)
fn parse_date(field: &str) -> Option<(u16, u8, u8)> {
    if field.len() != 6 {
        return None;
    }
    Some((
        2000 + field[4..6].parse::<u16>().ok()?,
        field[2..4].parse().ok()?,
        field[0..2].parse().ok()?,
    ))
}

/// NMEA (d)ddmm.mmmm plus hemisphere -> signed degrees. The degrees are
/// everything before the last two digits ahead of the '.'.
fn parse_coord(value: &str, hemisphere: &str) -> Option<f64> {
    let dot = value.find('.')?;
    let degrees: f64 = value.get(..dot.checked_sub(2)?)?.parse().ok()?;
    let minutes: f64 = value.get(dot - 2..)?.parse().ok()?;
    let magnitude = degrees + minutes / 60.0;
    match hemisphere {
        "N" | "E" => Some(magnitude),
        "S" | "W" => Some(-magnitude),
        _ => None,
    }
}
