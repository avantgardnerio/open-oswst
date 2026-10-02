//! SPI wake-path bench: where does ~1ms per SPI call go, when the wire time
//! is 20-120µs? Every call through lora-phy goes: transfer queued -> SPI
//! interrupt -> the hal's callback -> its IsrReactor task -> our task's waker
//! -> our task runs again.
//!
//! Sends the SX1262 a harmless GetStatus (2 bytes) CALLS times per path, from
//! a thread placed like the app's radio (core 1, priority 10), 1ms apart:
//!   1. raw-polling: ESP-IDF's polling transfer. It spins, so it's never a
//!      candidate: it's the floor, the wire plus the driver
//!   2. raw-blocking: ESP-IDF's spi_device_transmit. The thread sleeps until
//!      the transfer's interrupt
//!   3. raw-async: ESP-IDF's queue, with our own completion callback that
//!      wakes our task straight from the interrupt (no reactor). Also split:
//!      queued -> interrupt -> our task running
//!   4. hal-async: esp-idf-hal's async SPI, as lora-phy uses it today
//!
//! No RF, no lora-phy: only SPI. Our block_on's waker is safe to call from an
//! interrupt (it notifies the task FromISR and yields), which is what makes
//! path 3 possible.
//!
//! Build & flash: cargo build --bin spi_wake && espflash flash -p <PORT> --bootloader <built bootloader.bin> --partition-table target/xtensa-esp32s3-espidf/debug/partition-table.bin target/xtensa-esp32s3-espidf/debug/spi_wake

use core::future::poll_fn;
use core::task::Poll;
use embassy_sync::waitqueue::AtomicWaker;
use embedded_hal_async::spi::{Operation, SpiDevice};
use esp_idf_svc::hal::cpu::Core;
use esp_idf_svc::hal::gpio::PinDriver;
use esp_idf_svc::hal::spi::config::Config as SpiConfig;
use esp_idf_svc::hal::spi::{SpiDeviceDriver, SpiDriverConfig};
use esp_idf_svc::hal::task::block_on;
use esp_idf_svc::hal::task::thread::ThreadSpawnConfiguration;
use esp_idf_svc::hal::units::Hertz;
use esp_idf_svc::sys::*;
use open_oswst::board;
use open_oswst::devices::radio;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Calls per path
const CALLS: usize = 500;
const SPI_HZ: u32 = 2_000_000;
/// The radio's SPI pins (see board.rs), by number for the raw ESP-IDF API
const SCK: i32 = 9;
const MOSI: i32 = 10;
const MISO: i32 = 11;
const NSS: i32 = 8;
/// SX1262 GetStatus, then a NOP clocking the status out
const GET_STATUS: [u8; 2] = [0xC0, 0x00];

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("spi_wake starting");

    let radio::Peripherals {
        spi,
        sck,
        mosi,
        miso,
        nss,
        reset,
        ..
    } = board::take().radio;
    // The SX1262 sits in reset while this is low
    let mut reset = PinDriver::output(reset).unwrap();
    reset.set_high().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));

    ThreadSpawnConfiguration {
        name: Some(c"radio"),
        priority: 10,
        pin_to_core: Some(Core::Core1),
        ..Default::default()
    }
    .set()
    .unwrap();
    std::thread::Builder::new()
        .stack_size(16384)
        .spawn(move || block_on(run(spi, sck, mosi, miso, nss)))
        .unwrap();
    ThreadSpawnConfiguration::default().set().unwrap();

    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

async fn run(
    spi: esp_idf_svc::hal::spi::SPI2<'static>,
    sck: esp_idf_svc::hal::gpio::AnyIOPin<'static>,
    mosi: esp_idf_svc::hal::gpio::AnyIOPin<'static>,
    miso: esp_idf_svc::hal::gpio::AnyIOPin<'static>,
    nss: esp_idf_svc::hal::gpio::AnyIOPin<'static>,
) {
    log::info!("on core {:?}", esp_idf_svc::hal::cpu::core());
    // Allocated once: nothing on the measured path touches the heap
    let mut total = vec![0i64; CALLS];
    let mut to_isr = vec![0i64; CALLS];
    let mut to_task = vec![0i64; CALLS];

    // 1-3 on a raw ESP-IDF device, freed before the hal takes the bus
    let dev = unsafe { raw_device() };
    for us in total.iter_mut() {
        *us = time(|| unsafe { esp!(spi_device_polling_transmit(dev, &mut get_status())) });
        pause().await;
    }
    report("raw-polling", &mut total);
    for us in total.iter_mut() {
        *us = time(|| unsafe { esp!(spi_device_transmit(dev, &mut get_status())) });
        pause().await;
    }
    report("raw-blocking", &mut total);
    for i in 0..CALLS {
        let (queued, isr, resumed) = unsafe { raw_async_call(dev).await };
        total[i] = resumed - queued;
        to_isr[i] = isr - queued;
        to_task[i] = resumed - isr;
        pause().await;
    }
    report("raw-async", &mut total);
    report("  queued->isr", &mut to_isr);
    report("  isr->task", &mut to_task);
    unsafe {
        esp!(spi_bus_remove_device(dev)).unwrap();
        esp!(spi_bus_free(spi_host_device_t_SPI2_HOST)).unwrap();
    }

    // 4: the hal's async path
    let mut hal = SpiDeviceDriver::new_single(
        spi,
        sck,
        mosi,
        Some(miso),
        Some(nss),
        &SpiDriverConfig::new(),
        &SpiConfig::new().baudrate(Hertz(SPI_HZ)),
    )
    .unwrap();
    for us in total.iter_mut() {
        let mut rx = [0u8; 2];
        let start = now();
        SpiDevice::transaction(&mut hal, &mut [Operation::Transfer(&mut rx, &GET_STATUS)])
            .await
            .unwrap();
        *us = now() - start;
        pause().await;
    }
    report("hal-async", &mut total);
    log::info!("done");
}

/// The radio's SPI bus and the SX1262 on it, through ESP-IDF directly, with
/// our completion callback.
unsafe fn raw_device() -> spi_device_handle_t {
    let bus = spi_bus_config_t {
        __bindgen_anon_1: spi_bus_config_t__bindgen_ty_1 { mosi_io_num: MOSI },
        __bindgen_anon_2: spi_bus_config_t__bindgen_ty_2 { miso_io_num: MISO },
        sclk_io_num: SCK,
        __bindgen_anon_3: spi_bus_config_t__bindgen_ty_3 { quadwp_io_num: -1 },
        __bindgen_anon_4: spi_bus_config_t__bindgen_ty_4 { quadhd_io_num: -1 },
        data4_io_num: -1,
        data5_io_num: -1,
        data6_io_num: -1,
        data7_io_num: -1,
        max_transfer_sz: 64,
        ..Default::default()
    };
    esp!(spi_bus_initialize(
        spi_host_device_t_SPI2_HOST,
        &bus,
        spi_common_dma_t_SPI_DMA_CH_AUTO
    ))
    .unwrap();

    let config = spi_device_interface_config_t {
        clock_speed_hz: SPI_HZ as i32,
        mode: 0,
        spics_io_num: NSS,
        queue_size: 1,
        post_cb: Some(on_done),
        ..Default::default()
    };
    let mut dev: spi_device_handle_t = core::ptr::null_mut();
    esp!(spi_bus_add_device(
        spi_host_device_t_SPI2_HOST,
        &config,
        &mut dev
    ))
    .unwrap();
    dev
}

fn get_status() -> spi_transaction_t {
    spi_transaction_t {
        flags: SPI_TRANS_USE_TXDATA | SPI_TRANS_USE_RXDATA,
        length: GET_STATUS.len() * 8, // bits
        __bindgen_anon_1: spi_transaction_t__bindgen_ty_1 {
            tx_data: [GET_STATUS[0], GET_STATUS[1], 0, 0],
        },
        ..Default::default()
    }
}

static DONE: AtomicWaker = AtomicWaker::new();
static FIRED: AtomicBool = AtomicBool::new(false);
/// When the interrupt came: µs since boot, wrapped to 32 bits (no 64-bit
/// atomics on this chip; it only has to span one transfer)
static ISR_AT: AtomicU32 = AtomicU32::new(0);

/// ESP-IDF calls this from the SPI interrupt when a transfer is done: note
/// when, and wake our task directly.
extern "C" fn on_done(_t: *mut spi_transaction_t) {
    ISR_AT.store(now() as u32, Ordering::Relaxed);
    FIRED.store(true, Ordering::Release);
    DONE.wake();
}

/// One transfer through the queue, awaited on our callback's wake.
/// Returns when it was queued, when the interrupt came, when we ran again.
async unsafe fn raw_async_call(dev: spi_device_handle_t) -> (i64, i64, i64) {
    let mut t = get_status();
    FIRED.store(false, Ordering::Relaxed);
    let queued = now();
    esp!(spi_device_queue_trans(dev, &mut t, u32::MAX)).unwrap();
    poll_fn(|cx| {
        DONE.register(cx.waker());
        if FIRED.load(Ordering::Acquire) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
    let resumed = now();
    // Already done, so this doesn't wait
    let mut done: *mut spi_transaction_t = core::ptr::null_mut();
    esp!(spi_device_get_trans_result(dev, &mut done, 0)).unwrap();
    let isr = queued + ISR_AT.load(Ordering::Relaxed).wrapping_sub(queued as u32) as i64;
    (queued, isr, resumed)
}

/// Microseconds since boot
fn now() -> i64 {
    unsafe { esp_timer_get_time() }
}

fn time(f: impl FnOnce() -> Result<(), EspError>) -> i64 {
    let start = now();
    f().unwrap();
    now() - start
}

/// Between calls, like a real driver doing other work
async fn pause() {
    embassy_time::Timer::after_millis(1).await;
}

fn report(name: &str, us: &mut [i64]) {
    us.sort_unstable();
    let at = |f: f64| us[((us.len() - 1) as f64 * f) as usize];
    log::info!(
        "{:14} min {:5}  p50 {:5}  p90 {:5}  p99 {:5}  max {:5} us",
        name,
        us[0],
        at(0.5),
        at(0.9),
        at(0.99),
        us[us.len() - 1]
    );
}
