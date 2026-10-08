//! WiFi, so a radio can be updated, configured and read without USB (the
//! HTTP API is in http.rs). Joins the first network from config.toml that's
//! in range, and keeps rejoining. Answers as `oswst-XXXX.local` (the MAC's
//! last 4 hex digits) and announces `_oswst._tcp`, so scripts can find every
//! radio on the network.
//!
//! With no networks in config.toml, WiFi never starts: no heap spent, and
//! nothing transmitted. WARNING: WiFi beacons and probes are easy to
//! direction-find.
//!
//! The setting config::WIFI_ON says whether it runs, and it's saved: switched
//! off or on in the menu (Privacy), WiFi stops or starts within ON_CHECK and
//! stays that way across reboots. `POST /wifi/off` stops it until the next
//! reboot (or the menu) only, for field tests: WiFi on core 0 costs the
//! audio its timing.

use std::sync::mpsc;
use std::thread::sleep;
use std::time::Duration;

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::cpu::Core;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::mdns::EspMdns;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi};

use std::sync::Mutex;

use open_oswst_core::config;
use open_oswst_core::devices::network::Network;

use crate::devices::settings::WifiNetwork;
use crate::{clock, http, thread};

/// Where the WiFi is at, for the screen (Platform::network). Off until
/// `start` finds networks configured.
static STATE: Mutex<Network> = Mutex::new(Network::Off);

pub fn state() -> Network {
    STATE.lock().unwrap().clone()
}

fn set_state(network: Network) {
    *STATE.lock().unwrap() = network;
}

/// How often to check the connection, and rejoin if it's gone
const CHECK_EVERY: Duration = Duration::from_secs(10);
/// How soon WiFi stops or starts once config::WIFI_ON changes (the menu)
const ON_CHECK: Duration = Duration::from_secs(1);
/// Below the codec (5) and far below the radio (10, on the other core)
const PRIORITY: u8 = 3;

/// Start WiFi and the HTTP server on their own thread, if any networks are
/// configured. `mac` is the board's, as printed (AA:BB:CC:DD:EE:FF).
pub fn start(modem: Modem<'static>, networks: Vec<WifiNetwork>, mac: String) {
    if networks.is_empty() {
        log::info!("WiFi: no networks in config.toml, staying off");
        return;
    }
    thread::spawn(c"net", 8192, Some(PRIORITY), Some(Core::Core0), move || {
        run(modem, networks, mac)
    });
}

/// WiFi on and off as config::WIFI_ON says, for as long as the radio runs.
/// While it's off, nothing is transmitted; until it's first on, nothing of
/// the WiFi driver is even set up (no heap spent)
fn run(modem: Modem<'static>, networks: Vec<WifiNetwork>, mac: String) {
    let name = hostname(&mac);
    if !config::WIFI_ON.is_on() {
        log::info!("WiFi: off (wifi_on = false) until switched on");
        wait_for_on();
    }
    let sys_loop = EspSystemEventLoop::take().unwrap();
    // ESP-IDF's own NVS partition, for the PHY's calibration data. Not given
    // to the WiFi driver: then it never writes its config to flash by itself
    // (a flash write stalls both cores, the radio's too)
    let _nvs = EspDefaultNvsPartition::take()
        .map_err(|e| log::warn!("NVS unavailable, PHY recalibrates each boot: {}", e));
    let driver = EspWifi::new(modem, sys_loop.clone(), None).unwrap();
    let mut wifi = BlockingWifi::wrap(driver, sys_loop).unwrap();
    wifi.set_configuration(&Configuration::Client(ClientConfiguration::default()))
        .unwrap();

    // /wifi/off sends on this. We keep a sender too, so the channel never
    // closes even if the HTTP server failed to start
    let (off_tx, off_rx) = mpsc::channel();
    loop {
        set_state(Network::Searching);
        wifi.start().unwrap();
        let mdns = announce(&name, &mac);
        let http = http::start(&name, &mac, off_tx.clone());
        // Sets the clock once a network with internet is joined (it retries)
        let ntp = clock::start_ntp();

        stay_joined(&mut wifi, &networks, &off_rx);

        // Give /wifi/off's reply time to get out, then stop everything
        sleep(Duration::from_millis(500));
        drop(ntp);
        drop(http);
        drop(mdns);
        if let Err(e) = wifi.stop() {
            log::warn!("WiFi: stop failed: {}", e);
        }
        set_state(Network::Off);
        if config::WIFI_ON.is_on() {
            // /wifi/off: the menu's choice (the file's) comes back at the reboot
            config::WIFI_ON.set_until_reboot(0);
            log::info!("WiFi: off until the next reboot or the menu (/wifi/off)");
        } else {
            log::info!("WiFi: off (wifi_on = false)");
        }
        while off_rx.try_recv().is_ok() {} // a /wifi/off that came twice

        wait_for_on();
        log::info!("WiFi: on again");
    }
}

/// Join a network and rejoin when it's lost, until WiFi is to stop
fn stay_joined(
    wifi: &mut BlockingWifi<EspWifi<'static>>,
    networks: &[WifiNetwork],
    off_rx: &mpsc::Receiver<()>,
) {
    // Logged only when it changes: away from the networks, every scan would
    // say the same "none joined" (~every 20s on a walk), and each line in
    // the log costs a flash write that stalls both cores
    let mut joined = false;
    let mut searching_logged = false;
    loop {
        if !wifi.is_connected().unwrap_or(false) {
            if joined {
                log::info!("WiFi: lost the network");
            }
            set_state(Network::Searching);
            joined = join(wifi, networks);
            if joined {
                searching_logged = false;
            } else if !searching_logged {
                log::info!(
                    "WiFi: none of the {} configured networks joined; still looking",
                    networks.len()
                );
                searching_logged = true;
            }
        }
        if wait_for_off(off_rx) {
            return;
        }
    }
}

/// Wait until config::WIFI_ON is on (the menu), checking every ON_CHECK
fn wait_for_on() {
    while !config::WIFI_ON.is_on() {
        sleep(ON_CHECK);
    }
}

/// Wait out CHECK_EVERY, or less if WiFi is to stop: /wifi/off (a message on
/// `off_rx`) or config::WIFI_ON switched off (checked every ON_CHECK)
fn wait_for_off(off_rx: &mpsc::Receiver<()>) -> bool {
    let started = std::time::Instant::now();
    while started.elapsed() < CHECK_EVERY {
        if off_rx.recv_timeout(ON_CHECK).is_ok() || !config::WIFI_ON.is_on() {
            return true;
        }
    }
    false
}

/// Join the first configured network that's in range. False if none
fn join(wifi: &mut BlockingWifi<EspWifi<'static>>, networks: &[WifiNetwork]) -> bool {
    let in_range = match wifi.scan() {
        Ok(found) => found,
        Err(e) => {
            log::warn!("WiFi: scan failed: {}", e);
            return false;
        }
    };
    for network in networks {
        let Some(ap) = in_range.iter().find(|ap| ap.ssid.as_str() == network.ssid) else {
            continue;
        };
        let (Ok(ssid), Ok(password)) = (
            network.ssid.as_str().try_into(),
            network.password.as_str().try_into(),
        ) else {
            log::warn!("WiFi: {:?}: ssid or password too long", network.ssid);
            continue;
        };
        let config = ClientConfiguration {
            ssid,
            password,
            auth_method: ap.auth_method.unwrap_or(AuthMethod::WPA2Personal),
            channel: Some(ap.channel),
            ..Default::default()
        };
        if let Err(e) = wifi.set_configuration(&Configuration::Client(config)) {
            log::warn!("WiFi: configuring {:?} failed: {}", network.ssid, e);
            continue;
        }
        match wifi.connect().and_then(|()| wifi.wait_netif_up()) {
            Ok(()) => {
                let ip = wifi.wifi().sta_netif().get_ip_info().map(|info| info.ip);
                log::info!(
                    "WiFi: joined {:?} (RSSI {}), IP {:?}",
                    network.ssid,
                    ap.signal_strength,
                    ip
                );
                set_state(Network::Joined {
                    ssid: network.ssid.as_str().try_into().unwrap_or_default(),
                    ip: ip.map(|ip| ip.octets()).unwrap_or_default(),
                });
                return true;
            }
            Err(e) => {
                log::warn!("WiFi: joining {:?} failed: {}", network.ssid, e);
                let _ = wifi.disconnect();
            }
        }
    }
    false
}

/// mDNS: answer as NAME.local, and announce the HTTP API as `_oswst._tcp`.
fn announce(name: &str, mac: &str) -> Option<EspMdns> {
    let mut mdns = EspMdns::take()
        .map_err(|e| log::warn!("mDNS unavailable: {}", e))
        .ok()?;
    let result = mdns
        .set_hostname(name)
        .and_then(|()| mdns.set_instance_name(name))
        .and_then(|()| mdns.add_service(None, "_oswst", "_tcp", 80, &[("mac", mac)]));
    if let Err(e) = result {
        log::warn!("mDNS setup failed: {}", e);
    }
    Some(mdns)
}

/// oswst-c62c for MAC F8:5B:1B:A2:C6:2C
fn hostname(mac: &str) -> String {
    let hex: String = mac.chars().filter(|c| *c != ':').collect();
    format!("oswst-{}", hex[hex.len() - 4..].to_lowercase())
}
