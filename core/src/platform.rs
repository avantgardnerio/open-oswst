//! What a place the app runs (the board, the desktop) has to provide. The
//! radio, speaker and screen aren't here: they're channels in `devices`,
//! driven by whoever owns that hardware.

use crate::devices::network::Network;
use crate::devices::{gps::Gps, knob::Knob, mic::Mic, ptt::Ptt, settings::Settings};

pub trait Platform {
    type Mic: Mic;
    type Ptt: Ptt;
    type Knob: Knob;
    type Gps: Gps;
    type Settings: Settings;

    fn random() -> u32;

    /// µs since boot, on the radio's clock: the same as `RxPacket::end_us`
    fn now_us() -> i64;

    /// The WiFi, for the screen
    fn network() -> Network;
}
