//! WiFi scans, for the menu's list of networks to add. The menu raises
//! SCAN_WANTED while that list is on the screen; the firmware's network code
//! scans back to back while it's up and sends each scan's networks on SCANS.
//! Nothing is scanned for the menu at any other time.

use core::sync::atomic::AtomicBool;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;

/// One network a scan saw
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    pub ssid: heapless::String<32>,
    pub rssi_dbm: i8,
    pub open: bool, // no password
}

/// Up while the menu wants scans
pub static SCAN_WANTED: AtomicBool = AtomicBool::new(false);
/// The latest scan wins
pub static SCANS: Signal<CriticalSectionRawMutex, Vec<Seen>> = Signal::new();
