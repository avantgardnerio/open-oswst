//! WiFi, so a radio can be updated, configured and read without USB (the
//! HTTP API is in http.rs). Joins the first network from config.toml that's
//! in range, and keeps rejoining. Answers as `oswst-XXXX.local` (the MAC's
//! last 4 hex digits) and announces `_oswst._tcp`, so scripts can find every
//! radio on the network.
//!
//! With no networks in config.toml, WiFi never starts: no heap spent, and
//! nothing transmitted. ⚠️ WiFi beacons and probes are easy to
//! direction-find. It's on whenever networks are configured, by choice,
//! until the menus come back to switch it (see the wifi-ota plan).

use std::thread::sleep;
use std::time::Duration;

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::cpu::Core;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::mdns::EspMdns;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi};

use crate::devices::settings::WifiNetwork;
use crate::{http, thread};

/// How often to check the connection, and rejoin if it's gone
const CHECK_EVERY: Duration = Duration::from_secs(10);
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

fn run(modem: Modem<'static>, networks: Vec<WifiNetwork>, mac: String) {
    let name = hostname(&mac);
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
    wifi.start().unwrap();

    // Kept alive for as long as this thread runs
    let _mdns = announce(&name, &mac);
    let _http = http::start(&name, &mac);

    loop {
        if !wifi.is_connected().unwrap_or(false) {
            join(&mut wifi, &networks);
        }
        sleep(CHECK_EVERY);
    }
}

/// Join the first configured network that's in range.
fn join(wifi: &mut BlockingWifi<EspWifi<'static>>, networks: &[WifiNetwork]) {
    let in_range = match wifi.scan() {
        Ok(found) => found,
        Err(e) => {
            log::warn!("WiFi: scan failed: {}", e);
            return;
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
                return;
            }
            Err(e) => {
                log::warn!("WiFi: joining {:?} failed: {}", network.ssid, e);
                let _ = wifi.disconnect();
            }
        }
    }
    log::info!(
        "WiFi: none of the {} configured networks joined",
        networks.len()
    );
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
