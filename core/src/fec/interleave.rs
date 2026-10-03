//! Bit interleaver for scheme A: 576 bits written into a 24 x 24 grid by row
//! and read out by column.
//!
//! The Viterbi decoder copes with errors spread out, but not with several
//! close together. A bad LoRa symbol damages bits that sit close together in
//! the packet, so we spread neighbouring coded bits 24 bits apart before they
//! go out. Our own choice of grid: QMesh's isn't documented.

use super::{bit, set_bit, CODED_BYTES};

const SIDE: usize = 24;
const _: () = assert!(SIDE * SIDE == CODED_BYTES * 8);

pub fn interleave(input: &[u8; CODED_BYTES], out: &mut [u8; CODED_BYTES]) {
    for row in 0..SIDE {
        for col in 0..SIDE {
            set_bit(out, col * SIDE + row, bit(input, row * SIDE + col));
        }
    }
}

/// The grid is square, so undoing it is the same transpose
pub fn deinterleave(input: &[u8; CODED_BYTES], out: &mut [u8; CODED_BYTES]) {
    interleave(input, out);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_spread() {
        let mut input = [0u8; CODED_BYTES];
        input[0] = 0b1100_0000; // bits 0 and 1, neighbours
        let mut mixed = [0u8; CODED_BYTES];
        interleave(&input, &mut mixed);
        assert_eq!(bit(&mixed, 0), 1);
        assert_eq!(bit(&mixed, 24), 1); // now 24 apart
        let mut back = [0u8; CODED_BYTES];
        deinterleave(&mixed, &mut back);
        assert_eq!(back, input);
    }
}
