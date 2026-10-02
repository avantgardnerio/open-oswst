//! Operating mode: what the radio does with what it hears. Saved by name
//! (`mode = "repeater"` in the config file), so it survives a reboot.

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

    /// As saved in the config file
    pub fn name(self) -> &'static str {
        match self {
            Mode::Normal => "normal",
            Mode::Repeater => "repeater",
            Mode::Echo => "echo",
        }
    }

    pub fn from_name(name: &str) -> Option<Mode> {
        match name {
            "normal" => Some(Mode::Normal),
            "repeater" => Some(Mode::Repeater),
            "echo" => Some(Mode::Echo),
            _ => None,
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
    let saved = settings.and_then(|settings| settings.get("mode"));
    let mode = match saved.as_deref().map(Mode::from_name) {
        Some(Some(mode)) => mode,
        Some(None) => {
            log::warn!("Config: unknown mode {:?}, using normal", saved);
            Mode::Normal
        }
        None => Mode::Normal,
    };
    set(mode);
    mode
}
