//! Our packets' CRC, checked by the radio driver instead of LoRa's own.
//!
//! LoRa's CRC is only checked if the received PHY header says there is one.
//! Noise can corrupt that header into "no CRC" and still pass its 5-bit
//! checksum, and then garbage arrives as a good packet. So we send with
//! LoRa's CRC off and append this one ourselves: the same 16 bits on the
//! air, but every packet is checked, whatever its header claims.
//!
//! CRC-16/CCITT-FALSE: polynomial 0x1021, initial value 0xFFFF, sent
//! big-endian after the data.

/// Bytes the CRC adds to a packet on the air
pub const CRC_BYTES: usize = 2;

pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// `framed` is data then its CRC: the data, if the CRC matches
pub fn check(framed: &[u8]) -> Option<&[u8]> {
    let n = framed.len().checked_sub(CRC_BYTES)?;
    let (data, sent) = framed.split_at(n);
    (crc16(data).to_be_bytes() == sent).then_some(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc16_ccitt_false_check_value() {
        // The catalogue's check value for "123456789"
        assert_eq!(crc16(b"123456789"), 0x29B1);
    }

    #[test]
    fn check_takes_good_and_refuses_bad() {
        let mut framed = b"voice".to_vec();
        framed.extend_from_slice(&crc16(b"voice").to_be_bytes());
        assert_eq!(check(&framed), Some(&b"voice"[..]));
        framed[1] ^= 0x01;
        assert_eq!(check(&framed), None);
        assert_eq!(check(&[0x12]), None); // shorter than a CRC
    }
}
