//! The push-to-talk button: GPIO0 (the PRG button), active LOW with the
//! internal pull-up.
//!
//! Debounced: a press or release only counts once the pin has held its new
//! level for STEADY_MS. The pin's interrupt (devices::irq_pin) fires on the
//! first edge, so without this a contact glitch counted as a release: on
//! the walk of 2026-10-07 a press ended after ~0.4 s with no packets sent,
//! and a new one started 10 ms later, twice. (Through esp-idf-hal's slower
//! pin waits, before 2026-10-06, it never showed up.)

use embassy_futures::select::{select, Either};
use embassy_time::Timer;
use embedded_hal_async::digital::Wait;
use esp_idf_svc::hal::gpio::{AnyIOPin, Pull};

use super::irq_pin::IrqPin;

/// How long the pin must hold a level before it counts. The glitches seen
/// reversed within ~10 ms; a real press or release is only this much later
const STEADY_MS: u64 = 30;

pub struct Ptt(IrqPin);

pub fn init(pin: AnyIOPin<'static>) -> Ptt {
    Ptt(IrqPin::new(pin.into(), Pull::Up))
}

impl open_oswst_core::devices::ptt::Ptt for Ptt {
    async fn pressed(&mut self) {
        loop {
            let _ = self.0.wait_for_low().await;
            // Still low after STEADY_MS: a press. Back high first: a glitch
            match select(self.0.wait_for_high(), Timer::after_millis(STEADY_MS)).await {
                Either::First(_) => continue,
                Either::Second(()) => return,
            }
        }
    }

    async fn released(&mut self) {
        loop {
            let _ = self.0.wait_for_high().await;
            // Still high after STEADY_MS: a release. Back low first: a glitch
            match select(self.0.wait_for_low(), Timer::after_millis(STEADY_MS)).await {
                Either::First(_) => continue,
                Either::Second(()) => return,
            }
        }
    }

    fn is_pressed(&self) -> bool {
        self.0.is_low()
    }
}
