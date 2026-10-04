//! The 2-byte header on every packet: |5b type|7b txid|4b seq|.
//! A voice or echo header with no payload after it marks end of
//! transmission (EOT).

use crate::air;
use crate::devices::radio::TxRequest;

/// Live voice from a PTT press
pub const TYPE_VOICE: u8 = 0x00;
/// Voice played back by an echo station. Plays like voice, but echo stations
/// never echo it, so two of them can't bounce a transmission back and forth.
pub const TYPE_ECHO: u8 = 0x01;

/// Wake-up: sent first, header only, after a long preamble
/// (air::wake_preamble_symbols) for a radio sweeping the hop channels to
/// find. Carries no audio. Not an EOT.
pub const TYPE_WAKE: u8 = 0x02;

/// The wake-up packet for transmission `txid`
pub fn wake(txid: u8) -> TxRequest {
    let mut data = heapless::Vec::new();
    let _ = data.extend_from_slice(&pack(TYPE_WAKE, txid, 0));
    TxRequest {
        data,
        preamble: Some(air::wake_preamble_symbols()),
    }
}

pub struct Header {
    pub pkt_type: u8,
    pub txid: u8, // random per transmission: the dedup key
    pub seq: u8,  // wraps at 16
}

pub fn pack(pkt_type: u8, txid: u8, seq: u8) -> [u8; 2] {
    ((pkt_type as u16) << 11 | (txid as u16) << 4 | seq as u16).to_be_bytes()
}

pub fn unpack(bytes: [u8; 2]) -> Header {
    let header = u16::from_be_bytes(bytes);
    Header {
        pkt_type: (header >> 11) as u8,
        txid: ((header >> 4) & 0x7F) as u8,
        seq: (header & 0x0F) as u8,
    }
}
