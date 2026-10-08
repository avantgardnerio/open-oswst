//! The management server's protocol (issue #34), shared by the radio and
//! the server (server/). The radio always dials out, so nothing on it
//! listens: it sends a request, the server answers it with one response,
//! and the radio may send another on the same connection.
//!
//! Each message is a frame: its length (2 bytes, big endian), then that many
//! bytes of postcard. Noise will encrypt each frame's bytes (a Noise message
//! is at most 65535 bytes, hence the 2-byte length); nothing above the
//! frames changes when it does.
//!
//! `send` and `receive` take any serde type, so the radio and server agree
//! on messages only through the enums below. WARNING: postcard numbers an
//! enum's variants by position. Only ever add variants at the end: reorder
//! or remove one and radios and the server disagree on what was sent.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

/// What the radio asks
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Bytes to send straight back: proves the link works, and at what size
    Echo(Vec<u8>),
}

/// What the server answers
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Echo(Vec<u8>),
    /// The request couldn't be read (one this server doesn't know, from a
    /// newer radio), or failed
    Error(String),
}

/// The longest frame: what its 2-byte length can say
pub const MAX_FRAME: usize = u16::MAX as usize;

/// Send one message as a frame
pub fn send<T: Serialize>(stream: &mut impl Write, message: &T) -> io::Result<()> {
    let bytes = postcard::to_allocvec(message).map_err(invalid)?;
    write_frame(stream, &bytes)
}

/// Wait for one frame and read the message in it
pub fn receive<T: DeserializeOwned>(stream: &mut impl Read) -> io::Result<T> {
    let bytes = read_frame(stream)?;
    postcard::from_bytes(&bytes).map_err(invalid)
}

/// One write, length and bytes together: written apart, TCP holds the
/// bytes back until the length is acknowledged (Nagle against delayed ACK,
/// ~40 ms a frame)
pub fn write_frame(stream: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    let len = u16::try_from(bytes.len()).map_err(|_| invalid("frame over 65535 bytes"))?;
    let mut frame = Vec::with_capacity(2 + bytes.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(bytes);
    stream.write_all(&frame)?;
    stream.flush()
}

pub fn read_frame(stream: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 2];
    stream.read_exact(&mut len)?;
    let mut bytes = vec![0u8; u16::from_be_bytes(len) as usize];
    stream.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn invalid(error: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_survives_the_trip() {
        let mut wire = Vec::new();
        send(&mut wire, &Request::Echo(vec![1, 2, 3])).unwrap();
        // 2 bytes of length, then variant 0, length 3, the bytes
        assert_eq!(wire, [0, 5, 0, 3, 1, 2, 3]);
        let got: Request = receive(&mut wire.as_slice()).unwrap();
        assert_eq!(got, Request::Echo(vec![1, 2, 3]));
    }

    #[test]
    fn an_unknown_variant_is_an_error_not_a_guess() {
        let mut wire = Vec::new();
        write_frame(&mut wire, &[9, 0]).unwrap();
        let got: io::Result<Request> = receive(&mut wire.as_slice());
        assert_eq!(got.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_frame_too_long_is_refused() {
        let mut wire = Vec::new();
        assert!(write_frame(&mut wire, &vec![0; MAX_FRAME + 1]).is_err());
    }
}
