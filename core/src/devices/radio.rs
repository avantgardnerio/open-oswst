//! The radio, as two queues: packets heard, and packets to send. The driver
//! (SX1262 on the board, the simulated air on the desktop) owns both ends
//! that face the air.

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;

pub struct RxPacket {
    pub data: heapless::Vec<u8, 255>,
    pub rssi: i16,
    pub snr: i16,
}

pub struct TxRequest {
    pub data: heapless::Vec<u8, 255>,
    /// A preamble other than the radio's usual, in symbols
    pub preamble: Option<u16>,
}

// Static, ISR-safe
pub static RX_CHAN: Channel<CriticalSectionRawMutex, RxPacket, 2> = Channel::new();
pub static TX_CHAN: Channel<CriticalSectionRawMutex, TxRequest, 4> = Channel::new();
