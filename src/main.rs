mod app;

use embassy_futures::join::join3;
use esp_idf_svc::hal::task::block_on;
use esp_idf_svc::nvs::{EspCustomNvsPartition, EspNvs};
use open_oswst::board;
use open_oswst::codec;
use open_oswst::devices::{mic, radio, screen, speaker};
use std::sync::atomic::AtomicBool;

/// Whether this device is a repeater, read from NVS at boot.
pub(crate) static IS_REPEATER: AtomicBool = AtomicBool::new(false);

/// Read the base MAC address from eFuse
fn get_mac() -> [u8; 6] {
    let mut mac = [0u8; 6];
    unsafe {
        esp_idf_svc::sys::esp_read_mac(
            mac.as_mut_ptr(),
            esp_idf_svc::sys::esp_mac_type_t_ESP_MAC_WIFI_STA,
        );
    }
    mac
}

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("open-oswst starting...");

    let board = board::take();

    // Read config from dedicated NVS partition
    let nvs_partition = EspCustomNvsPartition::take("open-oswst").unwrap();
    // Unprovisioned boards have no config namespace — fall back to defaults
    let repeater = match EspNvs::new(nvs_partition, "config", false) {
        Ok(nvs) => nvs.get_u8("repeater").unwrap().unwrap_or(0) != 0,
        Err(e) => {
            log::warn!("No NVS config ({}), using defaults", e);
            false
        }
    };
    IS_REPEATER.store(repeater, std::sync::atomic::Ordering::Relaxed);
    log::info!("Config: repeater={}", repeater);

    // Get MAC for display
    let mac = get_mac();
    let mut mac_str = heapless::String::<18>::new();
    let _ = core::fmt::write(
        &mut mac_str,
        format_args!(
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        ),
    );

    // Spawn codec thread with sync channel (capacity 2 to allow pipelining)
    let (codec_tx, codec_rx) = std::sync::mpsc::sync_channel::<codec::CodecRequest>(2);
    std::thread::Builder::new()
        .name("codec".into())
        .stack_size(32768)
        .spawn(move || codec::run(codec_rx))
        .unwrap();

    block_on(async {
        let radio_fut = radio::init(board.radio).await;
        let speaker_fut = speaker::init(board.speaker).await;
        let screen = screen::init(board.screen);
        let mic = mic::init(board.mic);

        let app_fut = app::init(
            app::Peripherals { ptt: board.ptt },
            mic,
            screen,
            mac_str,
            codec_tx,
        )
        .await;

        log::info!("All systems ready");
        join3(radio_fut, app_fut, speaker_fut).await;
    });
}
