//! The 2-byte header on every packet: |5b type|7b txid|3b hops|1b spare|.
//! `hops`: how many repeaters relayed this copy, 0 from the talker. Hop h of
//! a transmission's packet n goes on the air in conveyor bin 2n + h
//! (conveyor.rs), so every copy of a packet, however it came, is the same
//! packet: copies that loop back through repeaters are duplicates. The spare
//! bit is sent 0 and ignored.
//! A transmission ends with an end packet (EOT): who sent it and where they
//! were (`Ident`), the same 26 bytes as a voice packet, so it takes the
//! same air and fits a repeater's slot like one.

use crate::air;
use crate::codec::{HEADER_BYTES, PACKET_BYTES};
use crate::devices::radio::TxRequest;

/// What a packet is: the header's 5-bit type
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PacketType {
    /// Live voice from a PTT press
    Voice = 0x00,
    /// Voice played back by an echo station. Plays like voice, but echo
    /// stations never echo it, so two of them can't bounce a transmission
    /// back and forth
    Echo = 0x01,
    /// Wake-up: sent first, header only, after a long preamble
    /// (air::wake_preamble_symbols) for a radio sweeping the hop channels
    /// to find. Carries no audio. Not an EOT
    Wake = 0x02,
    /// The end of live voice: the talker's Ident
    VoiceEnd = 0x03,
    /// The end of an echo replay: the echo station's Ident. Separate from
    /// VoiceEnd so echo stations never replay a replay
    EchoEnd = 0x04,
    /// src/bin/radio_timing.rs's test packets: the app drops them, so other
    /// radios nearby ignore a timing run
    Bench = 0x1F,
}

impl TryFrom<u8> for PacketType {
    type Error = u8;

    /// The type a header's 5 bits name, or the bits if they name none
    fn try_from(bits: u8) -> Result<PacketType, u8> {
        Ok(match bits {
            0x00 => PacketType::Voice,
            0x01 => PacketType::Echo,
            0x02 => PacketType::Wake,
            0x03 => PacketType::VoiceEnd,
            0x04 => PacketType::EchoEnd,
            0x1F => PacketType::Bench,
            other => return Err(other),
        })
    }
}

/// Position bytes in an end packet: latitude and longitude, 3 bytes each
const POSITION_BYTES: usize = 6;
/// The longest name an end packet carries: the rest of its 26 bytes
pub const NAME_BYTES: usize = PACKET_BYTES - HEADER_BYTES - POSITION_BYTES;

/// 24-bit signed coordinates: ±2^23 steps over ±90° (latitude, ~1.2m) and
/// ±180° (longitude, ~2.4m at the equator, less towards the poles)
const COORD_STEPS: f64 = 8_388_607.0; // 2^23 - 1
/// A latitude no position encodes to: "no fix"
const NO_FIX: i32 = -8_388_608; // -2^23

/// Who sent a transmission, and where they were when it ended
#[derive(Clone, Debug, PartialEq)]
pub struct Ident {
    pub name: heapless::String<NAME_BYTES>,
    pub position: Option<(f64, f64)>, // lat, lon in degrees
}

/// The end packet for transmission `txid`: header, latitude, longitude, then
/// the name, zero-padded to 26 bytes
pub fn end(end_type: PacketType, txid: u8, ident: &Ident) -> heapless::Vec<u8, 255> {
    let mut data = heapless::Vec::new();
    let _ = data.extend_from_slice(&pack(end_type, txid));
    let (lat, lon) = match ident.position {
        Some((lat, lon)) => (to_steps(lat, 90.0), to_steps(lon, 180.0)),
        None => (NO_FIX, 0),
    };
    let _ = data.extend_from_slice(&lat.to_be_bytes()[1..]);
    let _ = data.extend_from_slice(&lon.to_be_bytes()[1..]);
    let _ = data.extend_from_slice(ident.name.as_bytes());
    let _ = data.resize(PACKET_BYTES, 0);
    data
}

/// The Ident in an end packet; None if it isn't one we can read
pub fn read_end(data: &[u8]) -> Option<Ident> {
    if data.len() != PACKET_BYTES {
        return None;
    }
    let lat = from_24_bits(&data[HEADER_BYTES..HEADER_BYTES + 3]);
    let lon = from_24_bits(&data[HEADER_BYTES + 3..HEADER_BYTES + POSITION_BYTES]);
    let position = (lat != NO_FIX).then(|| {
        (
            lat as f64 * 90.0 / COORD_STEPS,
            lon as f64 * 180.0 / COORD_STEPS,
        )
    });
    let name_bytes = &data[HEADER_BYTES + POSITION_BYTES..];
    let len = name_bytes
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(name_bytes.len());
    let name = core::str::from_utf8(&name_bytes[..len]).ok()?;
    Some(Ident {
        name: name.try_into().ok()?,
        position,
    })
}

fn to_steps(degrees: f64, range: f64) -> i32 {
    let steps = (degrees / range * COORD_STEPS).round();
    steps.clamp(-COORD_STEPS, COORD_STEPS) as i32
}

/// A big-endian 24-bit signed number
fn from_24_bits(bytes: &[u8]) -> i32 {
    let raw = (bytes[0] as i32) << 16 | (bytes[1] as i32) << 8 | bytes[2] as i32;
    (raw << 8) >> 8 // sign-extend
}

/// The wake-up packet for transmission `txid`
pub fn wake(txid: u8) -> TxRequest {
    let mut data = heapless::Vec::new();
    let _ = data.extend_from_slice(&pack(PacketType::Wake, txid));
    TxRequest {
        data,
        preamble: Some(air::wake_preamble_symbols()),
        channel: 0,
        clear_air_first: true,
        send_at_us: None,
    }
}

/// The most relays a copy can have had: 3 bits. Only a backstop: a copy
/// that loops back through repeaters is a duplicate long before this
pub const MAX_HOPS: u8 = 7;

pub struct Header {
    /// The type, or the 5 bits if they name no type we know (garbage)
    pub pkt_type: Result<PacketType, u8>,
    pub txid: u8, // random per transmission: the dedup key
    /// Repeaters this copy came through: 0 = straight from the talker
    pub hops: u8,
}

/// A header straight from the talker: no hops yet
pub fn pack(pkt_type: PacketType, txid: u8) -> [u8; 2] {
    ((pkt_type as u16) << 11 | (txid as u16) << 4).to_be_bytes()
}

pub fn unpack(bytes: [u8; 2]) -> Header {
    let header = u16::from_be_bytes(bytes);
    Header {
        pkt_type: PacketType::try_from((header >> 11) as u8),
        txid: ((header >> 4) & 0x7F) as u8,
        hops: ((header >> 1) & 0x07) as u8,
    }
}

/// The header a repeater sends its relay of a copy with: one more hop. None
/// once the copy has had MAX_HOPS
pub fn relayed(bytes: [u8; 2]) -> Option<[u8; 2]> {
    let header = u16::from_be_bytes(bytes);
    let hops = ((header >> 1) & 0x07) as u8;
    if hops >= MAX_HOPS {
        return None;
    }
    let bumped = (header & !0x000E) | ((hops as u16 + 1) << 1);
    Some(bumped.to_be_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ident(name: &str, position: Option<(f64, f64)>) -> Ident {
        Ident {
            name: name.try_into().unwrap(),
            position,
        }
    }

    #[test]
    fn an_end_packet_is_a_voice_packets_size_and_reads_back() {
        let sent = ident("Cornelious", Some((40.543_901, -105.091_852)));
        let data = end(PacketType::VoiceEnd, 42, &sent);
        assert_eq!(data.len(), PACKET_BYTES);
        let header = unpack([data[0], data[1]]);
        assert_eq!(
            (header.pkt_type, header.txid, header.hops),
            (Ok(PacketType::VoiceEnd), 42, 0)
        );
        let got = read_end(&data).unwrap();
        assert_eq!(got.name, "Cornelious");
        let (lat, lon) = got.position.unwrap();
        // 3 bytes each: within ~1.2m (latitude) and ~2.4m (longitude)
        assert!((lat - 40.543_901).abs() < 1.1e-5);
        assert!((lon + 105.091_852).abs() < 2.2e-5);
    }

    #[test]
    fn no_fix_and_the_extremes() {
        assert_eq!(
            read_end(&end(PacketType::EchoEnd, 1, &ident("x", None)))
                .unwrap()
                .position,
            None
        );
        for (lat, lon) in [(90.0, 180.0), (-90.0, -180.0), (0.0, 0.0)] {
            let got =
                read_end(&end(PacketType::VoiceEnd, 1, &ident("x", Some((lat, lon))))).unwrap();
            let (got_lat, got_lon) = got.position.unwrap();
            assert!((got_lat - lat).abs() < 1.1e-5 && (got_lon - lon).abs() < 2.2e-5);
        }
    }

    #[test]
    fn a_name_fills_the_packet_and_garbage_names_are_refused() {
        let long = "eighteen-byte-name";
        assert_eq!(long.len(), NAME_BYTES);
        assert_eq!(
            read_end(&end(PacketType::VoiceEnd, 1, &ident(long, None)))
                .unwrap()
                .name,
            long
        );
        let mut data = end(PacketType::VoiceEnd, 1, &ident("ok", None));
        data[HEADER_BYTES + POSITION_BYTES] = 0xFF; // not UTF-8
        assert_eq!(read_end(&data), None);
        assert_eq!(read_end(&data[..HEADER_BYTES]), None);
    }

    #[test]
    fn every_type_round_trips_and_unknown_bits_are_kept() {
        use PacketType::*;
        for t in [Voice, Echo, Wake, VoiceEnd, EchoEnd, Bench] {
            assert_eq!(PacketType::try_from(t as u8), Ok(t));
            assert_eq!(unpack(pack(t, 99)).pkt_type, Ok(t));
        }
        assert_eq!(PacketType::try_from(0x15), Err(0x15));
    }

    #[test]
    fn each_relay_adds_a_hop_up_to_the_most() {
        let mut header = pack(PacketType::Voice, 99);
        for hops in 1..=MAX_HOPS {
            header = relayed(header).unwrap();
            let got = unpack(header);
            assert_eq!(
                (got.pkt_type, got.txid, got.hops),
                (Ok(PacketType::Voice), 99, hops)
            );
        }
        assert_eq!(relayed(header), None);
    }

    #[test]
    fn the_spare_bit_is_ignored() {
        let [high, low] = pack(PacketType::Echo, 127);
        let got = unpack([high, low | 1]);
        assert_eq!(
            (got.pkt_type, got.txid, got.hops),
            (Ok(PacketType::Echo), 127, 0)
        );
    }
}
