//! Everything that runs the same on the board and on the desktop: the radio
//! app, its protocol and codec, menus and logging. Hardware stays behind the
//! interfaces in `devices`, which the firmware and the desktop implement.

pub mod app;
pub mod codec;
pub mod devices;
pub mod echo;
pub mod fec;
pub mod logger;
pub mod menu;
pub mod mode;
pub mod packet;
pub mod platform;
pub mod playback_timing;
pub mod rx_buffer;
