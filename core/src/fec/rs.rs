//! Reed-Solomon over GF(256). Works on whole bytes: with `parity` check bytes
//! it corrects any `parity / 2` bad bytes, however many bits in each are
//! wrong, and wherever they are.
//!
//! A block is data bytes then parity bytes, read as the coefficients of a
//! polynomial, first byte = highest power. Shorter than 255 bytes is fine:
//! it's the same code with leading zeros left out.
//!
//! Decoding is the textbook chain: syndromes (is anything wrong?) ->
//! Berlekamp-Massey (the error locator polynomial) -> Chien search (which
//! bytes) -> Forney (by how much).

/// Field polynomial x^8 + x^4 + x^3 + x^2 + 1, the usual one
const FIELD_POLY: u16 = 0x11D;
/// Most parity we support (scheme B uses 45). Sizes the decoder's arrays
const MAX_PARITY: usize = 64;

/// log and antilog tables, so multiply and divide are additions
struct Tables {
    exp: [u8; 512], // alpha^i; doubled so exp[log a + log b] never wraps
    log: [u8; 256],
}

static TABLES: Tables = build_tables();

const fn build_tables() -> Tables {
    let mut exp = [0u8; 512];
    let mut log = [0u8; 256];
    let mut x: u16 = 1;
    let mut i = 0;
    while i < 255 {
        exp[i] = x as u8;
        log[x as usize] = i as u8;
        x <<= 1;
        if x & 0x100 != 0 {
            x ^= FIELD_POLY;
        }
        i += 1;
    }
    while i < 512 {
        exp[i] = exp[i - 255];
        i += 1;
    }
    Tables { exp, log }
}

/// Fill `parity` with the check bytes for `data` (the remainder of
/// data(x) * x^parity divided by the generator polynomial)
pub fn encode(data: &[u8], parity: &mut [u8]) {
    let generator = generator(parity.len());
    parity.fill(0);
    for &byte in data {
        let feedback = byte ^ parity[0];
        parity.copy_within(1.., 0);
        *parity.last_mut().unwrap() = 0;
        if feedback != 0 {
            for (p, &g) in parity.iter_mut().zip(&generator[1..]) {
                *p ^= mul(feedback, g);
            }
        }
    }
}

/// Correct `block` (data then `parity` check bytes) in place. Some(number of
/// bytes fixed), or None if there are too many errors to fix
pub fn decode(block: &mut [u8], parity: usize) -> Option<usize> {
    assert!(parity <= MAX_PARITY && block.len() <= 255);
    let mut syndromes = [0u8; MAX_PARITY];
    if !compute_syndromes(block, &mut syndromes[..parity]) {
        return Some(0); // a valid codeword: nothing to fix
    }
    let syndromes = &syndromes[..parity];

    let (locator, errors) = berlekamp_massey(syndromes);
    if errors > parity / 2 {
        return None;
    }

    // Omega(x) = S(x) * Lambda(x) mod x^parity, lowest power first
    let mut omega = [0u8; MAX_PARITY];
    for (k, o) in omega.iter_mut().enumerate().take(parity) {
        for i in 0..=k.min(errors) {
            *o ^= mul(locator[i], syndromes[k - i]);
        }
    }

    // Chien search: try every byte position. Position j holds the power
    // n-1-j, and is in error if Lambda(alpha^-(n-1-j)) = 0
    let n = block.len();
    let mut fixed = 0;
    for (j, byte) in block.iter_mut().enumerate() {
        let power = n - 1 - j;
        let x_inv = TABLES.exp[(255 - power % 255) % 255];
        if eval_low_first(&locator[..=errors], x_inv) != 0 {
            continue;
        }
        // Forney: the error value is X * Omega(X^-1) / Lambda'(X^-1). The
        // derivative keeps only the odd powers (in GF(2^m), 2 = 0):
        // lambda_1 + lambda_3 x^2 + lambda_5 x^4 ...
        let mut derivative = 0u8;
        let mut x_pow = 1u8; // x_inv^(i-1)
        for (i, &coefficient) in locator.iter().enumerate().take(errors + 1).skip(1) {
            if i % 2 == 1 {
                derivative ^= mul(coefficient, x_pow);
            }
            x_pow = mul(x_pow, x_inv);
        }
        if derivative == 0 {
            return None;
        }
        let x = TABLES.exp[power % 255];
        *byte ^= mul(x, div(eval_low_first(&omega[..parity], x_inv), derivative));
        fixed += 1;
    }

    // The locator's degree must match the roots found in the block. If not,
    // there were more errors than we can fix
    if fixed != errors {
        return None;
    }
    let mut check = [0u8; MAX_PARITY];
    if compute_syndromes(block, &mut check[..parity]) {
        return None;
    }
    Some(fixed)
}

/// The generator polynomial (x + alpha^0)(x + alpha^1)...(x + alpha^(parity-1)),
/// highest power first. Monic, so element 0 is always 1
fn generator(parity: usize) -> [u8; MAX_PARITY + 1] {
    assert!(parity <= MAX_PARITY);
    let mut g = [0u8; MAX_PARITY + 1];
    g[0] = 1;
    for i in 0..parity {
        let root = TABLES.exp[i];
        for j in (1..=i + 1).rev() {
            g[j] ^= mul(root, g[j - 1]);
        }
    }
    g
}

/// S_i = block(alpha^i). All zero means a valid codeword; returns whether
/// any is nonzero
fn compute_syndromes(block: &[u8], syndromes: &mut [u8]) -> bool {
    let mut any = false;
    for (i, s) in syndromes.iter_mut().enumerate() {
        let alpha_i = TABLES.exp[i];
        *s = block.iter().fold(0, |acc, &b| mul(acc, alpha_i) ^ b);
        any |= *s != 0;
    }
    any
}

/// The error locator Lambda(x), lowest power first, and its degree (the
/// number of errors). Massey's algorithm: build the shortest polynomial that
/// generates the syndrome sequence
fn berlekamp_massey(syndromes: &[u8]) -> ([u8; MAX_PARITY + 1], usize) {
    let mut locator = [0u8; MAX_PARITY + 1];
    locator[0] = 1;
    let mut prev = locator; // the locator before the last length change
    let mut length = 0;
    let mut shift = 1; // steps since `prev` was saved
    let mut prev_discrepancy = 1u8;

    for n in 0..syndromes.len() {
        // How far off the current locator is at predicting syndrome n
        let mut discrepancy = syndromes[n];
        for i in 1..=length {
            discrepancy ^= mul(locator[i], syndromes[n - i]);
        }
        if discrepancy == 0 {
            shift += 1;
            continue;
        }
        let scale = div(discrepancy, prev_discrepancy);
        let before = locator;
        for i in shift..=MAX_PARITY {
            locator[i] ^= mul(scale, prev[i - shift]);
        }
        if 2 * length <= n {
            length = n + 1 - length;
            prev = before;
            prev_discrepancy = discrepancy;
            shift = 1;
        } else {
            shift += 1;
        }
    }
    (locator, length)
}

/// Evaluate a polynomial stored lowest power first
fn eval_low_first(poly: &[u8], x: u8) -> u8 {
    poly.iter().rev().fold(0, |acc, &c| mul(acc, x) ^ c)
}

fn mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        return 0;
    }
    TABLES.exp[TABLES.log[a as usize] as usize + TABLES.log[b as usize] as usize]
}

fn div(a: u8, b: u8) -> u8 {
    if a == 0 {
        return 0;
    }
    TABLES.exp[TABLES.log[a as usize] as usize + 255 - TABLES.log[b as usize] as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny deterministic generator, so the tests need no crates
    fn next(seed: &mut u32) -> u32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 17;
        *seed ^= *seed << 5;
        *seed
    }

    fn codeword(seed: &mut u32, data_len: usize, parity: usize) -> Vec<u8> {
        let mut block: Vec<u8> = (0..data_len + parity).map(|_| next(seed) as u8).collect();
        let (data, check) = block.split_at_mut(data_len);
        encode(data, check);
        block
    }

    #[test]
    fn known_vector() {
        // A case worked by hand, so the encoder isn't only checked against
        // its own decoder. Data [1] with 2 parity bytes: the remainder of x^2
        // divided by g = (x + 1)(x + alpha) = x^2 + 3x + 2 is 3x + 2
        let mut parity = [0u8; 2];
        encode(&[1], &mut parity);
        assert_eq!(parity, [3, 2]);
    }

    #[test]
    fn corrects_up_to_half_the_parity() {
        let mut seed = 0x1234_5678;
        for parity in [8, 45] {
            for trial in 0..200 {
                let good = codeword(&mut seed, 27, parity);
                let mut bad = good.clone();
                let errors = trial % (parity / 2 + 1);
                let mut hit = Vec::new();
                while hit.len() < errors {
                    let at = next(&mut seed) as usize % bad.len();
                    if !hit.contains(&at) {
                        bad[at] ^= (next(&mut seed) as u8) | 1;
                        hit.push(at);
                    }
                }
                assert_eq!(decode(&mut bad, parity), Some(errors));
                assert_eq!(bad, good);
            }
        }
    }

    #[test]
    fn too_many_errors_is_none_or_a_codeword() {
        // Past the limit the decoder must not claim a fix it didn't make:
        // it says None, or (rarely) lands on a different valid codeword,
        // which is what the CRC is for
        let mut seed = 0x0BAD_F00D;
        let mut none = 0;
        for _ in 0..200 {
            let mut bad = codeword(&mut seed, 27, 8);
            for i in 0..6 {
                bad[i * 5] ^= 0x41;
            }
            match decode(&mut bad, 8) {
                None => none += 1,
                Some(_) => {
                    let mut syndromes = [0u8; 8];
                    assert!(!compute_syndromes(&bad, &mut syndromes));
                }
            }
        }
        assert!(none > 190);
    }
}
