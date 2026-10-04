//! Operating mode: what the radio does with what it hears. A setting
//! (config::MODE), saved by name: `mode = "repeater"` in the config file.

use crate::config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Play what we hear
    Normal = 0,
    /// Relay what we hear, straight away
    Repeater = 1,
    /// Record what we hear, then play it back over the air
    Echo = 2,
}

/// As saved in the config file, in Mode's order
pub const NAMES: [&str; 3] = ["normal", "repeater", "echo"];

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
        NAMES[self as usize]
    }
}

/// The current mode: the config's (config.rs)
pub fn get() -> Mode {
    Mode::from_u8(config::MODE.get() as u8)
}
