//! What the radio settings mean on the air: packet air times, and the
//! wake-up packet's preamble that follows from them. Pure math from the LoRa
//! constants below and the codec's packet size.
//!
//! The LoRa constants are the radio's setup (src/devices/radio.rs uses them
//! or must match them). They move into the config as we go.

use crate::codec::{HEADER_BYTES, PACKET_BYTES};

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

/// The wake-up packet's preamble: as long as it can be while a header-only
/// packet still takes no more air than a voice packet, so it fits the
/// relay slot like any packet. 42 at SF7/125k (12 + 48 - 18)
pub fn wake_preamble_symbols() -> u16 {
    let extra = payload_symbols(PACKET_BYTES) - payload_symbols(HEADER_BYTES);
    PREAMBLE_SYMBOLS + extra as u16
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
/// header and a CRC: 8 + ceil((8PL - 4SF + 28 + 16) / 4(SF - 2DE)) × (CR+4)
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
    fn wake_packet_takes_a_voice_packets_air() {
        assert_eq!(wake_preamble_symbols(), 42);
        assert_eq!(
            packet_us(wake_preamble_symbols(), HEADER_BYTES),
            packet_us(PREAMBLE_SYMBOLS, PACKET_BYTES)
        );
    }
}
