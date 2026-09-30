//! Operating mode: what the radio does with what it hears. Saved to NVS as
//! `config/mode`, so it survives a reboot.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Play what we hear
    Normal = 0,
    /// Relay what we hear, straight away
    Repeater = 1,
    /// Record what we hear, then play it back over the air
    Echo = 2,
}

impl Mode {
    pub fn from_u8(value: u8) -> Mode {
        match value {
            1 => Mode::Repeater,
            2 => Mode::Echo,
            _ => Mode::Normal,
        }
    }
}

static MODE: AtomicU8 = AtomicU8::new(Mode::Normal as u8);

pub fn get() -> Mode {
    Mode::from_u8(MODE.load(Ordering::Relaxed))
}

pub fn set(mode: Mode) {
    MODE.store(mode as u8, Ordering::Relaxed);
}
