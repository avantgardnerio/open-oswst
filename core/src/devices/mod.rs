//! Device interfaces. Radio, speaker and screen are channels (whoever drives
//! the hardware feeds and drains them); the rest are small traits.

pub mod gps;
pub mod knob;
pub mod mic;
pub mod ptt;
pub mod radio;
pub mod screen;
pub mod settings;
pub mod speaker;
