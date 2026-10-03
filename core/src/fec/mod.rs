//! # RETIRED (2026-10-03): FEC does not help this radio. Not used, on purpose.
//!
//! We built it, simulated it and then measured the thing it's meant to fix,
//! in the field, twice. FEC repairs packets that arrive damaged. Ours almost
//! never arrive damaged: they don't arrive at all.
//!
//! - **Simulation** (`examples/fec_sim`, SF7, white noise): both candidates
//!   gain ~3 dB over today's packet, which is exactly what moving to 250 kHz
//!   (the bandwidth they need for the same airtime) costs. Net ~0 dB. They
//!   only win in short noise bursts inside a packet.
//! - **West walk** (away from town, 17 PTTs to ~1.35 km): 2 packets on the
//!   whole walk failed their CRC. Every other loss left no trace.
//! - **East walk** (toward town, the noisiest direction we know, repeater
//!   off, 44 PTTs to ~1.3 km): of 454 packets lost on the return path, 12
//!   failed their CRC or header check (<3%). The other ~97% were never
//!   detected. That was with real interference: noise bursts filled 44% of
//!   the time in one 10 s window out of ten, peaking at 100%.
//!
//! So even in the noisiest place we've tested, at most ~3% of the losses are
//! the kind any in-packet FEC could repair. The losses are missed detections:
//! the receiver never locks onto the packet. What helps that is a channel the
//! noise isn't on (frequency hopping), more margin (antenna height, power),
//! and detection itself (preamble length), not coding.
//!
//! QMesh, the voice-over-LoRa project this design followed, estimates 5-7 dB
//! of gain from this coding over LoRa's own Hamming code. Their own word for
//! that number is "a very rough estimate", and we found no field measurement
//! behind it. Ours points the other way. We'd encourage anyone (QMesh
//! included) to log CRC failures against total losses on a real walk before
//! building on that figure.
//!
//! The code below stays as written and tested, for the record and in case a
//! different radio, channel or environment ever turns the numbers around.
//! Field logs: `logs/` (not in git) F85B1BA2C62C-20261003-100318/0011.txt
//! (west), F85B1BA2C62C-20261003-160209/log/0310.txt (east), and the echo
//! station's logs from the same runs.
//!
//! ---
//!
//! Forward error correction for a voice packet: two candidates, both filling
//! the same 72 bytes (the same airtime). Never on the air: `examples/fec_sim`
//! compares them against today's packet on a simulated LoRa link.
//!
//! Both start the same way: the 26 voice bytes plus our own CRC-8, because the
//! radio's CRC is off (a corrupt packet still reaches us, for the FEC to fix).
//!
//! - **A, QMesh's scheme**: Reed-Solomon with 8 parity bytes, then a rate-1/2
//!   convolutional code, then a bit interleaver.
//! - **B, Reed-Solomon only**: all 45 spare bytes are parity.
//!
//! The radio stays on LoRa CR 4/5. That's only a parity bit, which the chip
//! can't correct with, so on its own it does nothing for us.

pub mod conv;
pub mod crc8;
pub mod interleave;
pub mod rs;

/// Codec2 1200 frames plus our 2-byte header, as today
pub const VOICE_BYTES: usize = 26;
/// The voice bytes plus a CRC-8
const DATA_BYTES: usize = VOICE_BYTES + 1;
/// The coded packet. 72 bytes is the most that fits in 108 LoRa symbols
/// (SF7, CR 4/5, implicit header, CRC off); 73 would take 113
pub const CODED_BYTES: usize = 72;

/// A: Reed-Solomon parity bytes (QMesh uses RS(32,24): 8 of them)
const A_PARITY: usize = 8;
/// A: what the convolutional code is fed. 35 bytes in, 2 x (280 + 6 tail)
/// = 572 bits out, padded to 576 = 72 bytes
const A_RS_BYTES: usize = DATA_BYTES + A_PARITY;

/// B: every byte that isn't data is parity, so it corrects 22 bad bytes
const B_PARITY: usize = CODED_BYTES - DATA_BYTES;

pub fn encode_a(voice: &[u8; VOICE_BYTES]) -> [u8; CODED_BYTES] {
    let mut block = [0u8; A_RS_BYTES];
    add_crc(voice, &mut block);
    let (data, parity) = block.split_at_mut(DATA_BYTES);
    rs::encode(data, parity);

    let mut coded = [0u8; CODED_BYTES];
    conv::encode(&block, &mut coded);
    let mut out = [0u8; CODED_BYTES];
    interleave::interleave(&coded, &mut out);
    out
}

/// None if it can't be corrected (or the CRC says the correction is wrong)
pub fn decode_a(received: &[u8; CODED_BYTES]) -> Option<[u8; VOICE_BYTES]> {
    let mut coded = [0u8; CODED_BYTES];
    interleave::deinterleave(received, &mut coded);
    let mut block = [0u8; A_RS_BYTES];
    conv::decode(&coded, &mut block);

    rs::decode(&mut block, A_PARITY)?;
    check_crc(&block[..DATA_BYTES])
}

pub fn encode_b(voice: &[u8; VOICE_BYTES]) -> [u8; CODED_BYTES] {
    let mut block = [0u8; CODED_BYTES];
    add_crc(voice, &mut block);
    let (data, parity) = block.split_at_mut(DATA_BYTES);
    rs::encode(data, parity);
    block
}

pub fn decode_b(received: &[u8; CODED_BYTES]) -> Option<[u8; VOICE_BYTES]> {
    let mut block = *received;
    rs::decode(&mut block, B_PARITY)?;
    check_crc(&block[..DATA_BYTES])
}

/// Voice bytes then their CRC-8, at the front of `block`
fn add_crc(voice: &[u8; VOICE_BYTES], block: &mut [u8]) {
    block[..VOICE_BYTES].copy_from_slice(voice);
    block[VOICE_BYTES] = crc8::crc8(voice);
}

fn check_crc(data: &[u8]) -> Option<[u8; VOICE_BYTES]> {
    let (voice, crc) = data.split_at(VOICE_BYTES);
    (crc8::crc8(voice) == crc[0]).then(|| voice.try_into().unwrap())
}

/// Bit `i` of a byte buffer, most significant bit of each byte first
fn bit(buf: &[u8], i: usize) -> u8 {
    (buf[i / 8] >> (7 - i % 8)) & 1
}

fn set_bit(buf: &mut [u8], i: usize, value: u8) {
    let mask = 0x80 >> (i % 8);
    if value != 0 {
        buf[i / 8] |= mask;
    } else {
        buf[i / 8] &= !mask;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VOICE: [u8; VOICE_BYTES] = [
        0x00, 0x13, 0xA5, 0x5A, 0xFF, 0x01, 0x80, 0x7E, 0x42, 0x99, 0x10, 0x20, 0x30, 0x40, 0x50,
        0x60, 0x70, 0x81, 0x92, 0xA3, 0xB4, 0xC5, 0xD6, 0xE7, 0xF8, 0x09,
    ];

    #[test]
    fn a_round_trip() {
        assert_eq!(decode_a(&encode_a(&VOICE)), Some(VOICE));
    }

    #[test]
    fn b_round_trip() {
        assert_eq!(decode_b(&encode_b(&VOICE)), Some(VOICE));
    }

    #[test]
    fn a_corrects_scattered_bit_errors() {
        let mut coded = encode_a(&VOICE);
        // 20 flipped bits, spread out: the interleaver hands the Viterbi
        // decoder isolated errors, which it fixes before RS even looks
        for i in 0..20 {
            coded[i * 3] ^= 1 << (i % 8);
        }
        assert_eq!(decode_a(&coded), Some(VOICE));
    }

    #[test]
    fn b_corrects_22_bad_bytes_but_not_23() {
        let mut coded = encode_b(&VOICE);
        for i in 0..22 {
            coded[i * 3] ^= 0x5A;
        }
        assert_eq!(decode_b(&coded), Some(VOICE));
        coded[70] ^= 0x01;
        assert_eq!(decode_b(&coded), None);
    }
}
