//! The system clock: what time() and the file dates on /data read. It starts
//! at 1970 on every boot. Two ways to set it, whichever comes first:
//! - the GPS, as soon as it reports the UTC date and time (it does long
//!   before it has a position). Good to ~0.5 s: NMEA only, no PPS pin
//! - NTP, once WiFi reaches the internet: milliseconds, and kept right with
//!   a check every hour. The GPS never overrides it
//!
//! Logs a line when it's set, tying the log's uptime to real time.

use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use esp_idf_svc::sntp::{EspSntp, SntpConf};
use open_oswst_core::utc;

/// What set the clock: nothing yet, the GPS or NTP
static SOURCE: AtomicU8 = AtomicU8::new(NOT_SET);
const NOT_SET: u8 = 0;
const GPS: u8 = 1;
const NTP: u8 = 2;

/// The earliest date a GPS report is believed: some receivers report a
/// default (1980, 2000...) before they've heard the satellites' time
const PLAUSIBLE_YEAR: u16 = 2024;

pub fn is_set() -> bool {
    SOURCE.load(Ordering::Relaxed) != NOT_SET
}

/// Set by NTP: good to milliseconds, not just the GPS's ~0.5 s
pub fn from_ntp() -> bool {
    SOURCE.load(Ordering::Relaxed) == NTP
}

/// The GPS has a UTC date and time: set the clock from it, unless it's set
/// already (from the GPS, or better, from NTP)
pub fn set_from_gps(date: (u16, u8, u8), time: (u8, u8, u8)) {
    if is_set() || date.0 < PLAUSIBLE_YEAR {
        return;
    }
    let seconds = utc::unix_seconds(date, time);
    let now = esp_idf_svc::sys::timeval {
        tv_sec: seconds,
        tv_usec: 0,
    };
    unsafe { esp_idf_svc::sys::settimeofday(&now, core::ptr::null()) };
    SOURCE.store(GPS, Ordering::Relaxed);
    log::info!("Clock: set from GPS, {}", utc::iso(seconds));
}

/// Start NTP (pool.ntp.org); keep what this returns for as long as it
/// should run. Its first sync takes over the clock, even from the GPS
pub fn start_ntp() -> Option<EspSntp<'static>> {
    let started = EspSntp::new_with_callback(&SntpConf::default(), |_| {
        if SOURCE.swap(NTP, Ordering::Relaxed) != NTP {
            log::info!("Clock: set from NTP, {}", utc::iso(now()));
        }
    });
    started
        .map_err(|e| log::warn!("Clock: NTP didn't start: {}", e))
        .ok()
}

/// Seconds since 1970 by the system clock
pub fn now() -> i64 {
    now_ms() / 1000
}

/// Milliseconds since 1970 by the system clock
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as i64)
}
