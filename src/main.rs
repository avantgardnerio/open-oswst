mod app;
mod menu;

use embassy_futures::join::join3;
use esp_idf_svc::hal::task::block_on;
use esp_idf_svc::nvs::{EspCustomNvsPartition, EspNvs};
use open_oswst::board;
use open_oswst::codec;
use open_oswst::devices::{encoder, mic, radio, screen, speaker};
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

    // Config lives in the dedicated NVS partition. Opened read-write so the
    // menu can save settings; that also creates the namespace on fresh boards.
    let nvs_partition = EspCustomNvsPartition::take("open-oswst").unwrap();
    let nvs = EspNvs::new(nvs_partition, "config", true)
        .map_err(|e| log::warn!("NVS config unavailable ({}), settings won't persist", e))
        .ok();
    let repeater = nvs
        .as_ref()
        .and_then(|nvs| nvs.get_u8("repeater").ok().flatten())
        .unwrap_or(0)
        != 0;
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
        let encoder = encoder::init(board.vol);

        let app_fut = app::init(
            app::Peripherals { ptt: board.ptt },
            mic,
            encoder,
            screen,
            mac_str,
            nvs,
            codec_tx,
        )
        .await;

        log::info!("All systems ready");
        join3(radio_fut, app_fut, speaker_fut).await;
    });
}
