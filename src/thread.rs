//! Spawning our long-lived threads with their FreeRTOS task name set. std's
//! thread name never reaches FreeRTOS, so without this every thread shows up
//! as "pthread" in the stack report (`Stack free` in main.rs).

use esp_idf_svc::hal::cpu::Core;
use esp_idf_svc::hal::task::thread::ThreadSpawnConfiguration;
use std::ffi::CStr;

/// Spawn `f` as FreeRTOS task `name` with `stack` bytes, at `priority` (None:
/// the pthread default, 5), on `core` (None: whichever is free).
pub fn spawn(
    name: &'static CStr,
    stack: usize,
    priority: Option<u8>,
    core: Option<Core>,
    f: impl FnOnce() + Send + 'static,
) {
    let mut config = ThreadSpawnConfiguration {
        name: Some(name),
        pin_to_core: core,
        ..Default::default()
    };
    if let Some(priority) = priority {
        config.priority = priority;
    }
    config.set().unwrap();
    // std's stack size wins over the spawn config's, so it's set here
    std::thread::Builder::new()
        .stack_size(stack)
        .spawn(f)
        .unwrap();
    ThreadSpawnConfiguration::default().set().unwrap();
}
