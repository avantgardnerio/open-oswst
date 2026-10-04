//! Assisted GPS: what we tell the GPS so it finds the sky in seconds, not
//! minutes. The messages are built in the core crate (open_oswst_core::agnss);
//! this decides what to send when, and keeps it on /data. The GPS thread
//! writes them (devices::gps::send).
//!
//! - WiFi joined: once NTP has the time, where the network is (its `lat`,
//!   `lon`, `alt` in config.toml) and the exact time (AID-INI), then the
//!   satellites' orbits, downloaded (Espruino's hourly CASIC file: 32 GPS
//!   ephemerides). Kept on /data, and fetched again every hour while joined
//! - Boot: where we last had a fix (AID-INI, position only, we may have
//!   moved), then the kept orbits if they're under ORBITS_GOOD_FOR old by
//!   the GPS's own clock
//!
//! A repeater never writes /data (a flash write stalls both cores, and its
//! relays can't wait): it sends what it downloads, but keeps nothing.

use std::fs;
use std::time::{Duration, Instant};

use esp_idf_svc::http::client::{Configuration, EspHttpConnection};
use esp_idf_svc::http::Method;
use open_oswst_core::agnss::{self, When, Where};
use open_oswst_core::config;
use open_oswst_core::mode::{self, Mode};

use crate::clock;
use crate::devices::gps;
use crate::devices::settings::WifiNetwork;

/// Espruino's assisted-GPS file for CASIC chips (Bangle.js 2): base64 of a
/// text banner and CASIC frames, regenerated hourly. Plain HTTP: TLS would
/// need ~30-40KB of heap at once
const ORBITS_URL: &str = "http://www.espruino.com/agps/casic.base64";
/// Fetched again this often while joined (Espruino regenerates hourly)
const REFETCH_EVERY: Duration = Duration::from_secs(3600);
/// A download failed: try again this soon
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(5 * 60);
/// Kept orbits are sent at boot if the GPS's clock says they're this new
/// (GPS ephemerides are good for about ±2 hours around their reference time)
const ORBITS_GOOD_FOR_S: i64 = 4 * 3600;
/// How long to wait for NTP after joining before seeding without the time
/// (its first sync came ~27s after joining, 2026-10-04)
const NTP_WAIT: Duration = Duration::from_secs(45);

/// The orbits as last downloaded: the unix time they were fetched (8 bytes
/// LE), then the frames
const ORBITS_FILE: &str = "/data/agnss.bin";
/// Our last fix: "lat lon unix"
const FIX_FILE: &str = "/data/gps-fix";

/// How sure we are of a configured network's location: where the radio is
/// within its range, or less sure without an altitude
const NETWORK_ACCURACY_M: f32 = 100.0;
const NETWORK_NO_ALT_ACCURACY_M: f32 = 2_000.0;
/// The last fix may be far behind us
const LAST_FIX_ACCURACY_M: f32 = 10_000.0;
/// NTP time, 1 sigma
const NTP_ACCURACY_S: f32 = 0.1;

fn repeater() -> bool {
    mode::get() == Mode::Repeater
}

/// Just joined `network`: seed the GPS with where and when, then fetch
/// orbits if due. Runs on the network thread (it blocks: waits for NTP,
/// downloads). `next_check`: see fetch_if_due
pub fn on_joined(network: &WifiNetwork, next_check: &mut Option<Instant>) {
    if !config::AGNSS.is_on() {
        return;
    }
    let started = Instant::now();
    while !clock::from_ntp() && started.elapsed() < NTP_WAIT {
        std::thread::sleep(Duration::from_millis(500));
    }
    let place = network.location.map(|(lat, lon, alt)| Where {
        lat,
        lon,
        alt: alt.unwrap_or(0.0),
        accuracy_m: if alt.is_some() {
            NETWORK_ACCURACY_M
        } else {
            NETWORK_NO_ALT_ACCURACY_M
        },
    });
    let time = clock::from_ntp().then(|| When {
        unix_ms: clock::now_ms(),
        accuracy_s: NTP_ACCURACY_S,
    });
    if place.is_some() || time.is_some() {
        gps::send(agnss::aid_ini(place, time));
        log::info!(
            "AGNSS: told the GPS {} and {}",
            if place.is_some() {
                "where (this network)"
            } else {
                "no position"
            },
            if time.is_some() {
                "the time (NTP)"
            } else {
                "no time"
            },
        );
    }
    // Orbits right after where and when, as Bangle.js 2 does: sent before
    // the time (at boot, 1s after a cold start) the GPS didn't use them
    // (satellites in view crept up as with none, 2026-10-04)
    if time.is_some() {
        send_kept_orbits();
    }
    fetch_if_due(next_check);
}

/// Download the orbits if they're due: only once the clock is set (so the
/// kept file carries a real time), and not while the kept ones are under
/// REFETCH_EVERY old (the GPS thread already sent those at boot). A
/// repeater keeps none, so it downloads every REFETCH_EVERY. Called on
/// joining, and by the network thread every few seconds while joined:
/// `next_check` keeps that from touching flash each time
pub fn fetch_if_due(next_check: &mut Option<Instant>) {
    if !config::AGNSS.is_on() || !clock::is_set() {
        return;
    }
    if next_check.is_some_and(|at| Instant::now() < at) {
        return;
    }
    let refetch_s = REFETCH_EVERY.as_secs() as i64;
    if let Some(age) = kept_age_s().filter(|age| (0..refetch_s).contains(age)) {
        if next_check.is_none() {
            log::info!("AGNSS: kept orbits are {} min old: no download", age / 60);
        }
        *next_check = Some(Instant::now() + Duration::from_secs((refetch_s - age) as u64));
        return;
    }
    let retry_in = if fetch_orbits() {
        REFETCH_EVERY
    } else {
        RETRY_AFTER_FAILURE
    };
    *next_check = Some(Instant::now() + retry_in);
}

/// How old the kept orbits are, by the clock, if there are any
fn kept_age_s() -> Option<i64> {
    use std::io::Read;
    let mut stamp = [0u8; 8];
    fs::File::open(ORBITS_FILE)
        .ok()?
        .read_exact(&mut stamp)
        .ok()?;
    Some(clock::now() - i64::from_le_bytes(stamp))
}

/// Download the orbits, send them to the GPS, and keep them
fn fetch_orbits() -> bool {
    let text = match download(ORBITS_URL) {
        Ok(text) => text,
        Err(e) => {
            log::warn!("AGNSS: download failed: {}", e);
            return false;
        }
    };
    // One copy at a time: the heap is ~19KB at its lowest with a ~9KB
    // largest block, and copies of these ~3KB ran it out (2026-10-04)
    let decoded = agnss::base64_decode(&text);
    drop(text);
    let Some(mut frames) = decoded else {
        log::warn!("AGNSS: the download isn't base64");
        return false;
    };
    let count = agnss::keep_good_frames(&mut frames);
    if count == 0 {
        log::warn!("AGNSS: no good frames in the download");
        return false;
    }
    if !repeater() && clock::is_set() {
        if let Err(e) = keep_orbits(&frames) {
            log::warn!("AGNSS: keeping the orbits failed: {}", e);
        }
    }
    log::info!(
        "AGNSS: sending the GPS {} orbit frames ({} bytes)",
        count,
        frames.len()
    );
    gps::send(frames);
    true
}

/// The fetch time, then the frames: two writes, no joined copy
fn keep_orbits(frames: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = fs::File::create(ORBITS_FILE)?;
    file.write_all(&clock::now().to_le_bytes())?;
    file.write_all(frames)
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    let config = Configuration {
        timeout: Some(Duration::from_secs(10)),
        ..Default::default()
    };
    let mut http = EspHttpConnection::new(&config).map_err(|e| e.to_string())?;
    http.initiate_request(Method::Get, url, &[])
        .map_err(|e| e.to_string())?;
    http.initiate_response().map_err(|e| e.to_string())?;
    if http.status() != 200 {
        return Err(format!("HTTP {}", http.status()));
    }
    let mut body = Vec::with_capacity(4096);
    let mut buf = [0u8; 512];
    loop {
        let n = http.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buf[..n]);
        if body.len() > 16 * 1024 {
            return Err("longer than 16KB".into());
        }
    }
    Ok(body)
}

/// Boot, the GPS just powered: tell it where we last had a fix
pub fn seed_at_boot() {
    let Ok(text) = fs::read_to_string(FIX_FILE) else {
        return;
    };
    let mut parts = text.split_whitespace().map(str::parse::<f64>);
    let (Some(Ok(lat)), Some(Ok(lon))) = (parts.next(), parts.next()) else {
        return;
    };
    let place = Where {
        lat,
        lon,
        alt: 0.0,
        accuracy_m: LAST_FIX_ACCURACY_M,
    };
    gps::send(agnss::aid_ini(Some(place), None));
    log::info!("AGNSS: told the GPS our last fix, {:.4},{:.4}", lat, lon);
}

/// The clock is set (from the GPS's own time, or NTP): send the kept
/// orbits if they're still good. Once per boot
pub fn send_kept_orbits() {
    let Ok(mut kept) = fs::read(ORBITS_FILE) else {
        return;
    };
    if kept.len() < 8 {
        return;
    }
    let fetched = i64::from_le_bytes(kept[..8].try_into().unwrap());
    let age = clock::now() - fetched;
    if !(0..ORBITS_GOOD_FOR_S).contains(&age) {
        log::info!("AGNSS: kept orbits are {} min old: not sent", age / 60);
        return;
    }
    kept.drain(..8); // in place: no second copy
    let mut frames = kept;
    let count = agnss::keep_good_frames(&mut frames);
    log::info!(
        "AGNSS: sending the GPS {} kept orbit frames, {} min old",
        count,
        age / 60
    );
    gps::send(frames);
}

/// We have a fix: keep it for the next boot (not on a repeater)
pub fn keep_fix(lat: f64, lon: f64) {
    if repeater() {
        return;
    }
    let text = format!("{:.6} {:.6} {}\n", lat, lon, clock::now());
    if let Err(e) = fs::write(FIX_FILE, text) {
        log::warn!("AGNSS: keeping the fix failed: {}", e);
    }
}
