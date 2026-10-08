//! The management server, as the app reaches it: the app puts a request on
//! REQUESTS, whoever has the network (the firmware's net thread) sends it,
//! and the answer comes back on ANSWERS. The protocol is management.rs.

use crate::management::{Request, Response};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;

/// One at a time: the app waits for each answer
pub static REQUESTS: Channel<CriticalSectionRawMutex, Request, 1> = Channel::new();
/// The server's response, or why there's none
pub static ANSWERS: Signal<CriticalSectionRawMutex, Result<Response, String>> = Signal::new();
