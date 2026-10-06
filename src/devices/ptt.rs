//! The push-to-talk button: GPIO0 (the PRG button), active LOW with the
//! internal pull-up.

use embedded_hal_async::digital::Wait;
use esp_idf_svc::hal::gpio::{AnyIOPin, Pull};

use super::irq_pin::IrqPin;

pub struct Ptt(IrqPin);

pub fn init(pin: AnyIOPin<'static>) -> Ptt {
    Ptt(IrqPin::new(pin.into(), Pull::Up))
}

impl open_oswst_core::devices::ptt::Ptt for Ptt {
    async fn pressed(&mut self) {
        let _ = self.0.wait_for_low().await;
    }

    async fn released(&mut self) {
        let _ = self.0.wait_for_high().await;
    }

    fn is_pressed(&self) -> bool {
        self.0.is_low()
    }
}
