//! Firmware entry point: brings up the board's hardware and runs the app from
//! the core crate on it. Everything radio-behaviour lives in core.

use core::fmt::Write as _;
use embassy_futures::join::join4;
use esp_idf_svc::hal::cpu::Core;
use esp_idf_svc::hal::task::block_on;
use open_oswst::devices::{encoder, fem, gps, mic, ptt, radio, screen, settings, speaker, storage};
use open_oswst::{board, firmware, net, thread};
use open_oswst_core::devices::network::Network;
use open_oswst_core::platform::Platform;
use open_oswst_core::{air, app, codec, config, logger};
use std::path::Path;

/// The radio thread's priority: above the codec's (5), so the codec can't
/// delay a TX or an IRQ, and below the hal's IsrReactor (11), which wakes it.
const RADIO_PRIORITY: u8 = 10;

/// The app and speaker (this, the main task, on core 0). The main task starts
/// at 1, below everything: the codec, screen, GPS and HTTP server (5) could
/// all delay the speaker. Now level with the radio, which is on the other
/// core; only WiFi and the hal's IsrReactor (11) come first.
const APP_PRIORITY: u8 = 10;

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

    fn now_us() -> i64 {
        unsafe { esp_idf_svc::sys::esp_timer_get_time() }
    }

    fn network() -> Network {
        net::state()
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
    config::load(Some(&settings));
    let wifi_networks = settings.wifi_networks();
    log::info!("Config: {} WiFi network(s)", wifi_networks.len());
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
    // ~25.6KB used at worst (Stack free log, 2026-10-02): Codec2 is deep.
    // On core 1, away from the app, speaker and WiFi on core 0. The radio is
    // there too, but above it (10 vs 5), and mostly waiting
    thread::spawn(c"codec", 32768, None, Some(Core::Core1), move || {
        codec::run(codec_rx)
    });

    let (start_hz, sweep_hz) = channels();
    spawn_radio(board.radio, start_hz, sweep_hz);
    // WiFi and the HTTP API, on core 0 at a low priority (only if networks
    // are configured)
    net::start(board.modem, wifi_networks, mac_str.to_string());
    firmware::confirm_later();

    unsafe { esp_idf_svc::sys::vTaskPrioritySet(core::ptr::null_mut(), APP_PRIORITY as u32) };
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
        join4(app_fut, speaker_fut, log_memory(), log_cpu()).await;
    });
}

/// Log memory every 5 minutes: the heap (free now, the largest block one
/// allocation can get, the least ever free), and each task's stack at its
/// fullest (the least ever free). The least-evers only move when something
/// new happens, so more often only fills the log
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
        embassy_time::Timer::after_secs(300).await;
    }
}

/// Tasks the CPU log can follow; more are left out
const CPU_MAX_TASKS: usize = 24;

/// How often the CPU log samples, and how many samples make a log line
const CPU_SAMPLE_SECS: u64 = 1;
const CPU_SAMPLES_PER_LINE: u32 = 60;

/// One task's CPU time, as the CPU log follows it
struct TaskCpu {
    number: u32, // FreeRTOS's task number: names aren't unique
    name: heapless::String<16>,
    last_us: u32,   // its run-time counter at the last sample
    line_us: u64,   // run since the line began
    worst_pct: u32, // its busiest second since then
    least_pct: u32, // its idlest: for an idle task, the core's busiest
}

/// Log CPU use every 60s, like htop: each core's load and each busy task's
/// share of a core, averaged over the minute and in its worst 1s. The worst
/// second is what starves the codec; an average hides it. A core's load is
/// 100% less its idle task's share. Tasks under 1% both ways are left out
async fn log_cpu() {
    use esp_idf_svc::sys::*;
    // Both tables live across the sample timer's await, so in this future,
    // which is on the main task's stack: ~2KB there cost it ~4.5KB of
    // headroom (Stack free log, 2026-10-04). Allocated once, on the heap
    let mut tasks = Box::new(heapless::Vec::<TaskCpu, CPU_MAX_TASKS>::new());
    let mut snapshot: Box<[TaskStatus_t; CPU_MAX_TASKS]> = Box::new(unsafe { core::mem::zeroed() });
    let mut last_at = unsafe { esp_timer_get_time() };
    let mut line_us = 0u64;
    let mut samples = 0u32;
    loop {
        embassy_time::Timer::after_secs(CPU_SAMPLE_SECS).await;

        let n = unsafe {
            uxTaskGetSystemState(
                snapshot.as_mut_ptr(),
                CPU_MAX_TASKS as u32,
                core::ptr::null_mut(),
            )
        } as usize;
        let now = unsafe { esp_timer_get_time() };
        let sample_us = (now - last_at).max(1) as u64;
        last_at = now;
        // The first sample only finds the tasks: nothing ran "since" yet
        let priming = tasks.is_empty();
        line_us += sample_us;
        samples += 1;

        for status in &snapshot[..n] {
            // The counter is 32 bits of microseconds: it wraps every ~71
            // minutes, so take differences with wrapping
            let counter = status.ulRunTimeCounter;
            let number = status.xTaskNumber;
            match tasks.iter_mut().find(|t| t.number == number) {
                Some(task) => {
                    let ran_us = counter.wrapping_sub(task.last_us) as u64;
                    task.last_us = counter;
                    task.line_us += ran_us;
                    let pct = (ran_us * 100 / sample_us) as u32;
                    task.worst_pct = task.worst_pct.max(pct);
                    task.least_pct = task.least_pct.min(pct);
                }
                // New: counted from the next sample on
                None => {
                    let name = unsafe { core::ffi::CStr::from_ptr(status.pcTaskName) };
                    let mut task = TaskCpu {
                        number,
                        name: heapless::String::new(),
                        last_us: counter,
                        line_us: 0,
                        worst_pct: 0,
                        least_pct: u32::MAX,
                    };
                    let _ = task.name.push_str(&name.to_string_lossy());
                    let _ = tasks.push(task);
                }
            }
        }

        if priming {
            line_us = 0;
            samples = 0;
            continue;
        }
        if samples < CPU_SAMPLES_PER_LINE {
            continue;
        }
        let avg_pct = |task: &TaskCpu| (task.line_us * 100 / line_us) as u32;
        let mut line = heapless::String::<256>::new();
        // Cores: 100% less idle. Idle's idlest second is the core's busiest
        for core in ["IDLE0", "IDLE1"] {
            if let Some(idle) = tasks.iter().find(|t| t.name == core) {
                let _ = write!(
                    line,
                    " core{} {}/{}",
                    &core[4..],
                    100u32.saturating_sub(avg_pct(idle)),
                    100u32.saturating_sub(idle.least_pct.min(100))
                );
            }
        }
        let _ = write!(line, " |");
        tasks.sort_unstable_by_key(|t| core::cmp::Reverse(t.line_us));
        for task in tasks.iter() {
            let avg = avg_pct(task);
            if task.name.starts_with("IDLE") || (avg < 1 && task.worst_pct < 1) {
                continue;
            }
            let _ = write!(line, " {} {}/{}", task.name, avg, task.worst_pct);
        }
        log::info!("CPU % ({}s avg/worst 1s):{}", line_us / 1_000_000, line);

        for task in tasks.iter_mut() {
            task.line_us = 0;
            task.worst_pct = 0;
            task.least_pct = u32::MAX;
        }
        line_us = 0;
        samples = 0;
    }
}

/// The radio gets its own thread on core 1, so nothing else running can
/// delay it (on core 0 with the codec, a TX step took up to 200ms instead of
/// 75). It talks to the app only through RX_CHAN and TX_CHAN.
/// From the config: the channel we send on (the first hop channel), and
/// the channels to sweep while idle (none unless the sweep flag is on).
/// Nothing hops yet: we send on the first one only
fn channels() -> (u32, Vec<u32>) {
    let hops = air::hop_slots(
        config::START_SLOT.get() as u32,
        config::HOP_SEED.get() as u64,
        config::RX_HOPS.get() as u32,
    );
    let start_hz = air::slot_hz(hops[0]);
    log::info!("Channel: slot {} = {} Hz", hops[0], start_hz);
    if !config::SWEEP.is_on() {
        return (start_hz, Vec::new());
    }
    log::info!("Channel: sweeping slots {:?}", hops);
    (start_hz, hops.into_iter().map(air::slot_hz).collect())
}

fn spawn_radio(pins: radio::Peripherals, frequency_hz: u32, sweep_hz: Vec<u32>) {
    // The radio is created on this thread, not moved to it: created on core 0
    // and moved, it hung (src/bin/radio_timing.rs). lora-phy's futures are big.
    thread::spawn(
        c"radio",
        // 16KB overflowed with the sweep (CAD) path added (2026-10-03);
        // ~9.6KB used at worst before it (Stack free log, 2026-10-02)
        24576,
        Some(RADIO_PRIORITY),
        Some(Core::Core1),
        move || block_on(async { radio::init(pins, frequency_hz, sweep_hz).await.await }),
    );
}
