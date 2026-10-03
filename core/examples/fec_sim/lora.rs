//! A LoRa modem (SF7, CR 4/5) simulated from bytes to symbols and back, with
//! the coding the SX1262 does inside: Hamming code, diagonal interleaver,
//! Gray mapping. That's what decides how a bad symbol turns into bad bits.
//!
//! The chip's insides aren't documented. This follows gr-lora_sdr (Tapparel
//! et al., EPFL), which decodes real Semtech transmissions, and the symbol
//! counts are checked against Semtech's airtime formula below.
//!
//! The packet is a string of nibbles (each byte low nibble first, the
//! explicit header's 5 nibbles in front). They go out in blocks:
//!
//! - The first block is always 8 symbols carrying SF-2 nibbles at CR 4/8,
//!   each symbol using only its top SF-2 bits ("reduced rate"): sturdier, so
//!   the header gets through.
//! - Every later block is 4+CR = 5 symbols carrying SF = 7 nibbles at CR 4/5.
//!
//! Inside a block the interleaver gives every symbol one bit of every
//! codeword. So one bad symbol damages up to 7 nibbles (3.5 bytes) by a bit
//! each. At CR 4/5 the 5th bit is parity only: the chip can't correct with
//! it, so we ignore it on receive (and a bad parity-carrying symbol is free).
//!
//! Not simulated: preamble detection and sync. A packet whose preamble is
//! missed is lost however good its FEC; those losses come on top.

use crate::channel::Channel;

pub const SF: usize = 7;
const CR: usize = 1; // 4/(4+CR) = 4/5
const PREAMBLE_SYMBOLS: f64 = 8.0 + 4.25; // 8 programmed + sync word + SFD

pub struct Format {
    pub explicit_header: bool,
    /// The radio's own CRC-16. Modelled as perfect: any damage = lost packet
    pub crc: bool,
}

/// Send `payload` and return what the receiver hands up: None if the header
/// or the CRC failed, otherwise the bytes, right or wrong
pub fn send(payload: &[u8], format: &Format, channel: &mut Channel) -> Option<Vec<u8>> {
    let sent = nibbles(payload, format);
    let received = through_modem(&sent, channel);

    let header = if format.explicit_header { 5 } else { 0 };
    if received[..header] != sent[..header] {
        return None; // header lost: the chip never reports the packet
    }
    if format.crc && received != sent {
        return None;
    }
    let body = &received[header..header + payload.len() * 2];
    Some(body.chunks(2).map(|n| n[0] | n[1] << 4).collect())
}

/// Symbols after the preamble
pub fn symbols(payload_len: usize, format: &Format) -> usize {
    let nibbles = nibbles(&vec![0; payload_len], format).len();
    let first = SF - 2;
    let later_blocks = nibbles.saturating_sub(first).div_ceil(SF);
    8 + later_blocks * (4 + CR)
}

pub fn airtime_ms(payload_len: usize, format: &Format, bandwidth_hz: f64) -> f64 {
    let symbol_ms = symbol_ms(bandwidth_hz);
    (PREAMBLE_SYMBOLS + symbols(payload_len, format) as f64) * symbol_ms
}

pub fn symbol_ms(bandwidth_hz: f64) -> f64 {
    (1 << SF) as f64 / bandwidth_hz * 1000.0
}

/// The packet as the modem sees it: header, payload, CRC, in nibbles
fn nibbles(payload: &[u8], format: &Format) -> Vec<u8> {
    let mut out = Vec::new();
    if format.explicit_header {
        // Length, coding rate + CRC flag, checksum. What's in it doesn't
        // matter here, only that it arrives intact
        let len = payload.len() as u8;
        out.extend([
            len >> 4,
            len & 0xF,
            (CR as u8) << 1 | format.crc as u8,
            0x5,
            0xA,
        ]);
    }
    // The CRC's value doesn't matter either: the check is modelled as "all
    // these nibbles arrived intact"
    let crc: &[u8] = if format.crc { &[0xC3, 0x3C] } else { &[] };
    for &byte in payload.iter().chain(crc) {
        out.extend([byte & 0xF, byte >> 4]);
    }
    out
}

/// Code, interleave and send every block, and decode what comes back
fn through_modem(sent: &[u8], channel: &mut Channel) -> Vec<u8> {
    let mut received = Vec::with_capacity(sent.len() + SF);
    let mut first_block = true;
    while received.len() < sent.len() {
        // (nibbles in this block, bits per codeword, bits dropped per symbol)
        let (width, codeword_bits, reduced) = if first_block {
            (SF - 2, 8, 2)
        } else {
            (SF, 4 + CR, 0)
        };
        first_block = false;

        let start = received.len();
        let mut codewords = [0u8; SF];
        for (k, cw) in codewords.iter_mut().enumerate().take(width) {
            let nibble = sent.get(start + k).copied().unwrap_or(0); // pad
            *cw = hamming_encode(nibble, codeword_bits);
        }

        // Symbol i carries bit i of every codeword, on a diagonal: its bit j
        // (most significant first) is from codeword (i - j - 1) mod width
        let mut got = [0u8; SF];
        for i in 0..codeword_bits {
            let codeword_bit = codeword_bits - 1 - i;
            let mut bits = 0u32;
            for j in 0..width {
                let k = (i + 2 * width - j - 1) % width;
                bits = bits << 1 | ((codewords[k] >> codeword_bit) & 1) as u32;
            }
            // Gray mapping, so a symbol one bin off costs only one bit
            let bin = from_gray(bits) << reduced;
            let rx_bits = to_gray(channel.demodulate(bin) >> reduced);
            for j in 0..width {
                let k = (i + 2 * width - j - 1) % width;
                let bit = ((rx_bits >> (width - 1 - j)) & 1) as u8;
                got[k] |= bit << codeword_bit;
            }
        }
        for &cw in got.iter().take(width) {
            received.push(hamming_decode(cw, codeword_bits));
        }
    }
    received.truncate(sent.len());
    received
}

/// 4 data bits (first) then the check bits: 1 for CR 4/5, 4 for CR 4/8
fn hamming_encode(nibble: u8, codeword_bits: usize) -> u8 {
    let d = |i: u8| (nibble >> i) & 1;
    if codeword_bits == 5 {
        return nibble << 1 | (nibble.count_ones() as u8 & 1);
    }
    let p0 = d(3) ^ d(2) ^ d(1);
    let p1 = d(2) ^ d(1) ^ d(0);
    let p2 = d(3) ^ d(2) ^ d(0);
    let p3 = d(3) ^ d(1) ^ d(0);
    nibble << 4 | p0 << 3 | p1 << 2 | p2 << 1 | p3
}

/// CR 4/5: the parity bit can only detect, so the data bits pass through.
/// CR 4/8: the nearest codeword (fixes 1 bad bit)
fn hamming_decode(codeword: u8, codeword_bits: usize) -> u8 {
    if codeword_bits == 5 {
        return codeword >> 1;
    }
    let mut best = codeword >> 4;
    let mut best_distance = (hamming_encode(best, 8) ^ codeword).count_ones();
    for nibble in 0..16 {
        let distance = (hamming_encode(nibble, 8) ^ codeword).count_ones();
        if distance < best_distance {
            best = nibble;
            best_distance = distance;
        }
    }
    best
}

fn to_gray(x: u32) -> u32 {
    x ^ (x >> 1)
}

fn from_gray(mut g: u32) -> u32 {
    let mut x = 0;
    while g != 0 {
        x ^= g;
        g >>= 1;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODAY: Format = Format {
        explicit_header: true,
        crc: true,
    };
    const CODED: Format = Format {
        explicit_header: false,
        crc: false,
    };

    /// Semtech's formula (SX1262 datasheet 6.1.4), no low data rate optimise
    fn semtech_symbols(len: usize, format: &Format) -> usize {
        let numerator = 8 * len as i64 - 4 * SF as i64 + 28 + 16 * format.crc as i64
            - 20 * (!format.explicit_header) as i64;
        let blocks = (numerator.max(0) as usize).div_ceil(4 * SF);
        8 + blocks * (4 + CR)
    }

    #[test]
    fn symbol_counts_match_semtech() {
        for len in 1..=100 {
            assert_eq!(symbols(len, &TODAY), semtech_symbols(len, &TODAY), "{len}");
            assert_eq!(symbols(len, &CODED), semtech_symbols(len, &CODED), "{len}");
        }
        // The two packets being compared take the same time on air
        assert_eq!(symbols(26, &TODAY), 48);
        assert_eq!(symbols(72, &CODED), 108);
        assert!((airtime_ms(26, &TODAY, 125e3) - 61.7).abs() < 0.1);
        assert!((airtime_ms(72, &CODED, 250e3) - 61.6).abs() < 0.1);
    }

    #[test]
    fn clean_channel_round_trip() {
        let payload: Vec<u8> = (0..72).map(|i| (i * 37 + 11) as u8).collect();
        let mut channel = Channel::new(1, SF as u32, 30.0, 1.0, None);
        assert_eq!(send(&payload, &TODAY, &mut channel), Some(payload.clone()));
        assert_eq!(send(&payload, &CODED, &mut channel), Some(payload));
    }

    #[test]
    fn hamming_4_8_fixes_one_bit() {
        for nibble in 0..16 {
            let cw = hamming_encode(nibble, 8);
            for bit in 0..8 {
                assert_eq!(hamming_decode(cw ^ 1 << bit, 8), nibble);
            }
        }
    }

    #[test]
    fn gray_round_trip_and_neighbours_differ_by_one_bit() {
        for x in 0..128 {
            assert_eq!(to_gray(from_gray(x)), x);
            assert_eq!((to_gray(x) ^ to_gray(x + 1)).count_ones(), 1);
        }
    }
}
