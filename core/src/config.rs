//! Every setting in config.toml, declared once here. A setting is a global
//! atomic, so any thread reads it (`config::MODE.get()`) with nothing passed
//! around. `load()` fills them all from the file at boot.
//!
//! Settings sit at the top of the file. Flags are settings too, in its
//! `[flags]` table: experiments, added and dropped as they come and go.
//!
//! ```toml
//! mode = "echo"
//!
//! [flags]
//! wakeup_preamble = true
//! ```
//!
//! To add one: a `static` below, and its name in SETTINGS or FLAGS. To drop
//! one: delete both. A key left in a radio's file is logged and ignored.
//!
//! Each setting says, beyond its type and default:
//! - when a change applies: at boot (the value is fixed until a reboot, so
//!   anything set up from it stays consistent), or live (read at each use)
//! - whether a change is saved to the file, or lasts until power-off
//! - whether radios must agree on it to hear each other. Those make up the
//!   profile code (`profile()`): radios with different codes can't talk

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Mutex;

use crate::devices::settings::Settings;
use crate::mode;

// The settings

/// What the radio does with what it hears: a mode::Mode
pub static MODE: Setting = Setting::choice("mode", &mode::NAMES, 0).live();

/// WiFi on at boot (when networks are configured). /api/wifi/off turns it
/// off until a reboot, and that's never saved: with the menus gone, a reboot
/// must always bring WiFi back, the only way left to reach the radio
pub static WIFI_ON: Setting = Setting::bool("wifi_on", true).live().unsaved();

/// The channel to start on: a slot round the band (air::slots). 103 = 915
/// MHz. The last slot is 206 at 125 kHz (a test keeps this in step)
pub static START_SLOT: Setting = Setting::number("start_slot", 0, 206, 103).must_match();

/// Seeds the order of the hop channels (air::hop_slots)
pub static HOP_SEED: Setting = Setting::number("hop_seed", 0, i32::MAX, 0).must_match();

/// How many channels a receiver sweeps (air::hop_slots, the start slot
/// first). With the sweep flag, a repeater relays each packet on the next of
/// these after the one it heard it on, then listens where it was
pub static RX_HOPS: Setting = Setting::number("rx_hops", 1, 207, 1).must_match();

/// How many channels a transmitter hops over. 0: it stays on its channel.
/// Only 0 is allowed yet: transmitters don't hop
pub static TX_HOPS: Setting = Setting::number("tx_hops", 0, 0, 0).must_match();

pub static SETTINGS: &[&Setting] = &[&MODE, &WIFI_ON, &START_SLOT, &HOP_SEED, &RX_HOPS, &TX_HOPS];

/// This radio's friendly name, sent in every end packet (packet::Ident)
/// and shown on the screens. None set: the short MAC
pub static NAME: Text = Text::new("name");

pub static TEXTS: &[&Text] = &[&NAME];

// The flags

/// Before each transmission we start (talking, or an echo replay), send a
/// wake-up packet: a header with a preamble as long as a whole voice packet
/// takes (air::wake_preamble_symbols), for radios that will sweep the hop
/// channels to find. Sender only: whoever hears one logs it and drops it
pub static WAKE_PREAMBLE: Setting = Setting::bool("wake_preamble", false).live();

/// Listen by sweeping the hop channels (hop_slots of start_slot, hop_seed,
/// rx_hops) with CAD while idle, instead of sitting on the start slot. A
/// hit locks onto that channel until it goes quiet. A repeater relays on the
/// next channel (rx_hops). We still start transmissions on the start slot
pub static SWEEP: Setting = Setting::bool("sweep", false);

/// Assisted GPS (agnss.rs): at boot, tell the GPS our last fix and send it
/// the kept satellite orbits; on joining WiFi, where the network is (its
/// lat/lon in config.toml), the NTP time, and freshly downloaded orbits
pub static AGNSS: Setting = Setting::bool("agnss", false);

/// Testing only: at boot, make the GPS forget everything (cold start)
/// before any assisted GPS, to time how long it takes to find a fix
pub static GPS_COLD_START: Setting = Setting::bool("gps_cold_start", false);

pub static FLAGS: &[&Setting] = &[&WAKE_PREAMBLE, &SWEEP, &AGNSS, &GPS_COLD_START];

/// A setting that's text: up to packet::NAME_BYTES of UTF-8, read at boot
/// (a change applies after a reboot)
pub struct Text {
    pub name: &'static str,
    value: Mutex<heapless::String<{ crate::packet::NAME_BYTES }>>,
}

impl Text {
    const fn new(name: &'static str) -> Text {
        Text {
            name,
            value: Mutex::new(heapless::String::new()),
        }
    }

    /// Empty if the file has none
    pub fn get(&self) -> heapless::String<{ crate::packet::NAME_BYTES }> {
        self.value.lock().unwrap().clone()
    }

    fn parse(&self, text: &str) -> Result<heapless::String<{ crate::packet::NAME_BYTES }>, String> {
        text.try_into().map_err(|_| {
            format!(
                "{} must be at most {} bytes",
                self.name,
                crate::packet::NAME_BYTES
            )
        })
    }
}

/// One setting: its declaration and its current value
pub struct Setting {
    pub name: &'static str,
    pub kind: Kind,
    default: i32,
    pub applies: Applies,
    pub saved: bool,
    pub must_match: bool,
    value: AtomicI32,
}

pub enum Kind {
    /// 0 or 1
    Bool,
    /// A whole number in min..=max
    Number { min: i32, max: i32 },
    /// The index of one of these names
    Choice(&'static [&'static str]),
}

#[derive(PartialEq)]
pub enum Applies {
    Boot,
    Live,
}

impl Setting {
    /// By default a setting applies at boot, a change is saved, and radios
    /// needn't agree on it. The `.live()` etc. below change that.
    const fn new(name: &'static str, kind: Kind, default: i32) -> Setting {
        Setting {
            name,
            kind,
            default,
            applies: Applies::Boot,
            saved: true,
            must_match: false,
            value: AtomicI32::new(default),
        }
    }

    pub const fn bool(name: &'static str, default: bool) -> Setting {
        Setting::new(name, Kind::Bool, default as i32)
    }

    pub const fn number(name: &'static str, min: i32, max: i32, default: i32) -> Setting {
        Setting::new(name, Kind::Number { min, max }, default)
    }

    pub const fn choice(
        name: &'static str,
        names: &'static [&'static str],
        default: i32,
    ) -> Setting {
        Setting::new(name, Kind::Choice(names), default)
    }

    pub const fn live(self) -> Setting {
        Setting {
            applies: Applies::Live,
            ..self
        }
    }

    pub const fn unsaved(self) -> Setting {
        Setting {
            saved: false,
            ..self
        }
    }

    pub const fn must_match(self) -> Setting {
        Setting {
            must_match: true,
            ..self
        }
    }

    pub fn get(&self) -> i32 {
        self.value.load(Ordering::Relaxed)
    }

    pub fn is_on(&self) -> bool {
        self.get() != 0
    }

    /// Change it. A boot setting keeps its value until the reboot; a saved
    /// one goes to the file (if there is one).
    pub fn set(&self, value: i32, file: Option<&mut impl Settings>) {
        if self.applies == Applies::Live {
            self.value.store(value, Ordering::Relaxed);
        }
        let text = self.text(value);
        match (self.saved, file) {
            (true, Some(file)) => file.set(&self.key(), &text),
            (true, None) => {
                log::warn!("Config: {} = {} not saved, no config file", self.name, text)
            }
            (false, _) => {}
        }
        match self.applies {
            Applies::Live => log::info!("Config: {} = {}", self.name, text),
            Applies::Boot => log::info!("Config: {} = {} after a reboot", self.name, text),
        }
    }

    /// Its key in the file: flags are in [flags]
    fn key(&self) -> String {
        if FLAGS.iter().any(|flag| std::ptr::eq(*flag, self)) {
            format!("flags.{}", self.name)
        } else {
            self.name.to_string()
        }
    }

    /// A value as the file has it
    fn text(&self, value: i32) -> String {
        match self.kind {
            Kind::Bool => (value != 0).to_string(),
            Kind::Number { .. } => value.to_string(),
            Kind::Choice(names) => names.get(value as usize).unwrap_or(&"?").to_string(),
        }
    }

    /// A value from the file, if it's one this setting can take
    fn parse(&self, text: &str) -> Result<i32, String> {
        match self.kind {
            Kind::Bool => match text {
                "true" => Ok(1),
                "false" => Ok(0),
                _ => Err(format!("{} must be true or false", self.name)),
            },
            Kind::Number { min, max } => match text.parse() {
                Ok(n) if (min..=max).contains(&n) => Ok(n),
                _ => Err(format!("{} must be a number, {}..={}", self.name, min, max)),
            },
            Kind::Choice(names) => match names.iter().position(|name| *name == text) {
                Some(i) => Ok(i as i32),
                None => Err(format!("{} must be one of {}", self.name, names.join(", "))),
            },
        }
    }
}

fn all() -> impl Iterator<Item = &'static Setting> {
    SETTINGS.iter().chain(FLAGS.iter()).copied()
}

/// Set every setting from the file (no file: all defaults). Logs them all,
/// so each boot's log says what it ran with, and any key in the file we
/// don't know (a dropped flag, a typo). Those stay in the file.
pub fn load(file: Option<&impl Settings>) {
    for setting in all() {
        let text = file.and_then(|file| file.get(&setting.key()));
        let value = match text.as_deref().map(|text| setting.parse(text)) {
            Some(Ok(value)) => value,
            Some(Err(e)) => {
                log::warn!("Config: {}, using {}", e, setting.text(setting.default));
                setting.default
            }
            None => setting.default,
        };
        setting.value.store(value, Ordering::Relaxed);
    }
    for text in TEXTS {
        let value = match file.and_then(|file| file.get(text.name)) {
            Some(value) => text.parse(&value).unwrap_or_else(|e| {
                log::warn!("Config: {}, using none", e);
                heapless::String::new()
            }),
            None => heapless::String::new(),
        };
        *text.value.lock().unwrap() = value;
    }
    if let Some(file) = file {
        for key in file.keys() {
            let known = all().any(|setting| setting.key() == key)
                || TEXTS.iter().any(|text| text.name == key);
            if !known {
                log::warn!("Config: {} is no setting of this firmware, ignored", key);
            }
        }
    }
    log::info!(
        "Config: {} name={:?}",
        summary(SETTINGS),
        NAME.get().as_str()
    );
    log::info!("Config flags: {}", summary(FLAGS));
    log::info!("Config profile: {}", profile());
}

/// Can this firmware use this file? Every setting in it must have a value it
/// can take. A file that fails is refused, and the old one stays.
pub fn check(file: &impl Settings) -> Result<(), String> {
    for setting in all() {
        if let Some(text) = file.get(&setting.key()) {
            setting.parse(&text)?;
        }
    }
    for text in TEXTS {
        if let Some(value) = file.get(text.name) {
            text.parse(&value)?;
        }
    }
    Ok(())
}

/// e.g. "mode=echo wifi_on=true"
fn summary(settings: &[&Setting]) -> String {
    let pairs: Vec<String> = settings
        .iter()
        .map(|setting| format!("{}={}", setting.name, setting.text(setting.get())))
        .collect();
    if pairs.is_empty() {
        "none".to_string()
    } else {
        pairs.join(" ")
    }
}

/// 4 hex digits from every must-match setting's name and value: two radios
/// with different codes can't hear each other. FNV-1a, folded to 16 bits.
pub fn profile() -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for setting in all().filter(|setting| setting.must_match) {
        let value = setting.get().to_le_bytes();
        for byte in setting.name.bytes().chain(value) {
            hash = (hash ^ byte as u32).wrapping_mul(0x0100_0193);
        }
    }
    format!("{:04X}", (hash >> 16) ^ (hash & 0xFFFF))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A config file in memory
    #[derive(Default)]
    struct File(HashMap<String, String>);

    impl Settings for File {
        fn get(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set(&mut self, key: &str, value: &str) {
            self.0.insert(key.into(), value.into());
        }
        fn keys(&self) -> Vec<String> {
            self.0.keys().cloned().collect()
        }
    }

    fn file(pairs: &[(&str, &str)]) -> File {
        File(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    #[test]
    fn a_name_loads_and_a_too_long_one_is_refused() {
        assert!(check(&file(&[("name", "eighteen-byte-name")])).is_ok());
        assert!(check(&file(&[("name", "nineteen-bytes-name")])).is_err());
        load(Some(&file(&[("name", "Cornelious")])));
        assert_eq!(NAME.get(), "Cornelious");
    }

    #[test]
    fn values_parse_by_kind() {
        let on = Setting::bool("on", false);
        assert_eq!(on.parse("true"), Ok(1));
        assert!(on.parse("yes").is_err());
        let n = Setting::number("n", 1, 10, 5);
        assert_eq!(n.parse("10"), Ok(10));
        assert!(n.parse("11").is_err());
        assert_eq!(MODE.parse("echo"), Ok(2));
        assert!(MODE.parse("loud").is_err());
    }

    #[test]
    fn check_refuses_a_bad_value_and_ignores_unknown_keys() {
        assert!(check(&file(&[("mode", "repeater"), ("gone", "1")])).is_ok());
        assert!(check(&file(&[("mode", "loud")])).is_err());
    }

    #[test]
    fn unsaved_and_boot_settings() {
        let mut saved = File::default();
        // Live, unsaved: changes now, the file never hears of it
        let wifi = Setting::bool("wifi_on", true).live().unsaved();
        wifi.set(0, Some(&mut saved));
        assert!(!wifi.is_on());
        assert!(saved.0.is_empty());
        // Boot: saved, but the value holds until a reboot
        let boot = Setting::number("n", 0, 99, 5);
        boot.set(7, Some(&mut saved));
        assert_eq!(boot.get(), 5);
        assert_eq!(saved.get("n").as_deref(), Some("7"));
    }

    #[test]
    fn slot_and_hop_ranges_match_the_band() {
        let Kind::Number { max, .. } = START_SLOT.kind else {
            panic!("start_slot is a number");
        };
        assert_eq!(max as u32, crate::air::slots() - 1);
        let Kind::Number { max, .. } = RX_HOPS.kind else {
            panic!("rx_hops is a number");
        };
        assert_eq!(max as u32, crate::air::slots());
    }

    #[test]
    fn profile_is_four_hex_digits() {
        let code = profile();
        assert_eq!(code.len(), 4);
        assert!(code.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
