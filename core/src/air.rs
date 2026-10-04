//! What the radio settings mean on the air: packet air times, the wake-up
//! packet's preamble that follows from them, and the channels. Pure math
//! from the LoRa constants below, the codec's packet size and the config.
//!
//! The LoRa constants are the radio's setup (src/devices/radio.rs uses them
//! or must match them). They move into the config as we go.

use crate::codec::{FRAMES_PER_PACKET, HEADER_BYTES, PACKET_BYTES};

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
pub const PREAMBLE_SYMBOLS: u16 = 12;

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
    PREAMBLE_SYMBOLS + extra as u16
}

/// One packet's audio, and so the time from one packet to the next: 160ms
pub fn slot_us() -> u32 {
    FRAMES_PER_PACKET as u32 * 40_000
}

/// 2^SF / bandwidth: 1024us at SF7/125k
pub fn symbol_us() -> u32 {
    (1 << SPREADING_FACTOR) * 1000 / BANDWIDTH_KHZ
}

/// A packet's air time (Semtech's formula): the preamble, 4.25 symbols of
/// sync word, then the header and payload. 65.8ms for a voice packet
pub fn packet_us(preamble_symbols: u16, bytes: usize) -> u32 {
    // In quarter symbols, so the 4.25 stays whole
    let quarters = preamble_symbols as u32 * 4 + 17 + payload_symbols(bytes) * 4;
    quarters * symbol_us() / 4
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
    8 + blocks as u32 * (CODING_RATE + 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_packet_is_65_8ms() {
        assert_eq!(symbol_us(), 1_024);
        assert_eq!(packet_us(PREAMBLE_SYMBOLS, PACKET_BYTES), 65_792);
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
    fn wake_packet_takes_a_voice_packets_air() {
        assert_eq!(wake_preamble_symbols(), 42);
        assert_eq!(
            packet_us(wake_preamble_symbols(), HEADER_BYTES),
            packet_us(PREAMBLE_SYMBOLS, PACKET_BYTES)
        );
    }
}
