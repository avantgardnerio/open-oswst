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
    /// False: `end_us` can't be trusted. Either the time from its header to
    /// its end wasn't a packet of its length's (air::after_header_us): it was
    /// decoded off its channel (at point-blank range a radio hears the next
    /// channel too), and in LoRa a frequency offset looks like a time offset.
    /// Or the radio task got to its IRQ too late to know the stamp was its
    /// own (a flash write froze it). Its bytes are as good as `crc_ok` says
    pub timing_ok: bool,
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
    /// When to start sending, in µs since boot (esp_timer: the clock of
    /// `RxPacket::end_us`); None: as soon as it can. Senders put each packet
    /// in the middle of its bin (air::guard_us)
    pub send_at_us: Option<i64>,
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
    /// Listen on a schedule of channels, until told otherwise
    Rotation(Rotation),
}

/// A schedule of channels that repeats on a clock: turns of `every_us`,
/// from `from_us` on, turn n on `channels[n % 4]`. Just times and channels:
/// the app works out which (a transmission's bins and which hops are on
/// which channel, conveyor.rs). `from_us` may be long past: the rotation
/// has been going round since then
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rotation {
    /// When turn 0 started, in µs since boot (esp_timer, like
    /// `RxPacket::end_us`)
    pub from_us: i64,
    /// How long each turn lasts, µs
    pub every_us: u32,
    /// The channel for each turn, round and round: an index into the hop
    /// channels, like `RxPacket::channel`
    pub channels: [u8; 4],
}

impl Rotation {
    /// The channel to be on at `at_us`
    pub fn channel_at(&self, at_us: i64) -> u8 {
        let turn = (at_us - self.from_us).div_euclid(self.every_us as i64);
        self.channels[turn.rem_euclid(self.channels.len() as i64) as usize]
    }

    /// When the turn after the one at `at_us` starts
    pub fn next_turn_us(&self, at_us: i64) -> i64 {
        let every_us = self.every_us as i64;
        let turn = (at_us - self.from_us).div_euclid(every_us);
        self.from_us + (turn + 1) * every_us
    }

    /// Every turn on the same channel: it never moves
    pub fn stays(&self) -> bool {
        self.channels
            .iter()
            .all(|&channel| channel == self.channels[0])
    }
}

// Static, ISR-safe
pub static RX_CHAN: Channel<CriticalSectionRawMutex, RxPacket, 2> = Channel::new();
pub static TX_CHAN: Channel<CriticalSectionRawMutex, TxRequest, 4> = Channel::new();
/// The latest Listen wins
pub static LISTEN: Signal<CriticalSectionRawMutex, Listen> = Signal::new();

#[cfg(test)]
mod tests {
    use super::*;

    const ROTATION: Rotation = Rotation {
        from_us: 1_000_000,
        every_us: 80_000,
        channels: [3, 4, 3, 2],
    };

    #[test]
    fn a_rotation_takes_turns_on_its_channels() {
        assert_eq!(ROTATION.channel_at(1_000_000), 3);
        assert_eq!(ROTATION.channel_at(1_079_999), 3);
        assert_eq!(ROTATION.channel_at(1_080_000), 4);
        assert_eq!(ROTATION.channel_at(1_160_000), 3);
        assert_eq!(ROTATION.channel_at(1_240_000), 2);
        assert_eq!(ROTATION.channel_at(1_320_000), 3);
        // Long after it started, and before
        assert_eq!(ROTATION.channel_at(1_000_000 + 1001 * 80_000 + 5), 4);
        assert_eq!(ROTATION.channel_at(999_999), 2);
    }

    #[test]
    fn the_next_turn_is_at_the_next_edge() {
        assert_eq!(ROTATION.next_turn_us(1_000_000), 1_080_000);
        assert_eq!(ROTATION.next_turn_us(1_079_999), 1_080_000);
        assert_eq!(ROTATION.next_turn_us(1_080_000), 1_160_000);
        assert_eq!(ROTATION.next_turn_us(999_999), 1_000_000);
    }
}
