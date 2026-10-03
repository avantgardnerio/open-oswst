//! Firmware entry point: brings up the board's hardware and runs the app from
//! the core crate on it. Everything radio-behaviour lives in core.

use core::fmt::Write as _;
use embassy_futures::join::join3;
use esp_idf_svc::hal::cpu::Core;
use esp_idf_svc::hal::task::block_on;
use open_oswst::devices::{encoder, fem, gps, mic, ptt, radio, screen, settings, speaker, storage};
use open_oswst::{board, net, thread};
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

    // After storage: the settings are a file on it
    let settings = settings::init();
    let mode = mode::load(Some(&settings));
    log::info!(
        "Config: mode={:?}, {} WiFi network(s)",
        mode,
        settings.wifi_networks().len()
    );
    let wifi_networks = settings.wifi_networks();
    let settings = Some(settings);

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
    // ~25.6KB used at worst (Stack free log, 2026-10-02): Codec2 is deep
    thread::spawn(c"codec", 32768, None, None, move || codec::run(codec_rx));

    spawn_radio(board.radio);
    // WiFi and the HTTP API, on core 0 at a low priority (only if networks
    // are configured)
    net::start(board.modem, wifi_networks, mac_str.to_string());

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
        join3(app_fut, speaker_fut, log_memory()).await;
    });
}

/// Log memory every 30s, to size the WiFi and HTTP stack before adding it:
/// the heap (free now, the largest block one allocation can get, the least
/// ever free), and each task's stack at its fullest (the least ever free).
async fn log_memory() {
    use esp_idf_svc::sys::*;
    const MAX_TASKS: usize = 24;
    loop {
        unsafe {
            log::info!(
                "Heap: free {} B, largest block {} B, min ever free {} B",
                esp_get_free_heap_size(),
                heap_caps_get_largest_free_block(MALLOC_CAP_8BIT),
                esp_get_minimum_free_heap_size()
            );
            let mut tasks: [TaskStatus_t; MAX_TASKS] = core::mem::zeroed();
            let n =
                uxTaskGetSystemState(tasks.as_mut_ptr(), MAX_TASKS as u32, core::ptr::null_mut())
                    as usize;
            let mut line = heapless::String::<512>::new();
            for task in &tasks[..n] {
                let name = core::ffi::CStr::from_ptr(task.pcTaskName).to_string_lossy();
                let _ = write!(line, " {}={}", name, task.usStackHighWaterMark);
            }
            log::info!("Stack free (least ever, B):{}", line);
        }
        embassy_time::Timer::after_secs(30).await;
    }
}

/// The radio gets its own thread on core 1, so nothing else running can
/// delay it (on core 0 with the codec, a TX step took up to 200ms instead of
/// 75). It talks to the app only through RX_CHAN and TX_CHAN.
fn spawn_radio(pins: radio::Peripherals) {
    // The radio is created on this thread, not moved to it: created on core 0
    // and moved, it hung (src/bin/radio_timing.rs). lora-phy's futures are big.
    thread::spawn(
        c"radio",
        // ~9.6KB used at worst (Stack free log, 2026-10-02)
        16384,
        Some(RADIO_PRIORITY),
        Some(Core::Core1),
        move || block_on(async { radio::init(pins).await.await }),
    );
}
