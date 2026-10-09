//! Everything that runs the same on the board and on the desktop: the radio
//! app, its protocol and codec, menus and logging. Hardware stays behind the
//! interfaces in `devices`, which the firmware and the desktop implement.

pub mod air;
pub mod app;
pub mod bundle;
pub mod climb;
pub mod codec;
pub mod config;
pub mod conveyor;
pub mod crc;
pub mod devices;
pub mod double_click;
pub mod echo;
pub mod fec;
pub mod gzip;
pub mod logger;
pub mod management;
pub mod menu;
pub mod mode;
pub mod packet;
pub mod password_entry;
pub mod platform;
pub mod playback_timing;
pub mod playout;
pub mod scan_list;
pub mod screen_text;
pub mod tar;
pub mod utc;
