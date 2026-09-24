//! Rotary encoder with push switch. Hands out turn and click events; knows
//! nothing about what they mean.
//!
//! `next()` is cancel-safe: all state lives in the struct, and each call first
//! catches up on anything that changed while nobody was awaiting it, so it can
//! sit in a `select` that drops it.

use embassy_futures::select::{select, select3, Either};
use embassy_time::Timer;
use esp_idf_svc::hal::gpio::{AnyIOPin, Input, PinDriver, Pull};

/// Switch must hold steady this long to count as a press or release
const DEBOUNCE_MS: u64 = 15;

/// Quadrature transitions per detent (one "click" of the knob)
const STEPS_PER_DETENT: i8 = 4;

pub struct Peripherals {
    pub a: AnyIOPin<'static>,
    pub b: AnyIOPin<'static>,
    pub sw: AnyIOPin<'static>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Cw,
    Ccw,
    Click,
}

pub struct Encoder {
    a: PinDriver<'static, Input>,
    b: PinDriver<'static, Input>,
    sw: PinDriver<'static, Input>,
    phase: u8,     // gray-code position 0..4
    steps: i8,     // transitions accumulated toward the next detent
    pressed: bool, // debounced switch state
}

/// Configure the pins. Common ties to GND, so all three use internal pull-ups.
pub fn init(p: Peripherals) -> Encoder {
    let a = PinDriver::input(p.a, Pull::Up).unwrap();
    let b = PinDriver::input(p.b, Pull::Up).unwrap();
    let sw = PinDriver::input(p.sw, Pull::Up).unwrap();
    let phase = gray_phase(a.is_high(), b.is_high());
    let pressed = sw.is_low();
    Encoder {
        a,
        b,
        sw,
        phase,
        steps: 0,
        pressed,
    }
}

impl Encoder {
    /// Wait for the next turn (one detent) or click.
    pub async fn next(&mut self) -> Event {
        loop {
            if let Some(event) = self.update_quadrature() {
                return event;
            }
            if self.sw.is_low() != self.pressed {
                if let Some(event) = self.debounce_switch().await {
                    return event;
                }
                continue;
            }
            let _ = select3(
                self.a.wait_for_any_edge(),
                self.b.wait_for_any_edge(),
                self.sw.wait_for_any_edge(),
            )
            .await;
        }
    }

    /// Advance the gray-code state machine; emit an event per full detent.
    fn update_quadrature(&mut self) -> Option<Event> {
        let phase = gray_phase(self.a.is_high(), self.b.is_high());
        match (phase + 4 - self.phase) % 4 {
            1 => self.steps += 1,
            3 => self.steps -= 1,
            _ => {} // no change, or skipped a state (bounce) — ignore
        }
        self.phase = phase;
        if self.steps >= STEPS_PER_DETENT {
            self.steps -= STEPS_PER_DETENT;
            Some(Event::Cw)
        } else if self.steps <= -STEPS_PER_DETENT {
            self.steps += STEPS_PER_DETENT;
            Some(Event::Ccw)
        } else {
            None
        }
    }

    /// Wait until the switch holds steady, then report a click on a new press.
    async fn debounce_switch(&mut self) -> Option<Event> {
        // Each edge restarts the stability window
        while let Either::First(_) = select(
            self.sw.wait_for_any_edge(),
            Timer::after_millis(DEBOUNCE_MS),
        )
        .await
        {}
        let pressed = self.sw.is_low();
        if pressed == self.pressed {
            return None;
        }
        self.pressed = pressed;
        pressed.then_some(Event::Click)
    }
}

/// Position in the CW sequence 00 → 10 → 11 → 01.
fn gray_phase(a: bool, b: bool) -> u8 {
    match (a, b) {
        (false, false) => 0,
        (true, false) => 1,
        (true, true) => 2,
        (false, true) => 3,
    }
}
