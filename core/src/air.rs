//! What the radio settings mean on the air: packet air times, the wake-up
//! packet's preamble that follows from them, and the channels. Pure math
//! from the LoRa constants below, the codec's packet size and the config.
//!
//! The LoRa constants are the radio's setup (src/devices/radio.rs uses them
//! or must match them). They move into the config as we go.

use crate::codec::{FRAMES_PER_PACKET, HEADER_BYTES, PACKET_BYTES};
use crate::config;

/// LoRa spreading factor. radio.rs sets SpreadingFactor::_7: keep in step
pub const SPREADING_FACTOR: u32 = 7;
/// LoRa bandwidth, kHz. radio.rs sets Bandwidth::_125KHz: keep in step
pub const BANDWIDTH_KHZ: u32 = 125;
/// LoRa coding rate 4/(4+this). radio.rs sets CodingRate::_4_5: keep in step
pub const CODING_RATE: u32 = 1;

/// Preamble length in symbols, TX and RX alike (every radio must agree). Was
/// 8. The 2026-10-03 walk lost its return packets to missed detections, not
/// corruption, with interference setting off the detector between real
/// packets. A longer preamble gives the detector more to lock onto, and a
/// second chance after a false alarm. 12, not 16: at 16 a packet (69.9ms)
/// plus a repeater's relay of it overran the 160ms slot on the desk, and
/// playback underran. 12 costs 4 symbols = 4.1ms per packet at SF7/125k
/// (61.7 -> 65.8ms) and leaves a single repeater ~17ms of slack.
/// The setting config::PREAMBLE_SYMBOLS (default 12) says which: 8 runs the
/// walk-5 baseline again
pub fn preamble_symbols() -> u16 {
    config::PREAMBLE_SYMBOLS.get() as u16
}

/// The sync word after the preamble: 4.25 symbols, in quarter symbols so it
/// stays whole. Fixed by LoRa, at any SF or bandwidth
const SYNC_WORD_QUARTERS: u32 = 17;
/// LoRa's explicit header: the first block after the sync word, always 8
/// symbols (sent at 4/8 whatever the coding rate)
const HEADER_SYMBOLS: u32 = 8;

/// The band we may use: 902-928 MHz (FCC 15.247)
const BAND_LOW_HZ: u32 = 902_000_000;
const BAND_HIGH_HZ: u32 = 928_000_000;

/// The band as a ring of channel slots, one bandwidth apart (the FCC's
/// minimum spacing for a hop channel). Slot 0 sits one bandwidth above the
/// band's bottom edge: every channel stays inside the band, and 915 MHz
/// lands on the grid (slot 103 at 125 kHz). 207 slots at 125 kHz
pub fn slots() -> u32 {
    let top_centre = BAND_HIGH_HZ - step_hz() / 2;
    (top_centre - first_slot_hz()) / step_hz() + 1
}

/// The centre frequency of `slot` (taken round the ring)
pub fn slot_hz(slot: u32) -> u32 {
    first_slot_hz() + (slot % slots()) * step_hz()
}

fn step_hz() -> u32 {
    BANDWIDTH_KHZ * 1000
}

fn first_slot_hz() -> u32 {
    BAND_LOW_HZ + step_hz()
}

/// The slots to hop over, in order: `start` first, then `count - 1` more,
/// each a different offset from it round the ring, shuffled by `seed`. Each
/// slot is used once, so hearing a channel tells a radio where in the
/// sequence it is. Every radio with the same three gets the same list.
pub fn hop_slots(start: u32, seed: u64, count: u32) -> Vec<u32> {
    let slots = slots();
    // Offsets 1.. from the start, shuffled (Fisher-Yates) as far as needed
    let mut offsets: Vec<u32> = (1..slots).collect();
    let mut random = SplitMix64(seed);
    let picks = (count.saturating_sub(1) as usize).min(offsets.len());
    for i in 0..picks {
        let j = i + (random.next() % (offsets.len() - i) as u64) as usize;
        offsets.swap(i, j);
    }
    std::iter::once(0)
        .chain(offsets[..picks].iter().copied())
        .map(|offset| (start + offset) % slots)
        .collect()
}

/// SplitMix64: tiny and fully specified, so the hop sequence never changes
/// with a library version
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

/// The wake-up packet's preamble: as long as it can be while a header-only
/// packet still takes no more air than a voice packet, so it fits the
/// relay slot like any packet. 42 at SF7/125k (12 + 48 - 18)
pub fn wake_preamble_symbols() -> u16 {
    let extra = payload_symbols(PACKET_BYTES) - payload_symbols(HEADER_BYTES);
    preamble_symbols() + extra as u16
}

/// One packet's audio, and so the time from one packet to the next: 160ms
pub fn slot_us() -> u32 {
    FRAMES_PER_PACKET as u32 * 40_000
}

/// The air as a conveyor belt of bins going by, one every 80 ms (half a
/// packet's audio): a talker sends in the even bins, and a repeater relays
/// each packet in the odd bin after it
pub fn bin_us() -> u32 {
    slot_us() / 2
}

/// The spare air in a bin, either side of its packet: (80 - 65.8) / 2 =
/// 7.1 ms. Senders start a packet this long after its bin starts, so it sits
/// in the middle and a radio switching channels at the bin's edges has this
/// much room either side
pub fn guard_us() -> u32 {
    (bin_us() - packet_us(preamble_symbols(), PACKET_BYTES)) / 2
}

/// 2^SF / bandwidth: 1024us at SF7/125k
pub fn symbol_us() -> u32 {
    (1 << SPREADING_FACTOR) * 1000 / BANDWIDTH_KHZ
}

/// A packet's air time (Semtech's formula): the preamble, 4.25 symbols of
/// sync word, then the header and payload. 65.8ms for a voice packet
pub fn packet_us(preamble_symbols: u16, bytes: usize) -> u32 {
    // In quarter symbols, so the 4.25 stays whole
    let quarters = preamble_symbols as u32 * 4 + SYNC_WORD_QUARTERS + payload_symbols(bytes) * 4;
    quarters * symbol_us() / 4
}

/// From a packet's header (the radio's header-valid IRQ, after the header's
/// 8 symbols) to its end: 40.96 ms for a voice packet, 10.2 ms for a
/// wake-up's 2 bytes. A packet whose header-to-end time is off this was
/// decoded off its channel: in LoRa a frequency offset looks like a time
/// offset, so its end time is skewed
pub fn after_header_us(bytes: usize) -> u32 {
    (payload_symbols(bytes) - HEADER_SYMBOLS) * symbol_us()
}

/// The least time from a packet's preamble IRQ to its header IRQ: the
/// preamble IRQ fires somewhere in the preamble, then come the sync word and
/// the header. 12.5 ms at SF7/125k
pub fn header_after_preamble_us() -> u32 {
    (SYNC_WORD_QUARTERS + HEADER_SYMBOLS * 4) * symbol_us() / 4
}

/// Symbols after the sync word for `bytes` of payload, with an explicit
/// header and a CRC: 8 + ceil((8PL - 4SF + 28 + 16) / 4(SF - 2DE)) × (CR+4).
/// The CRC is ours (crc.rs) with LoRa's off: the same 16 bits on the air
fn payload_symbols(bytes: usize) -> u32 {
    let sf = SPREADING_FACTOR as i32;
    // Low data rate optimisation: on for symbols over 16ms (SF11+ at 125k)
    let de = (symbol_us() > 16_000) as i32;
    let bits = 8 * bytes as i32 - 4 * sf + 28 + 16;
    let per_block = 4 * (sf - 2 * de);
    let blocks = if bits > 0 {
        (bits + per_block - 1) / per_block
    } else {
        0
    };
    HEADER_SYMBOLS + blocks as u32 * (CODING_RATE + 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_packet_is_65_8ms() {
        assert_eq!(symbol_us(), 1_024);
        assert_eq!(packet_us(preamble_symbols(), PACKET_BYTES), 65_792);
    }

    #[test]
    fn header_comes_12_5ms_after_the_preamble_irq_and_41ms_before_the_end() {
        assert_eq!(header_after_preamble_us(), 12_544);
        assert_eq!(after_header_us(PACKET_BYTES), 40_960);
    }

    #[test]
    fn the_band_is_207_slots_with_915_mhz_on_the_grid() {
        assert_eq!(slots(), 207);
        assert_eq!(slot_hz(0), 902_125_000);
        assert_eq!(slot_hz(103), 915_000_000);
        assert_eq!(slot_hz(206), 927_875_000);
        assert_eq!(slot_hz(207), 902_125_000); // round the ring
    }

    #[test]
    fn hop_slots_start_at_start_and_never_repeat() {
        assert_eq!(hop_slots(103, 0, 1), [103]);
        let hops = hop_slots(200, 42, 50);
        assert_eq!(hops.len(), 50);
        assert_eq!(hops[0], 200);
        assert!(hops.iter().all(|&slot| slot < slots()));
        let mut sorted = hops.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 50);
        // Same inputs, same list; another seed, another list
        assert_eq!(hop_slots(200, 42, 50), hops);
        assert_ne!(hop_slots(200, 43, 50), hops);
        // Every slot at most
        assert_eq!(hop_slots(0, 7, 1000).len(), 207);
    }

    #[test]
    fn a_packets_end_comes_a_fixed_time_after_its_header() {
        assert_eq!(after_header_us(PACKET_BYTES), 40_960);
        assert_eq!(after_header_us(HEADER_BYTES), 10_240);
    }

    #[test]
    fn a_packet_sits_in_the_middle_of_its_bin() {
        assert_eq!(bin_us(), 80_000);
        assert_eq!(guard_us(), 7_104);
        assert_eq!(
            2 * guard_us() + packet_us(preamble_symbols(), PACKET_BYTES),
            bin_us()
        );
    }

    #[test]
    fn wake_packet_takes_a_voice_packets_air() {
        assert_eq!(wake_preamble_symbols(), 42);
        assert_eq!(
            packet_us(wake_preamble_symbols(), HEADER_BYTES),
            packet_us(preamble_symbols(), PACKET_BYTES)
        );
    }
}
