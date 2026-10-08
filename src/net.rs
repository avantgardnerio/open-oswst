//! WiFi, so a radio can be updated, configured and read without USB (the
//! HTTP API is in http.rs). Joins the first network from config.toml that's
//! in range, and keeps rejoining. Answers as `oswst-XXXX.local` (the MAC's
//! last 4 hex digits) and announces `_oswst._tcp`, so scripts can find every
//! radio on the network.
//!
//! With no networks in config.toml, WiFi doesn't start: no heap spent, and
//! nothing transmitted. Until the menu's Add list scans for one: while that
//! list is on the screen (core's devices::wifi::SCAN_WANTED), it scans back
//! to back and the menu gets every scan. WARNING: WiFi beacons and probes
//! are easy to direction-find.
//!
//! The setting config::WIFI_ON says whether it runs, and it's saved: switched
//! off or on in the menu (Privacy), WiFi stops or starts within ON_CHECK and
//! stays that way across reboots. `POST /wifi/off` stops it until the next
//! reboot (or the menu) only, for field tests: WiFi on core 0 costs the
//! audio its timing.
//!
//! The HTTP API runs while WiFi does, unless config::HTTP_API_ON is off
//! (the menu, Privacy): it stops or starts within ON_CHECK.

use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread::sleep;
use std::time::{Duration, Instant};

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::cpu::Core;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::http::server::EspHttpServer;
use esp_idf_svc::mdns::EspMdns;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{
    AccessPointInfo, AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi,
};

use std::sync::Mutex;

use open_oswst_core::config;
use open_oswst_core::devices::management::{ANSWERS, REQUESTS};
use open_oswst_core::devices::network::Network;
use open_oswst_core::devices::wifi::{self, Seen};

use crate::devices::secrets;
use crate::devices::settings::{self, WifiNetwork};
use crate::{clock, http, management, thread};

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
/// How soon WiFi stops or starts once config::WIFI_ON changes (the menu),
/// and how soon the menu's scans start
const ON_CHECK: Duration = Duration::from_millis(250);
/// Below the codec (5) and far below the radio (10, on the other core)
const PRIORITY: u8 = 3;

/// Start WiFi and the HTTP server on their own thread. `mac` is the
/// board's, as printed (AA:BB:CC:DD:EE:FF).
pub fn start(modem: Modem<'static>, mac: String) {
    thread::spawn(c"net", 8192, Some(PRIORITY), Some(Core::Core0), move || {
        run(modem, mac)
    });
}

/// Why WiFi stopped
enum Stop {
    SwitchedOff, // config::WIFI_ON off (the menu)
    OffMessage,  // /wifi/off
    Unneeded,    // no networks saved, and the menu isn't scanning
}

/// WiFi is wanted: switched on, with a network to join or the menu scanning
fn wanted() -> bool {
    config::WIFI_ON.is_on()
        && (settings::has_wifi_networks() || wifi::SCAN_WANTED.load(Ordering::Relaxed))
}

/// WiFi on and off as wanted, for as long as the radio runs. While it's
/// off, nothing is transmitted; until it's first on, nothing of the WiFi
/// driver is even set up (no heap spent)
fn run(modem: Modem<'static>, mac: String) {
    let name = hostname(&mac);
    if !config::WIFI_ON.is_on() {
        log::info!("WiFi: off (wifi_on = false) until switched on");
    } else if !settings::has_wifi_networks() {
        log::info!("WiFi: no networks saved, off until one is (or the menu scans)");
    }
    wait_for_wanted();
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
        // Now the RNG is truly random: our key for the management server
        secrets::make_key();
        let mdns = announce(&name, &mac);
        let mut http = HttpApi::new(&name, &mac, off_tx.clone());
        // Sets the clock once a network with internet is joined (it retries)
        let ntp = clock::start_ntp();

        let stop = stay_joined(&mut wifi, &off_rx, &mut http);

        // Give /wifi/off's reply time to get out, then stop everything
        sleep(Duration::from_millis(500));
        drop(ntp);
        drop(http);
        drop(mdns);
        if let Err(e) = wifi.stop() {
            log::warn!("WiFi: stop failed: {}", e);
        }
        set_state(Network::Off);
        match stop {
            Stop::OffMessage => {
                // The menu's choice (the file's) comes back at the reboot
                config::WIFI_ON.set_until_reboot(0);
                log::info!("WiFi: off until the next reboot or the menu (/wifi/off)");
            }
            Stop::SwitchedOff => log::info!("WiFi: off (wifi_on = false)"),
            Stop::Unneeded => log::info!("WiFi: off (no networks saved)"),
        }
        while off_rx.try_recv().is_ok() {} // a /wifi/off that came twice

        wait_for_wanted();
        log::info!("WiFi: on again");
    }
}

/// The HTTP API (http.rs), running or not as config::HTTP_API_ON says
/// (the menu, Privacy)
struct HttpApi {
    name: String,
    mac: String,
    wifi_off: mpsc::Sender<()>,
    on: bool, // the setting as last followed: running, unless it failed to start
    server: Option<EspHttpServer<'static>>,
}

impl HttpApi {
    fn new(name: &str, mac: &str, wifi_off: mpsc::Sender<()>) -> HttpApi {
        let mut http = HttpApi {
            name: name.into(),
            mac: mac.into(),
            wifi_off,
            on: false,
            server: None,
        };
        http.follow_setting();
        http
    }

    /// Start or stop the server if the setting changed
    fn follow_setting(&mut self) {
        let on = config::HTTP_API_ON.is_on();
        if on == self.on {
            return;
        }
        self.on = on;
        if on {
            self.server = http::start(&self.name, &self.mac, self.wifi_off.clone());
        } else {
            self.server = None;
            log::info!("HTTP API: off (http_api_on = false)");
        }
    }
}

/// Join a network and rejoin when it's lost, scan for the menu while it
/// wants, send the menu's requests to the management server, and keep the
/// HTTP API as its setting says, until WiFi is to stop
fn stay_joined(
    wifi: &mut BlockingWifi<EspWifi<'static>>,
    off_rx: &mpsc::Receiver<()>,
    http: &mut HttpApi,
) -> Stop {
    // Logged only when it changes: away from the networks, every scan would
    // say the same "none joined" (~every 20s on a walk), and each line in
    // the log costs a flash write that stalls both cores
    let mut joined: Option<String> = None;
    let mut searching_logged = false;
    loop {
        http.follow_setting();
        // Read each time round: the menu adds and forgets them
        let networks = settings::wifi_networks();
        let scan_wanted = wifi::SCAN_WANTED.load(Ordering::Relaxed);
        if networks.is_empty() && !scan_wanted {
            return Stop::Unneeded;
        }

        let mut connected = wifi.is_connected().unwrap_or(false);
        if let Some(ssid) = &joined {
            if !connected {
                log::info!("WiFi: lost the network");
                joined = None;
            } else if !networks.iter().any(|network| network.ssid == *ssid) {
                log::info!("WiFi: leaving {:?}, forgotten", ssid);
                let _ = wifi.disconnect();
                set_state(Network::Searching);
                joined = None;
                connected = false;
            }
        }
        let to_join = !connected && !networks.is_empty();
        if to_join {
            set_state(Network::Searching);
        }

        if scan_wanted || to_join {
            match wifi.scan() {
                Ok(in_range) => {
                    if scan_wanted {
                        wifi::SCANS.signal(in_range.iter().map(seen).collect());
                    }
                    if to_join {
                        joined = join(wifi, &networks, &in_range);
                        if joined.is_some() {
                            searching_logged = false;
                        } else if !searching_logged {
                            log::info!(
                                "WiFi: none of the {} saved networks joined; still looking",
                                networks.len()
                            );
                            searching_logged = true;
                        }
                    }
                }
                Err(e) => log::warn!("WiFi: scan failed: {}", e),
            }
        }
        if let Some(stop) = wait(off_rx, http.on) {
            return stop;
        }
    }
}

/// For the menu: what a scan saw
fn seen(ap: &AccessPointInfo) -> Seen {
    Seen {
        ssid: ap.ssid.as_str().try_into().unwrap_or_default(),
        rssi_dbm: ap.signal_strength,
        open: ap.auth_method == Some(AuthMethod::None),
    }
}

/// Wait until WiFi is wanted, checking every ON_CHECK
fn wait_for_wanted() {
    while !wanted() {
        sleep(ON_CHECK);
    }
}

/// Wait out CHECK_EVERY, or less: not at all while the menu wants scans
/// (they go back to back), or once the HTTP API's setting differs from
/// `http_on`. Some if WiFi is to stop: /wifi/off (a message on `off_rx`) or
/// config::WIFI_ON switched off (checked every ON_CHECK). The menu's
/// requests to the management server are sent from here, as they come
fn wait(off_rx: &mpsc::Receiver<()>, http_on: bool) -> Option<Stop> {
    let started = Instant::now();
    loop {
        if off_rx.try_recv().is_ok() {
            return Some(Stop::OffMessage);
        }
        if !config::WIFI_ON.is_on() {
            return Some(Stop::SwitchedOff);
        }
        if let Ok(request) = REQUESTS.try_receive() {
            ANSWERS.signal(management::call(&request));
        }
        if wifi::SCAN_WANTED.load(Ordering::Relaxed)
            || config::HTTP_API_ON.is_on() != http_on
            || started.elapsed() >= CHECK_EVERY
        {
            return None;
        }
        if off_rx.recv_timeout(ON_CHECK).is_ok() {
            return Some(Stop::OffMessage);
        }
    }
}

/// Join the first saved network that's in range: its name, or None if none
fn join(
    wifi: &mut BlockingWifi<EspWifi<'static>>,
    networks: &[WifiNetwork],
    in_range: &[AccessPointInfo],
) -> Option<String> {
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
                return Some(network.ssid.clone());
            }
            Err(e) => {
                log::warn!("WiFi: joining {:?} failed: {}", network.ssid, e);
                let _ = wifi.disconnect();
            }
        }
    }
    None
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
