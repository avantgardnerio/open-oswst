//! Firmware entry point: brings up the board's hardware and runs the app from
//! the core crate on it. Everything radio-behaviour lives in core.

use embassy_futures::join::join;
use esp_idf_svc::hal::cpu::Core;
use esp_idf_svc::hal::task::block_on;
use esp_idf_svc::hal::task::thread::ThreadSpawnConfiguration;
use open_oswst::board;
use open_oswst::devices::{encoder, fem, gps, mic, ptt, radio, screen, settings, speaker, storage};
use open_oswst_core::platform::Platform;
use open_oswst_core::{app, codec, logger, mode};
use std::path::Path;

/// The radio thread's priority: above the codec's (5), so the codec can't
/// delay a TX or an IRQ, and below the hal's IsrReactor (11), which wakes it.
const RADIO_PRIORITY: u8 = 10;

/// The board, as the app sees it
struct Esp;

impl Platform for Esp {
    type Mic = mic::Mic;
    type Ptt = ptt::Ptt;
    type Knob = encoder::Encoder;
    type Gps = gps::Gps;
    type Settings = settings::Settings;

    fn random() -> u32 {
        unsafe { esp_idf_svc::sys::esp_random() }
    }
}

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
    // Same clock as ESP-IDF's own log lines
    logger::init(|| unsafe { esp_idf_svc::sys::esp_log_timestamp() });
    log::info!("open-oswst starting...");

    // Logs also go to a file per boot. Without storage we still log to serial
    let log_dir = Path::new(storage::ROOT).join("log");
    match storage::init().map(|()| logger::open_file(&log_dir, storage::usage)) {
        Ok(Ok(path)) => log::info!("Logging to {}", path.display()),
        Ok(Err(e)) => log::warn!("No log file ({}), serial only", e),
        Err(e) => log::warn!("Storage unavailable ({}), serial only", e),
    }

    let board = board::take();
    let _fem = fem::init(board.fem);
    let gps = gps::init(board.gps);

    let settings = settings::init();
    let mode = mode::load(settings.as_ref());
    log::info!("Config: mode={:?}", mode);

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

    spawn_radio(board.radio);

    block_on(async {
        let speaker_fut = speaker::init(board.speaker).await;

        let app_fut = app::init::<Esp>(
            app::Devices {
                mic: mic::init(board.mic),
                ptt: ptt::init(board.ptt),
                knob: encoder::init(board.vol),
                screen: screen::init(board.screen),
                gps,
                settings,
            },
            mac_str,
            codec_tx,
        )
        .await;

        log::info!("All systems ready");
        join(app_fut, speaker_fut).await;
    });
}

/// The radio gets its own thread on core 1, so nothing else running can
/// delay it (on core 0 with the codec, a TX step took up to 200ms instead of
/// 75). It talks to the app only through RX_CHAN and TX_CHAN.
fn spawn_radio(pins: radio::Peripherals) {
    ThreadSpawnConfiguration {
        name: Some(c"radio"),
        priority: RADIO_PRIORITY,
        pin_to_core: Some(Core::Core1),
        ..Default::default()
    }
    .set()
    .unwrap();
    // The radio is created on this thread, not moved to it: created on core 0
    // and moved, it hung (src/bin/radio_timing.rs). std's stack size wins over
    // the spawn config's; lora-phy's futures are big.
    std::thread::Builder::new()
        .stack_size(32768)
        .spawn(move || block_on(async { radio::init(pins).await.await }))
        .unwrap();
    ThreadSpawnConfiguration::default().set().unwrap();
}
