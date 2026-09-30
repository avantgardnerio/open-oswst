//! Operating mode: what the radio does with what it hears. Saved to NVS as
//! `config/mode`, so it survives a reboot.

use std::sync::atomic::{AtomicU8, Ordering};

use crate::devices::settings::Settings;

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

/// Read the saved mode (Normal if there's none, or no settings at all), and
/// make it current.
pub fn load(settings: Option<&impl Settings>) -> Mode {
    let read = |key| settings.and_then(|settings| settings.get_u8(key));
    // "mode" replaced an older "repeater" on/off flag; boards saved before
    // then only have that one
    let mode = match (read("mode"), read("repeater")) {
        (Some(mode), _) => Mode::from_u8(mode),
        (None, Some(1)) => Mode::Repeater,
        _ => Mode::Normal,
    };
    set(mode);
    mode
}
