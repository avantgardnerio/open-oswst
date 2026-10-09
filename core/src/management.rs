//! The management server's protocol (issue #34), shared by the radio and
//! the server (server/). The radio always dials out, so nothing on it
//! listens: it sends a request, the server answers it with one response,
//! and the radio may send another on the same connection.
//!
//! Layers, top down:
//! - messages: `Request` and `Response`, as postcard (`send`, `receive`)
//! - the link: each frame's bytes encrypted by Noise (`NoiseLink`). The
//!   handshake is Noise_XK_25519_AESGCM_SHA256: the radio knows the
//!   server's public key (an impostor can't answer it), and sends its own
//!   only once the session is forward secret. The server looks that key up
//!   in its list of radios. Both ends pass in their own crypto (snow's
//!   CryptoResolver): mbedTLS on the radio (noise.rs), RustCrypto on the
//!   server
//! - frames: a length (2 bytes, big endian), then that many bytes. A Noise
//!   message is at most 65535 bytes, tag included, hence the 2 bytes
//!
//! WARNING: postcard numbers an enum's variants by position. Only ever add
//! variants at the end: reorder or remove one and radios and the server
//! disagree on what was sent. A change neither can follow goes in PROLOGUE
//! instead: then old and new fail the handshake rather than misread.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use snow::resolvers::BoxedCryptoResolver;
use std::fmt;
use std::io::{self, Read, Write};

/// What the radio asks
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum Request {
    // WARNING: new variants only ever at the end (see the top)
    /// Bytes to send straight back: proves the link works, and at what size
    Echo(Vec<u8>),
    /// What we run: is there other firmware for us? Answered UpToDate or
    /// Offer. The server holds THE firmware, and radios take whatever it
    /// holds when it differs from theirs (so publishing an older image
    /// rolls them back)
    Hello {
        mac: [u8; 6],
        /// The running image's own SHA-256: the one ESP-IDF appends to
        /// every app image, its last 32 bytes
        running_sha256: Sha256,
        /// The build's version string (git hash): only shown, never compared
        version: String,
    },
    /// Part of the offered image. `sha256` says which image, so one
    /// published mid-download is noticed (Error IMAGE_CHANGED)
    Chunk {
        sha256: Sha256,
        offset: u32,
        len: u16,
    },
    /// Hello's successor: is there another update bundle for us (bundle.rs)?
    /// Answered UpToDate or BundleOffer. Up to date means we run the
    /// bundle's firmware AND installed that bundle last
    CheckBundle {
        mac: [u8; 6],
        running_sha256: Sha256,
        /// The SHA-256 of the bundle we installed last: None if none yet
        /// (new, or only ever updated by Hello)
        installed_sha256: Option<Sha256>,
        version: String,
    },
    /// Part of the offered bundle, as Chunk is of an image
    BundleChunk {
        sha256: Sha256,
        offset: u32,
        len: u16,
    },
}

/// What the server answers
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Echo(Vec<u8>),
    /// The request couldn't be read (one this server doesn't know, from a
    /// newer radio), or failed, or this radio isn't allowed (NOT_ALLOWED).
    /// Few words: the radio shows it on the screen
    Error(String),
    /// We run the server's image already
    UpToDate,
    Offer(Offer),
    /// The bytes a Chunk (or BundleChunk) asked for
    Chunk(Vec<u8>),
    /// An update bundle for us: `size` and `sha256` are the whole .tar.gz
    /// file's, `version` its firmware's
    BundleOffer(Offer),
}

/// Firmware (or a bundle) the server has for us
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub version: String,
    pub size: u32,
    /// The image's own SHA-256 (its last 32 bytes)
    pub sha256: Sha256,
}

pub type Sha256 = [u8; 32];

/// The most a Chunk asks for: the image comes this much at a time
pub const CHUNK: u16 = 4096;

/// What a radio asking for part of an image that's since been replaced
/// is told: start over with a Hello
pub const IMAGE_CHANGED: &str = "Image changed";

/// What a radio the server doesn't know (or has revoked) is told
pub const NOT_ALLOWED: &str = "Not allowed";

pub const PATTERN: &str = "Noise_XK_25519_AESGCM_SHA256";

/// Hashed into the handshake by both ends: a protocol neither old nor new
/// can follow changes this, and fails the handshake rather than misreads
pub const PROLOGUE: &[u8] = b"oswst-management/1";

/// The longest frame: what its 2-byte length can say
pub const MAX_FRAME: usize = u16::MAX as usize;

/// An AES-GCM tag, on every encrypted frame
const TAG_LEN: usize = 16;

/// X25519 keys, private and public alike
pub type Key = [u8; 32];

/// Why there's no link. Display is a few words, for the radio's screen
#[derive(Debug)]
pub enum LinkError {
    Io(io::Error),
    /// The server hung up on our first handshake message: it couldn't read
    /// it, so the key we have for it isn't its key (or it isn't our server),
    /// or it speaks another version of this protocol (PROLOGUE)
    Refused,
    Noise(snow::Error),
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            LinkError::Io(e) => write!(f, "{}", e.kind()),
            LinkError::Refused => write!(f, "Wrong key or version"),
            LinkError::Noise(e) => write!(f, "Noise: {}", e),
        }
    }
}

impl From<io::Error> for LinkError {
    fn from(e: io::Error) -> Self {
        LinkError::Io(e)
    }
}

impl From<snow::Error> for LinkError {
    fn from(e: snow::Error) -> Self {
        LinkError::Noise(e)
    }
}

/// A connection after the handshake: every frame encrypted
pub struct NoiseLink<S> {
    stream: S,
    noise: snow::TransportState,
}

impl<S: Read + Write> NoiseLink<S> {
    /// The radio's end: XK as initiator. `server` is the server's public key
    pub fn connect(
        mut stream: S,
        resolver: BoxedCryptoResolver,
        ours: &Key,
        server: &Key,
    ) -> Result<NoiseLink<S>, LinkError> {
        let mut noise = builder(resolver)?
            .local_private_key(ours)?
            .remote_public_key(server)?
            .build_initiator()?;
        let mut buf = [0u8; 256];
        // -> e, es
        let len = noise.write_message(&[], &mut buf)?;
        write_frame(&mut stream, &buf[..len])?;
        // <- e, ee. A server that couldn't read our message hangs up
        let frame = read_frame(&mut stream).map_err(|e| match e.kind() {
            io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset => LinkError::Refused,
            _ => LinkError::Io(e),
        })?;
        noise
            .read_message(&frame, &mut buf)
            .map_err(|_| LinkError::Refused)?;
        // -> s, se
        let len = noise.write_message(&[], &mut buf)?;
        write_frame(&mut stream, &buf[..len])?;
        Ok(NoiseLink {
            stream,
            noise: noise.into_transport_mode()?,
        })
    }

    /// The server's end: XK as responder. Also returns the radio's public
    /// key, for the server to look up
    pub fn accept(
        mut stream: S,
        resolver: BoxedCryptoResolver,
        ours: &Key,
    ) -> Result<(NoiseLink<S>, Key), LinkError> {
        let mut noise = builder(resolver)?
            .local_private_key(ours)?
            .build_responder()?;
        let mut buf = [0u8; 256];
        let frame = read_frame(&mut stream)?;
        noise.read_message(&frame, &mut buf)?;
        let len = noise.write_message(&[], &mut buf)?;
        write_frame(&mut stream, &buf[..len])?;
        let frame = read_frame(&mut stream)?;
        noise.read_message(&frame, &mut buf)?;
        let radio: Key = noise
            .get_remote_static()
            .and_then(|key| key.try_into().ok())
            .ok_or(LinkError::Noise(snow::Error::Input))?;
        let link = NoiseLink {
            stream,
            noise: noise.into_transport_mode()?,
        };
        Ok((link, radio))
    }

    /// Send one message, encrypted
    pub fn send<T: Serialize>(&mut self, message: &T) -> io::Result<()> {
        let plain = postcard::to_allocvec(message).map_err(invalid)?;
        if plain.len() + TAG_LEN > MAX_FRAME {
            return Err(invalid("message over a frame"));
        }
        let mut sealed = vec![0u8; plain.len() + TAG_LEN];
        let len = self
            .noise
            .write_message(&plain, &mut sealed)
            .map_err(invalid)?;
        write_frame(&mut self.stream, &sealed[..len])
    }

    /// Wait for one message. ErrorKind::InvalidData: it arrived, but failed
    /// to decrypt or isn't a message we know
    pub fn receive<T: DeserializeOwned>(&mut self) -> io::Result<T> {
        let sealed = read_frame(&mut self.stream)?;
        let mut plain = vec![0u8; sealed.len()];
        let len = self
            .noise
            .read_message(&sealed, &mut plain)
            .map_err(invalid)?;
        postcard::from_bytes(&plain[..len]).map_err(invalid)
    }
}

fn builder<'a>(resolver: BoxedCryptoResolver) -> Result<snow::Builder<'a>, snow::Error> {
    snow::Builder::with_resolver(PATTERN.parse()?, resolver).prologue(PROLOGUE)
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

/// A key as 64 hex digits, as the key files hold them
pub fn key_to_hex(key: &Key) -> String {
    key.iter().map(|byte| format!("{:02x}", byte)).collect()
}

pub fn key_from_hex(text: &str) -> Option<Key> {
    let text = text.trim();
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut key = [0u8; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};

    /// Both ends of a TCP connection on this PC: (radio, server)
    fn connection() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let radio = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        (radio, server)
    }

    fn keypair() -> (Key, Key) {
        let keys = snow::Builder::new(PATTERN.parse().unwrap())
            .generate_keypair()
            .unwrap();
        (
            keys.private.try_into().unwrap(),
            keys.public.try_into().unwrap(),
        )
    }

    fn resolver() -> BoxedCryptoResolver {
        Box::new(snow::resolvers::DefaultResolver)
    }

    #[test]
    fn handshake_then_a_request_and_its_response() {
        let (server_private, server_public) = keypair();
        let (radio_private, radio_public) = keypair();
        let (radio_end, server_end) = connection();
        let server = std::thread::spawn(move || {
            let (mut link, radio) =
                NoiseLink::accept(server_end, resolver(), &server_private).unwrap();
            let request: Request = link.receive().unwrap();
            let Request::Echo(bytes) = request else {
                panic!("not an echo")
            };
            link.send(&Response::Echo(bytes)).unwrap();
            radio
        });
        let mut link =
            NoiseLink::connect(radio_end, resolver(), &radio_private, &server_public).unwrap();
        link.send(&Request::Echo(vec![1, 2, 3])).unwrap();
        let response: Response = link.receive().unwrap();
        assert_eq!(response, Response::Echo(vec![1, 2, 3]));
        // The server learnt who we are
        assert_eq!(server.join().unwrap(), radio_public);
    }

    #[test]
    fn the_wrong_server_key_is_caught() {
        let (server_private, _) = keypair();
        let (_, someone_else) = keypair();
        let (radio_private, _) = keypair();
        let (radio_end, server_end) = connection();
        // The server fails on our first message, and hangs up (drops it)
        std::thread::spawn(move || {
            assert!(NoiseLink::accept(server_end, resolver(), &server_private).is_err())
        });
        let result = NoiseLink::connect(radio_end, resolver(), &radio_private, &someone_else);
        assert!(matches!(result, Err(LinkError::Refused)));
    }

    #[test]
    fn an_unknown_variant_is_an_error_not_a_guess() {
        let got: Result<Request, _> = postcard::from_bytes(&[9, 0]);
        assert!(got.is_err());
    }

    #[test]
    fn a_frame_too_long_is_refused() {
        let mut wire = Vec::new();
        assert!(write_frame(&mut wire, &vec![0; MAX_FRAME + 1]).is_err());
    }

    #[test]
    fn keys_go_to_hex_and_back() {
        let (_, key) = keypair();
        assert_eq!(key_from_hex(&key_to_hex(&key)), Some(key));
        assert_eq!(key_from_hex("00"), None);
        assert_eq!(key_from_hex(&"zz".repeat(32)), None);
    }
}
