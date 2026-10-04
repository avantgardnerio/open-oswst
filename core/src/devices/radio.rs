//! The radio, as two queues: packets heard, and packets to send. The driver
//! (SX1262 on the board, the simulated air on the desktop) owns both ends
//! that face the air. Plus how to listen (LISTEN), for a radio that sweeps.

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;

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

/// How the app wants the radio to listen. Only a radio that sweeps channels
/// (the sweep flag) acts on it; one on a single channel ignores it.
#[derive(Clone, Copy, Debug)]
pub enum Listen {
    /// Stay with the transmission: we're sending, or about to. Today that's
    /// the start slot; once we hop, it's carrying on hopping this sequence
    Hold,
    /// The air has gone quiet: sweep the channels for the next transmission
    Sweep,
}

// Static, ISR-safe
pub static RX_CHAN: Channel<CriticalSectionRawMutex, RxPacket, 2> = Channel::new();
pub static TX_CHAN: Channel<CriticalSectionRawMutex, TxRequest, 4> = Channel::new();
/// The latest Listen wins
pub static LISTEN: Signal<CriticalSectionRawMutex, Listen> = Signal::new();
