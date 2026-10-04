//! The radio's SPI bus and its BUSY and DIO1 pins, for lora-phy, with
//! interrupts that wake the radio task directly.
//!
//! Through esp-idf-hal, every SPI command and every pin wait goes: interrupt
//! -> the hal's callback -> its IsrReactor task -> our task's waker -> our
//! task. That relay costs ~300us each time, so a command took ~370us for
//! 20-120us on the wire (src/bin/spi_wake.rs). Here the interrupt wakes our
//! task itself: ~60us. Our block_on's waker is safe to call from an
//! interrupt (it notifies the task FromISR), and the waker is kept in a
//! lock-free AtomicWaker.
//!
//! - RadioSpi: lora-phy's SPI device. Each lora-phy transaction (a command,
//!   then status or data) is one ESP-IDF transfer, chip select held low
//!   throughout, through buffers DMA can reach that are allocated once
//! - RadioPin: BUSY and DIO1. lora-phy waits for their levels
//!
//! Neither interrupt is in IRAM (CONFIG_SPI_MASTER_ISR_IN_IRAM and
//! CONFIG_GPIO_CTRL_FUNC_IN_IRAM are off): while the flash is being written
//! they wait, so the callbacks can live in flash too.
//!
//! One radio only: the wake-ups are statics.

use core::future::poll_fn;
use core::task::Poll;
// Not embassy's AtomicWaker: that one takes a critical section, which here
// is a FreeRTOS mutex, and a mutex can't be taken in an interrupt. This one is
// only atomics (esp-idf-hal uses it for the same reason)
use atomic_waker::AtomicWaker;
use embedded_hal::digital::ErrorType as PinErrorType;
use embedded_hal::spi::ErrorType as SpiErrorType;
use embedded_hal_async::digital::Wait;
use embedded_hal_async::spi::{Operation, SpiDevice};
use esp_idf_svc::hal::gpio::{AnyIOPin, AnyInputPin, Input, Output, Pin, PinDriver, Pull};
use esp_idf_svc::hal::spi::SPI2;
use esp_idf_svc::sys::*;
use lora_phy::iv::GenericSx126xInterfaceVariant;
use std::sync::atomic::{AtomicBool, Ordering};

use super::radio;

/// SPI clock, as with esp-idf-hal (devices::radio)
const SPI_HZ: i32 = 2_000_000;

/// The longest lora-phy transaction: reading a 255-byte packet out is the
/// command, its offset, a status byte and the packet: 258
const MAX_TRANSFER: usize = 264;

/// What lora-phy's SX126x driver is built from
pub type Interface = GenericSx126xInterfaceVariant<PinDriver<'static, Output>, RadioPin>;

/// The radio's SPI device and interface pins, from the board's radio
/// peripherals. RESET and the FEM's TX switch are plain outputs, as before
pub fn take(p: radio::Peripherals) -> (RadioSpi, Interface) {
    let spi = RadioSpi::new(p.spi, p.sck, p.mosi, p.miso, p.nss);
    let reset = PinDriver::output(p.reset).unwrap();
    // Both pins disarmed before the interrupt service goes in (RadioPin::new)
    let dio1 = RadioPin::new(p.dio1);
    let busy = RadioPin::new(p.busy);
    // The GPIO interrupt service is shared with esp-idf-hal: install it
    // through the hal, so neither installs it twice
    esp_idf_svc::hal::gpio::enable_isr_service().unwrap();
    dio1.listen();
    busy.listen();
    let rf_switch_tx = p.rf_switch_tx.map(|pin| PinDriver::output(pin).unwrap());
    let iv = GenericSx126xInterfaceVariant::new(reset, dio1, busy, None, rf_switch_tx).unwrap();
    (spi, iv)
}

/// Something went wrong on the bus: ESP-IDF refused a transfer, or lora-phy
/// asked for one longer than MAX_TRANSFER
#[derive(Debug)]
pub struct Error;

impl embedded_hal::spi::Error for Error {
    fn kind(&self) -> embedded_hal::spi::ErrorKind {
        embedded_hal::spi::ErrorKind::Other
    }
}

impl embedded_hal::digital::Error for Error {
    fn kind(&self) -> embedded_hal::digital::ErrorKind {
        embedded_hal::digital::ErrorKind::Other
    }
}

// --- SPI ---

/// 4-byte aligned, which DMA wants: otherwise ESP-IDF copies through a
/// buffer it allocates on every transfer
#[repr(C, align(4))]
struct Buffer([u8; MAX_TRANSFER]);

pub struct RadioSpi {
    device: spi_device_handle_t,
    tx: Box<Buffer>,
    rx: Box<Buffer>,
    // Owned, so nothing else can use the bus or its pins
    _spi: SPI2<'static>,
    _pins: [AnyIOPin<'static>; 4],
}

/// Set by the transfer-done callback, which wakes the waiting task
static SPI_DONE: AtomicBool = AtomicBool::new(false);
static SPI_WAKER: AtomicWaker = AtomicWaker::new();

impl RadioSpi {
    fn new(
        spi: SPI2<'static>,
        sck: AnyIOPin<'static>,
        mosi: AnyIOPin<'static>,
        miso: AnyIOPin<'static>,
        nss: AnyIOPin<'static>,
    ) -> Self {
        let bus = spi_bus_config_t {
            __bindgen_anon_1: spi_bus_config_t__bindgen_ty_1 {
                mosi_io_num: mosi.pin() as i32,
            },
            __bindgen_anon_2: spi_bus_config_t__bindgen_ty_2 {
                miso_io_num: miso.pin() as i32,
            },
            sclk_io_num: sck.pin() as i32,
            __bindgen_anon_3: spi_bus_config_t__bindgen_ty_3 { quadwp_io_num: -1 },
            __bindgen_anon_4: spi_bus_config_t__bindgen_ty_4 { quadhd_io_num: -1 },
            data4_io_num: -1,
            data5_io_num: -1,
            data6_io_num: -1,
            data7_io_num: -1,
            max_transfer_sz: MAX_TRANSFER as i32,
            ..Default::default()
        };
        let config = spi_device_interface_config_t {
            clock_speed_hz: SPI_HZ,
            mode: 0,
            spics_io_num: nss.pin() as i32,
            queue_size: 1,
            post_cb: Some(on_spi_done),
            ..Default::default()
        };
        let mut device: spi_device_handle_t = core::ptr::null_mut();
        unsafe {
            esp!(spi_bus_initialize(
                spi_host_device_t_SPI2_HOST,
                &bus,
                spi_common_dma_t_SPI_DMA_CH_AUTO
            ))
            .unwrap();
            esp!(spi_bus_add_device(
                spi_host_device_t_SPI2_HOST,
                &config,
                &mut device
            ))
            .unwrap();
        }
        RadioSpi {
            device,
            tx: Box::new(Buffer([0; MAX_TRANSFER])),
            rx: Box::new(Buffer([0; MAX_TRANSFER])),
            _spi: spi,
            _pins: [sck, mosi, miso, nss],
        }
    }

    /// Send the first `len` bytes of `tx`, filling `rx`, in one transfer
    async fn transfer(&mut self, len: usize) -> Result<(), Error> {
        if len == 0 {
            return Ok(());
        }
        let mut transaction = spi_transaction_t {
            length: len * 8, // bits
            __bindgen_anon_1: spi_transaction_t__bindgen_ty_1 {
                tx_buffer: self.tx.0.as_ptr() as *const _,
            },
            __bindgen_anon_2: spi_transaction_t__bindgen_ty_2 {
                rx_buffer: self.rx.0.as_mut_ptr() as *mut _,
            },
            ..Default::default()
        };
        SPI_DONE.store(false, Ordering::Relaxed);
        // The transaction lives in this future, which stays put until the
        // transfer is done and collected below
        esp!(unsafe { spi_device_queue_trans(self.device, &mut transaction, u32::MAX) })
            .map_err(|_| Error)?;
        poll_fn(|cx| {
            SPI_WAKER.register(cx.waker());
            if SPI_DONE.load(Ordering::Acquire) {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
        // Already done, so this doesn't wait
        let mut done: *mut spi_transaction_t = core::ptr::null_mut();
        esp!(unsafe { spi_device_get_trans_result(self.device, &mut done, 0) }).map_err(|_| Error)
    }
}

/// ESP-IDF calls this from the SPI interrupt when a transfer is done
extern "C" fn on_spi_done(_transaction: *mut spi_transaction_t) {
    SPI_DONE.store(true, Ordering::Release);
    SPI_WAKER.wake();
}

impl SpiErrorType for RadioSpi {
    type Error = Error;
}

impl SpiDevice<u8> for RadioSpi {
    /// Lay every operation out in one buffer (writes as given, reads as
    /// zeros), transfer it, then hand each read its part of what came back
    async fn transaction(&mut self, operations: &mut [Operation<'_, u8>]) -> Result<(), Error> {
        let mut len = 0;
        for operation in operations.iter() {
            let n = operation_len(operation)?;
            if len + n > MAX_TRANSFER {
                return Err(Error);
            }
            let tx = &mut self.tx.0[len..len + n];
            match operation {
                Operation::Write(bytes) => tx.copy_from_slice(bytes),
                Operation::Transfer(_, bytes) => {
                    tx[..bytes.len()].copy_from_slice(bytes);
                    tx[bytes.len()..].fill(0);
                }
                Operation::TransferInPlace(bytes) => tx.copy_from_slice(bytes),
                Operation::Read(_) | Operation::DelayNs(_) => tx.fill(0),
            }
            len += n;
        }

        self.transfer(len).await?;

        let mut at = 0;
        for operation in operations.iter_mut() {
            let n = operation_len(operation)?;
            let rx = &self.rx.0[at..at + n];
            match operation {
                Operation::Read(bytes) => bytes.copy_from_slice(rx),
                Operation::Transfer(bytes, _) => bytes.copy_from_slice(&rx[..bytes.len()]),
                Operation::TransferInPlace(bytes) => bytes.copy_from_slice(rx),
                Operation::Write(_) | Operation::DelayNs(_) => {}
            }
            at += n;
        }
        Ok(())
    }
}

/// Bytes an operation takes on the wire. lora-phy never asks for a delay
/// inside a transaction, and one here couldn't be honoured: refuse it
fn operation_len(operation: &Operation<'_, u8>) -> Result<usize, Error> {
    match operation {
        Operation::Write(bytes) => Ok(bytes.len()),
        Operation::Read(bytes) => Ok(bytes.len()),
        Operation::Transfer(read, write) => Ok(read.len().max(write.len())),
        Operation::TransferInPlace(bytes) => Ok(bytes.len()),
        Operation::DelayNs(_) => Err(Error),
    }
}

// --- BUSY and DIO1 ---

pub struct RadioPin {
    gpio: i32,
    // Keeps the pin configured as an input, and owned
    _driver: PinDriver<'static, Input>,
}

/// Per GPIO: set by the pin's interrupt, which wakes the waiting task
struct PinWake {
    fired: AtomicBool,
    waker: AtomicWaker,
}

static PIN_WAKES: [PinWake; SOC_GPIO_PIN_COUNT as usize] = [const {
    PinWake {
        fired: AtomicBool::new(false),
        waker: AtomicWaker::new(),
    }
}; SOC_GPIO_PIN_COUNT as usize];

impl RadioPin {
    /// An input with its interrupt disarmed. A software reboot (OTA, panic)
    /// keeps the GPIO's interrupt settings and doesn't reset the SX1262: the
    /// last image's level interrupt can still be armed, its level still
    /// there (BUSY low, DIO1 high). Installing the interrupt service then
    /// fires it with no handler to disarm it, forever: the interrupt
    /// watchdog reset the repeater on every boot. So every radio pin is
    /// disarmed before anything installs the service (`listen`)
    fn new(pin: AnyInputPin<'static>) -> Self {
        let gpio = pin.pin() as i32;
        let driver = PinDriver::input(pin, Pull::Floating).unwrap();
        unsafe {
            esp!(gpio_intr_disable(gpio)).unwrap();
            esp!(gpio_set_intr_type(gpio, gpio_int_type_t_GPIO_INTR_DISABLE)).unwrap();
        }
        RadioPin {
            gpio,
            _driver: driver,
        }
    }

    /// Hand the pin's interrupt to on_pin. The service must be installed
    fn listen(&self) {
        unsafe {
            esp!(gpio_isr_handler_add(
                self.gpio,
                Some(on_pin),
                self.gpio as *mut core::ffi::c_void
            ))
            .unwrap();
        }
    }

    /// Wait for the pin's level (or edge) to come. A level that's already
    /// there returns at once. Safe to drop mid-wait: the next wait disarms
    /// the interrupt before anything else
    async fn wait_for(&mut self, trigger: gpio_int_type_t) -> Result<(), Error> {
        let wake = &PIN_WAKES[self.gpio as usize];
        unsafe { esp!(gpio_intr_disable(self.gpio)).map_err(|_| Error)? };
        wake.fired.store(false, Ordering::Relaxed);

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
        poll_fn(|cx| {
            wake.waker.register(cx.waker());
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

/// ESP-IDF calls this from the GPIO interrupt. Disarm first: a level
/// interrupt keeps firing for as long as the level holds
unsafe extern "C" fn on_pin(arg: *mut core::ffi::c_void) {
    let gpio = arg as i32;
    gpio_intr_disable(gpio);
    let wake = &PIN_WAKES[gpio as usize];
    wake.fired.store(true, Ordering::Release);
    wake.waker.wake();
}

impl PinErrorType for RadioPin {
    type Error = Error;
}

impl Wait for RadioPin {
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
