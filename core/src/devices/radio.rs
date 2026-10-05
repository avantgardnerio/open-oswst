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
    /// Where it was heard: an index into the hop channels (air::hop_slots),
    /// 0 = the start slot. Always 0 for a radio that doesn't sweep
    pub channel: u8,
    /// False: our CRC didn't match, so `data` can't be trusted (not even
    /// its txid or hops). A packet was still on the air at `end_us`, so its
    /// timing is real
    pub crc_ok: bool,
    /// When the packet ended on the air: the radio's RX-done IRQ, in µs
    /// since boot, as the DIO1 interrupt noted it (the radio task's own,
    /// later, time if no interrupt fired: DIO1 was already high)
    pub end_us: i64,
}

pub struct TxRequest {
    pub data: heapless::Vec<u8, 255>,
    /// A preamble other than the radio's usual, in symbols
    pub preamble: Option<u16>,
    /// Where to send it: an index into the hop channels, 0 = the start slot.
    /// The radio listens as before afterwards. A radio that doesn't sweep
    /// sends everything on the start slot
    pub channel: u8,
    /// Listen before talk: wait for clear air first. A talker already on the
    /// air sends the rest of its transmission on the beat without it: the
    /// waits put packets up to 65 ms off the beat (walk of 2026-10-05)
    pub clear_air_first: bool,
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
