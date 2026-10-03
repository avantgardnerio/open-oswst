//! CRC-8 (polynomial 0x07). With the radio's CRC off, this is what tells a
//! good packet from one the FEC couldn't fix, or "fixed" wrongly.

pub fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &byte in data {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    #[test]
    fn check_value() {
        // The standard CRC-8 check: "123456789" -> 0xF4
        assert_eq!(super::crc8(b"123456789"), 0xF4);
    }
}
