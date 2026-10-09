//! The management server, as the app reaches it: the app puts a job on
//! JOBS, whoever has the network (the firmware's net thread) does it, and
//! the outcome comes back on ANSWERS (a check) or INSTALL (an install).
//! The protocol is management.rs.

use crate::management::{Offer, Response};
use core::sync::atomic::AtomicBool;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;

pub enum Job {
    /// Is there another update bundle for us? The answer: UpToDate,
    /// BundleOffer or Error
    Check,
    /// Install this bundle (bundle.rs): download, verify, extract, then
    /// reboot into its firmware
    Install(Offer),
}

/// How an install is going: its three steps, each in percent
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Installing {
    /// To the storage, from the management server
    Downloading(u8),
    /// Its SHA-256, against the offer's
    Verifying(u8),
    /// The firmware into the spare app slot, the files onto the storage
    Extracting(u8),
    /// Not installed (the running firmware stays); few words, for the screen
    Failed(String),
    /// Installed: the radio reboots into it in a moment
    Done,
}

/// One at a time: the app waits for each outcome
pub static JOBS: Channel<CriticalSectionRawMutex, Job, 1> = Channel::new();
/// A check's answer from the server, or why there's none
pub static ANSWERS: Signal<CriticalSectionRawMutex, Result<Response, String>> = Signal::new();
/// The latest news of an install
pub static INSTALL: Signal<CriticalSectionRawMutex, Installing> = Signal::new();
/// Raised to give up an install (the menu's click); it stops at the next chunk
pub static CANCEL: AtomicBool = AtomicBool::new(false);
