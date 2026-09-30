//! The push-to-talk button: GPIO0 (the PRG button), active LOW with the
//! internal pull-up.

use esp_idf_svc::hal::gpio::{AnyIOPin, Input, PinDriver, Pull};

pub struct Ptt(PinDriver<'static, Input>);

pub fn init(pin: AnyIOPin<'static>) -> Ptt {
    Ptt(PinDriver::input(pin, Pull::Up).unwrap())
}

impl open_oswst_core::devices::ptt::Ptt for Ptt {
    async fn pressed(&mut self) {
        let _ = self.0.wait_for_low().await;
    }

    fn is_pressed(&self) -> bool {
        self.0.is_low()
    }
}
