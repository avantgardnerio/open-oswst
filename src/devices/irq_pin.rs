//! Input pins whose interrupt wakes the task waiting on them, from IRAM:
//! the radio's DIO1 and BUSY, PTT and the encoder.
//!
//! A flash write (a log flush, an OTA, a config save) turns the caches off
//! on both cores for its length, and only interrupts that live in IRAM run
//! meanwhile. DIO1's interrupt notes when a packet ended (`fired_at_us`), so
//! it must run on time: with the handler in flash, a flush stamped a packet
//! ~7.8 ms late and threw a whole transmission off its slot grid (#36).
//!
//! So the whole GPIO interrupt path is IRAM-safe:
//! - the GPIO interrupt service is installed with ESP_INTR_FLAG_IRAM, and
//!   its dispatcher is IRAM_ATTR
//! - `gpio_intr_disable` is in IRAM (CONFIG_GPIO_CTRL_FUNC_IN_IRAM)
//! - esp_timer_get_time and FreeRTOS are in IRAM by default
//! - `on_pin` is in IRAM and calls only those
//!
//! One dispatcher serves every pin, so every pin with a handler must be
//! IRAM-safe: that's why PTT and the encoder wait here too, not through
//! esp-idf-hal's PinDriver (its callbacks are in flash).
//!
//! The interrupt wakes the task with a FreeRTOS notification, not a Rust
//! waker: a waker's code is in flash. esp-idf-hal's `block_on` (the radio
//! thread's and the main task's executor) sleeps until its task is notified
//! on index 0, so that notification is the wake-up. A future polled by any
//! other executor would never wake.
//!
//! While the flash is busy the waiting task can't run either (its code is in
//! flash too): the interrupt stamps the time and leaves the notification
//! pending, and the task picks it up when the write ends.

use core::future::poll_fn;
use core::task::Poll;
use embedded_hal::digital::ErrorType;
use embedded_hal_async::digital::Wait;
use esp_idf_svc::hal::gpio::{enable_isr_service, init_isr_alloc_flags, AnyInputPin, Input};
use esp_idf_svc::hal::gpio::{Pin, PinDriver, Pull};
use esp_idf_svc::hal::interrupt::InterruptType;
use esp_idf_svc::sys::*;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};

/// Per GPIO: set by the pin's interrupt, read by the waiting task. Statics
/// are in DRAM, which the interrupt can reach while the caches are off
struct PinWake {
    fired: AtomicBool,
    /// When it fired: the low 32 bits of esp_timer's µs (Xtensa has no
    /// 64-bit atomics; `fired_at_us` rebuilds the rest)
    fired_at_us: AtomicU32,
    /// The task waiting on the pin, to notify
    task: AtomicPtr<tskTaskControlBlock>,
}

static PIN_WAKES: [PinWake; SOC_GPIO_PIN_COUNT as usize] = [const {
    PinWake {
        fired: AtomicBool::new(false),
        fired_at_us: AtomicU32::new(0),
        task: AtomicPtr::new(core::ptr::null_mut()),
    }
}; SOC_GPIO_PIN_COUNT as usize];

/// Install the GPIO interrupt service, IRAM-safe. Once, before any IrqPin
/// (board::take).
///
/// Every pin's interrupt is disarmed first. A software reboot (OTA, panic)
/// keeps the GPIO's interrupt settings and doesn't reset the SX1262: the last
/// image's level interrupt can still be armed, its level still there (BUSY
/// low, DIO1 high). Installing the service then fires it with no handler to
/// disarm it, forever: the interrupt watchdog reset the repeater on every boot
pub fn install() {
    for gpio in 0..SOC_GPIO_PIN_COUNT as i32 {
        if SOC_GPIO_VALID_GPIO_MASK & (1 << gpio) == 0 {
            continue; // GPIO22-25 don't exist on the S3
        }
        unsafe {
            esp!(gpio_intr_disable(gpio)).unwrap();
            esp!(gpio_set_intr_type(gpio, gpio_int_type_t_GPIO_INTR_DISABLE)).unwrap();
        }
    }
    // Shared with esp-idf-hal: installed through the hal, so neither
    // installs it twice
    init_isr_alloc_flags(InterruptType::Iram.into());
    enable_isr_service().unwrap();
}

pub struct IrqPin {
    gpio: i32,
    // Keeps the pin configured as an input, and owned
    driver: PinDriver<'static, Input>,
}

impl IrqPin {
    /// An input, its interrupt handed to on_pin and disarmed until a wait.
    /// The service must be installed (`install`)
    pub fn new(pin: AnyInputPin<'static>, pull: Pull) -> Self {
        let gpio = pin.pin() as i32;
        let driver = PinDriver::input(pin, pull).unwrap();
        unsafe {
            esp!(gpio_isr_handler_add(
                gpio,
                Some(on_pin),
                gpio as *mut core::ffi::c_void
            ))
            .unwrap();
        }
        IrqPin { gpio, driver }
    }

    pub fn is_high(&self) -> bool {
        self.driver.is_high()
    }

    pub fn is_low(&self) -> bool {
        self.driver.is_low()
    }

    /// Wait for the pin's level (or edge) to come. A level that's already
    /// there returns at once. Safe to drop mid-wait: the next wait disarms
    /// the interrupt before anything else, and a stray notification from
    /// the old one only makes block_on poll once more
    async fn wait_for(&mut self, trigger: gpio_int_type_t) -> Result<(), Error> {
        let wake = &PIN_WAKES[self.gpio as usize];
        unsafe { esp!(gpio_intr_disable(self.gpio)).map_err(|_| Error)? };
        wake.fired.store(false, Ordering::Relaxed);
        wake.task
            .store(unsafe { xTaskGetCurrentTaskHandle() }, Ordering::Relaxed);

        // An edge only counts if it happens from now on
        let level = unsafe { gpio_get_level(self.gpio) };
        let already = (trigger == gpio_int_type_t_GPIO_INTR_HIGH_LEVEL && level == 1)
            || (trigger == gpio_int_type_t_GPIO_INTR_LOW_LEVEL && level == 0);
        if already {
            return Ok(());
        }

        // A level interrupt armed while the level is already there fires at
        // once, so nothing between the check and here is lost
        unsafe {
            esp!(gpio_set_intr_type(self.gpio, trigger)).map_err(|_| Error)?;
            esp!(gpio_intr_enable(self.gpio)).map_err(|_| Error)?;
        }
        poll_fn(|_| {
            if wake.fired.load(Ordering::Acquire) {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
        Ok(())
    }
}

/// The GPIO interrupt service calls this, from IRAM, even mid flash write.
/// Disarm first: a level interrupt keeps firing for as long as the level
/// holds. Nothing here may touch flash: no Rust helpers that might not
/// inline, no panic paths (`get`, not indexing: a panic's code is in
/// flash), no waker. Checked in the disassembly: every call and constant
/// it loads is in IRAM or DRAM
#[link_section = ".iram1.irq_pin_on_pin"]
unsafe extern "C" fn on_pin(arg: *mut core::ffi::c_void) {
    let gpio = arg as i32;
    gpio_intr_disable(gpio);
    // Always there: IrqPin::new registered it from a real pin
    let Some(wake) = PIN_WAKES.get(gpio as usize) else {
        return;
    };
    wake.fired_at_us
        .store(esp_timer_get_time() as u32, Ordering::Relaxed);
    wake.fired.store(true, Ordering::Release);

    let task = wake.task.load(Ordering::Relaxed);
    if task.is_null() {
        return;
    }
    let mut higher_priority_woken: BaseType_t = 0;
    xTaskGenericNotifyFromISR(
        task,
        0, // block_on's index
        1,
        eNotifyAction_eSetBits,
        core::ptr::null_mut(),
        &mut higher_priority_woken,
    );
    // portYIELD_FROM_ISR: switch to the woken task when the interrupt ends,
    // not at the next tick
    if higher_priority_woken != 0 {
        _frxt_setup_switch();
    }
}

/// When `gpio`'s interrupt ended its last wait, in µs since boot (esp_timer).
/// None if that wait found its level already there: no interrupt, so no
/// time. `now_us` is esp_timer's time now: the interrupt's is rebuilt from
/// its low 32 bits, so ask within ~71 minutes of it
pub fn fired_at_us(gpio: i32, now_us: i64) -> Option<i64> {
    let wake = &PIN_WAKES[gpio as usize];
    if !wake.fired.load(Ordering::Acquire) {
        return None;
    }
    let since_us = (now_us as u32).wrapping_sub(wake.fired_at_us.load(Ordering::Relaxed));
    Some(now_us - since_us as i64)
}

/// ESP-IDF refused to arm or disarm the pin's interrupt
#[derive(Debug)]
pub struct Error;

impl embedded_hal::digital::Error for Error {
    fn kind(&self) -> embedded_hal::digital::ErrorKind {
        embedded_hal::digital::ErrorKind::Other
    }
}

impl ErrorType for IrqPin {
    type Error = Error;
}

impl Wait for IrqPin {
    async fn wait_for_high(&mut self) -> Result<(), Error> {
        self.wait_for(gpio_int_type_t_GPIO_INTR_HIGH_LEVEL).await
    }

    async fn wait_for_low(&mut self) -> Result<(), Error> {
        self.wait_for(gpio_int_type_t_GPIO_INTR_LOW_LEVEL).await
    }

    async fn wait_for_rising_edge(&mut self) -> Result<(), Error> {
        self.wait_for(gpio_int_type_t_GPIO_INTR_POSEDGE).await
    }

    async fn wait_for_falling_edge(&mut self) -> Result<(), Error> {
        self.wait_for(gpio_int_type_t_GPIO_INTR_NEGEDGE).await
    }

    async fn wait_for_any_edge(&mut self) -> Result<(), Error> {
        self.wait_for(gpio_int_type_t_GPIO_INTR_ANYEDGE).await
    }
}
