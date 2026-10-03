//! Rate-1/2, constraint length 7 convolutional code (the standard 171/133
//! octal pair) with a hard-decision Viterbi decoder.
//!
//! Every input bit sends two bits, each a parity of the bit and the 6 before
//! it. The decoder finds the input whose coded bits differ least from what
//! arrived. "Hard": the radio hands us bits, not how sure it was of each one,
//! which costs this code roughly 2 dB against a soft decoder.
//!
//! After the data, 6 zero "tail" bits bring the encoder back to state 0, so
//! the decoder knows where the path ends.

use super::{bit, set_bit};

const G1: u8 = 0o171;
const G2: u8 = 0o133;
const STATES: usize = 64; // the 6 previous input bits
const TAIL_BITS: usize = 6;

/// The longest input we decode: scheme A's 35 bytes. Sizes the decoder's
/// table of decisions (one u64 per step, 2.3 KB)
const MAX_DATA_BITS: usize = 35 * 8;

/// Encode all of `data`, then the tail. `out` needs 2 x (bits + 6) bits;
/// any bits after that are left 0
pub fn encode(data: &[u8], out: &mut [u8]) {
    out.fill(0);
    let mut state = 0u8;
    for step in 0..data.len() * 8 + TAIL_BITS {
        let input = if step < data.len() * 8 {
            bit(data, step)
        } else {
            0
        };
        let (out1, out2) = outputs(state, input);
        set_bit(out, 2 * step, out1);
        set_bit(out, 2 * step + 1, out2);
        state = next_state(state, input);
    }
}

/// Recover `out.len()` bytes from what `encode` sent. Always gives an answer:
/// whether it's right is for the Reed-Solomon code and the CRC to judge
pub fn decode(received: &[u8], out: &mut [u8]) {
    let data_bits = out.len() * 8;
    assert!(data_bits <= MAX_DATA_BITS);
    let steps = data_bits + TAIL_BITS;

    // Forward pass. metric[s] = fewest bit errors on any path that ends in
    // state s. decisions[step] bit s = which of the two ways into s won
    let mut metric = [u16::MAX / 2; STATES];
    metric[0] = 0; // the encoder starts in state 0
    let mut decisions = [0u64; MAX_DATA_BITS + TAIL_BITS];
    for (step, decision) in decisions.iter_mut().enumerate().take(steps) {
        let got1 = bit(received, 2 * step);
        let got2 = bit(received, 2 * step + 1);
        let mut next = [0u16; STATES];
        for (state, next_metric) in next.iter_mut().enumerate() {
            let input = (state >> 5) as u8; // the newest bit of the state
            let mut best = u16::MAX;
            for oldest in 0..2u8 {
                let prev = previous_state(state as u8, oldest);
                let (out1, out2) = outputs(prev, input);
                let errors = (out1 != got1) as u16 + (out2 != got2) as u16;
                let total = metric[prev as usize].saturating_add(errors);
                if total < best {
                    best = total;
                    *decision = (*decision & !(1 << state)) | (oldest as u64) << state;
                }
            }
            *next_metric = best;
        }
        metric = next;
    }

    // Trace back from state 0 (the tail put the encoder there)
    let mut state = 0u8;
    for step in (0..steps).rev() {
        if step < data_bits {
            set_bit(out, step, state >> 5);
        }
        let oldest = ((decisions[step] >> state) & 1) as u8;
        state = previous_state(state, oldest);
    }
}

/// The two coded bits for `input` arriving in `state`. The shift register is
/// the input (bit 6) followed by the state (bits 5..0)
fn outputs(state: u8, input: u8) -> (u8, u8) {
    let register = input << 6 | state;
    (
        (register & G1).count_ones() as u8 & 1,
        (register & G2).count_ones() as u8 & 1,
    )
}

/// The register shifts right: the input becomes the state's top bit and the
/// oldest bit falls off the bottom
fn next_state(state: u8, input: u8) -> u8 {
    (input << 6 | state) >> 1
}

/// The state before `state`, given the bit that fell off the bottom
fn previous_state(state: u8, oldest: u8) -> u8 {
    (state & 0x1F) << 1 | oldest
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATA: [u8; 35] = [
        0x00, 0xFF, 0x13, 0x37, 0xA5, 0x5A, 0x01, 0x80, 0xDE, 0xAD, 0xBE, 0xEF, 0x11, 0x22, 0x33,
        0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0x0F, 0xF0, 0x3C, 0xC3,
        0x69, 0x96, 0x12, 0x34, 0x56,
    ];

    fn round_trip(flip: &[usize]) -> [u8; 35] {
        let mut coded = [0u8; 72];
        encode(&DATA, &mut coded);
        for &i in flip {
            coded[i / 8] ^= 0x80 >> (i % 8);
        }
        let mut out = [0u8; 35];
        decode(&coded, &mut out);
        out
    }

    #[test]
    fn clean() {
        assert_eq!(round_trip(&[]), DATA);
    }

    #[test]
    fn corrects_isolated_errors() {
        // Free distance 10: errors 40 bits apart are each fixed on their own
        let flips: Vec<usize> = (0..14).map(|i| 3 + i * 40).collect();
        assert_eq!(round_trip(&flips), DATA);
    }

    #[test]
    fn a_burst_breaks_it() {
        // 8 errors in a row: why scheme A needs the interleaver
        let flips: Vec<usize> = (100..108).collect();
        assert_ne!(round_trip(&flips), DATA);
    }
}
