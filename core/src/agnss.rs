//! Helping the GPS (Quectel L76K, CASIC AT6558R chipset) find the sky fast:
//! tell it roughly where and when it is, and hand it the satellites' orbits.
//!
//! - `aid_ini`: CASIC AID-INI (0x0B 0x01): position, GPS time and their
//!   accuracy. From a WiFi network's configured location and NTP time, or
//!   the last fix we saved
//! - `keep_good_frames`: the CASIC frames of an assisted-GPS file
//!   (Espruino's hourly `casic.base64`: 32 MSG-GPSEPH ephemerides and a
//!   MSG-GPSION), checked and ready to write to the GPS as they are. The
//!   chip takes its own ephemeris output messages as input (Bangle.js 2
//!   does this; the CASIC spec only lists them as output)
//!
//! Frames (docs/CASIC_AT6558_protocol_specification_V4.2.0.3.pdf, 2.2):
//! `BA CE`, payload length (u16 LE), class, id, payload (a multiple of 4
//! bytes), checksum: u32 LE, (id << 24) + (class << 16) + length plus the
//! payload as little-endian u32 words, wrapping.

/// GPS time runs ahead of UTC by the leap seconds since 1980: 18 since
/// 2017-01-01 (none announced since)
const GPS_MINUS_UTC_S: i64 = 18;
/// The GPS epoch, 1980-01-06T00:00:00Z, in unix seconds
const GPS_EPOCH_UNIX_S: i64 = 315_964_800;
const WEEK_S: i64 = 7 * 24 * 3600;
/// Light speed, for AID-INI's time variance (its unit is (s·c)², per spec)
const C_M_PER_S: f64 = 299_792_458.0;

/// A text command for the GPS: `$<body>*<checksum>\r\n`, the checksum the
/// XOR of the body's bytes (NMEA). e.g. `PCAS10,2`: cold start
pub fn nmea(body: &str) -> Vec<u8> {
    let sum = body.bytes().fold(0u8, |sum, byte| sum ^ byte);
    format!("${}*{:02X}\r\n", body, sum).into_bytes()
}

/// One CASIC frame
pub fn frame(class: u8, id: u8, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u16;
    let mut sum = ((id as u32) << 24)
        .wrapping_add((class as u32) << 16)
        .wrapping_add(len as u32);
    for word in payload.chunks(4) {
        let mut bytes = [0u8; 4];
        bytes[..word.len()].copy_from_slice(word);
        sum = sum.wrapping_add(u32::from_le_bytes(bytes));
    }
    let mut out = Vec::with_capacity(payload.len() + 10);
    out.extend_from_slice(&[0xBA, 0xCE]);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&[class, id]);
    out.extend_from_slice(payload);
    out.extend_from_slice(&sum.to_le_bytes());
    out
}

/// Roughly where we are: degrees, metres above the ellipsoid, and how far
/// off that may be (1 sigma, metres)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Where {
    pub lat: f64,
    pub lon: f64,
    pub alt: f64,
    pub accuracy_m: f32,
}

/// The time now, and how far off it may be (1 sigma, seconds)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct When {
    pub unix_ms: i64,
    pub accuracy_s: f32,
}

/// GPS week number and seconds into the week, for a UTC time
pub fn gps_week_tow(unix_ms: i64) -> (u16, f64) {
    let gps_ms = unix_ms - GPS_EPOCH_UNIX_S * 1000 + GPS_MINUS_UTC_S * 1000;
    let week = gps_ms.div_euclid(WEEK_S * 1000);
    let tow = gps_ms.rem_euclid(WEEK_S * 1000) as f64 / 1000.0;
    (week as u16, tow)
}

/// AID-INI (0x0B 0x01, 56 bytes): whatever we know of where and when. The
/// flags say which parts are valid, so either may be left out
pub fn aid_ini(place: Option<Where>, time: Option<When>) -> Vec<u8> {
    let mut p = [0u8; 56];
    let mut flags = 0u8;
    if let Some(at) = place {
        p[0..8].copy_from_slice(&at.lat.to_le_bytes());
        p[8..16].copy_from_slice(&at.lon.to_le_bytes());
        p[16..24].copy_from_slice(&at.alt.to_le_bytes());
        // pAcc: the variance of the 3D position, m². (Bangle.js 2 sends 0;
        // with either, the chip ignored our orbits, 2026-10-04)
        p[36..40].copy_from_slice(&(at.accuracy_m * at.accuracy_m).to_le_bytes());
        flags |= 0b0000_0001 | 0b0010_0000; // position valid, given as lat/lon/alt
    }
    if let Some(now) = time {
        let (week, tow) = gps_week_tow(now.unix_ms);
        p[24..32].copy_from_slice(&tow.to_le_bytes());
        // tAcc: the variance of the time, in (s·c)², e.g. 9 = 10ns
        let t_acc = (now.accuracy_s as f64 * C_M_PER_S).powi(2) as f32;
        p[40..44].copy_from_slice(&t_acc.to_le_bytes());
        p[52..54].copy_from_slice(&week.to_le_bytes());
        flags |= 0b0000_0010; // time valid
    }
    p[55] = flags;
    frame(0x0B, 0x01, &p)
}

/// Keep only the good CASIC frames of an assisted-GPS file, back to back,
/// in the same buffer (the heap can't spare a second copy): anything else
/// (Espruino's file starts with a text banner) and any frame whose checksum
/// fails is dropped. Returns how many frames are left
pub fn keep_good_frames(file: &mut Vec<u8>) -> usize {
    let mut kept = 0; // bytes of good frames at the front
    let mut count = 0;
    let mut i = 0;
    while i + 10 <= file.len() {
        if file[i] != 0xBA || file[i + 1] != 0xCE {
            i += 1;
            continue;
        }
        let len = u16::from_le_bytes([file[i + 2], file[i + 3]]) as usize;
        let end = i + 6 + len + 4;
        if end > file.len() {
            break;
        }
        if checksum_ok(&file[i..end]) {
            file.copy_within(i..end, kept);
            kept += end - i;
            count += 1;
            i = end;
        } else {
            i += 1;
        }
    }
    file.truncate(kept);
    count
}

/// Does a whole frame's checksum match?
fn checksum_ok(frame: &[u8]) -> bool {
    let len = frame.len() - 10;
    let mut sum = ((frame[5] as u32) << 24)
        .wrapping_add((frame[4] as u32) << 16)
        .wrapping_add(len as u32);
    for word in frame[6..6 + len].chunks(4) {
        let mut bytes = [0u8; 4];
        bytes[..word.len()].copy_from_slice(word);
        sum = sum.wrapping_add(u32::from_le_bytes(bytes));
    }
    sum.to_le_bytes() == frame[6 + len..]
}

/// Ask the GPS for NAV-GPSINFO (0x01 0x20) once: CFG-MSG with rate 0xFFFF
/// ("output once, now")
pub fn poll_gps_info() -> Vec<u8> {
    frame(0x06, 0x01, &[0x01, 0x20, 0xFF, 0xFF])
}

/// What NAV-GPSINFO says of the GPS satellites the receiver lists: above
/// all, whether it holds their orbits from ephemeris (ours, if it took
/// them) or only an almanac
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GpsInfo {
    pub listed: u8,
    pub used: u8,
    /// Orbit (prediction) information from ephemeris: flags bits 7:6 = 11
    pub ephemeris: u8,
    /// From an almanac only: bits 7:6 = 01
    pub almanac: u8,
    /// Prediction marked invalid: bit 4
    pub invalid: u8,
    /// Being heard now: C/N0 above 0
    pub heard: u8,
}

/// NAV-GPSINFO's payload: 8 bytes, then 12 per satellite
/// (docs/CASIC_AT6558_protocol_specification_V4.2.0.3.pdf, 2.7.7)
pub fn read_gps_info(payload: &[u8]) -> Option<GpsInfo> {
    let listed = *payload.get(4)?;
    let mut info = GpsInfo {
        listed,
        used: *payload.get(5)?,
        ..Default::default()
    };
    for sat in payload.get(8..8 + 12 * listed as usize)?.chunks(12) {
        let flags = sat[2];
        match flags >> 6 {
            0b11 => info.ephemeris += 1,
            0b01 => info.almanac += 1,
            _ => {}
        }
        if flags & 0b1_0000 != 0 {
            info.invalid += 1;
        }
        if sat[4] > 0 {
            info.heard += 1;
        }
    }
    Some(info)
}

/// Base64 (standard alphabet, padding optional, whitespace ignored), for
/// Espruino's file. None if it isn't base64
pub fn base64_decode(text: &[u8]) -> Option<Vec<u8>> {
    fn value(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    }
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for &c in text {
        if c == b'=' || c.is_ascii_whitespace() {
            continue;
        }
        acc = (acc << 6) | value(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real MSG-GPSION (0x08 0x06) frame from Espruino's file, 2026-10-04
    const ION: [u8; 26] = [
        0xBA, 0xCE, 0x10, 0x00, 0x08, 0x06, 0x4D, 0x04, 0xFA, 0x00, 0x12, 0x02, 0xFE, 0xFE, 0x38,
        0x02, 0xFC, 0x01, 0x03, 0x00, 0x00, 0x00, 0xAA, 0x08, 0xFC, 0x07,
    ];

    #[test]
    fn frames_match_the_chips_own() {
        assert_eq!(frame(0x08, 0x06, &ION[6..22]), ION);
    }

    #[test]
    fn the_file_keeps_only_good_frames() {
        let mut file = b"AGNSS data from CASIC.\nDataLength: 26.\n".to_vec();
        file.extend_from_slice(&ION);
        let mut broken = ION;
        broken[10] ^= 1; // a bad checksum
        file.extend_from_slice(&broken);
        file.extend_from_slice(&ION);
        assert_eq!(keep_good_frames(&mut file), 2);
        assert_eq!(file, [ION, ION].concat());
    }

    #[test]
    fn gps_time_from_utc() {
        // 2026-10-04T20:00:00Z: GPS week 2439, Sunday 20:00:18 GPS (checked in Python)
        let (week, tow) = gps_week_tow(1_791_144_000_000);
        assert_eq!(week, 2439);
        assert_eq!(tow, 20.0 * 3600.0 + 18.0);
        // The epoch itself, less the leap seconds
        assert_eq!(gps_week_tow((GPS_EPOCH_UNIX_S - 18) * 1000), (0, 0.0));
    }

    #[test]
    fn aid_ini_says_what_it_knows() {
        let place = Where {
            lat: 51.4779,
            lon: -0.0015,
            alt: 1525.0,
            accuracy_m: 100.0,
        };
        let now = When {
            unix_ms: 1_791_144_000_000,
            accuracy_s: 0.1,
        };
        let both = aid_ini(Some(place), Some(now));
        assert_eq!(both.len(), 66);
        assert_eq!(&both[..6], &[0xBA, 0xCE, 56, 0, 0x0B, 0x01]);
        let p = &both[6..62];
        assert_eq!(f64::from_le_bytes(p[0..8].try_into().unwrap()), 51.4779);
        assert_eq!(u16::from_le_bytes([p[52], p[53]]), 2439);
        assert_eq!(p[55], 0x23); // position, time, lat/lon: Bangle.js 2's flags
        assert_eq!(frame(0x0B, 0x01, p), both); // checksum
        assert_eq!(aid_ini(Some(place), None)[61], 0x21);
        assert_eq!(aid_ini(None, Some(now))[61], 0x02);
    }

    #[test]
    fn nmea_commands_match_the_spec() {
        // docs/Quectel_L76K_GNSS_protocol_specification_V1.1.pdf, PCAS10
        assert_eq!(nmea("PCAS10,0"), b"$PCAS10,0*1C\r\n");
    }

    #[test]
    fn gps_info_counts_orbit_sources() {
        assert_eq!(
            poll_gps_info(),
            frame(0x06, 0x01, &[0x01, 0x20, 0xFF, 0xFF])
        );
        let mut payload = vec![0, 0, 0, 0, 3, 1, 0, 0];
        // channel, svid, flags, quality, C/N0, elevation, azimuth (2), residual (4)
        payload.extend_from_slice(&[0, 5, 0b1100_0001, 0, 38, 40, 0, 0, 0, 0, 0, 0]);
        payload.extend_from_slice(&[1, 13, 0b0100_0000, 0, 0, 20, 0, 0, 0, 0, 0, 0]);
        payload.extend_from_slice(&[2, 21, 0b0001_0000, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let info = read_gps_info(&payload).unwrap();
        assert_eq!(
            info,
            GpsInfo {
                listed: 3,
                used: 1,
                ephemeris: 1,
                almanac: 1,
                invalid: 1,
                heard: 1
            }
        );
        assert_eq!(read_gps_info(&payload[..20]), None); // cut short
    }

    #[test]
    fn base64_round_trip() {
        assert_eq!(base64_decode(b"QUdOU1M=").unwrap(), b"AGNSS");
        assert_eq!(base64_decode(b"QUdO\nU1M").unwrap(), b"AGNSS");
        assert!(base64_decode(b"Q!").is_none());
    }
}
