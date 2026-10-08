//! A server for any request and response types: one thread per connection,
//! a Noise handshake, then the radio's requests answered one by one until
//! it hangs up. Which radios get in, what the requests are and what answers
//! them is the `Service`'s business (oswst.rs); the link, the frames and
//! their encoding are core's management.rs.

use open_oswst_core::management::{key_to_hex, Key, NoiseLink};
use serde::de::DeserializeOwned;
use serde::Serialize;
use snow::resolvers::DefaultResolver;
use std::fmt::Debug;
use std::io::{self, ErrorKind};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::keys::ServerKey;

/// A connection that sends nothing for this long is closed
const IDLE: Duration = Duration::from_secs(60);

pub trait Service: Send + Sync + 'static {
    type Request: DeserializeOwned + Debug;
    type Response: Serialize + Debug;
    /// Who's asking, as `handle` sees them
    type Peer: std::fmt::Display;

    /// Who has this public key, or why they can't come in
    fn admit(&self, key: &Key) -> Result<Self::Peer, String>;

    fn handle(&self, peer: &Self::Peer, request: Self::Request) -> Self::Response;

    /// The one answer a radio that isn't admitted gets, before we hang up
    fn refused(&self) -> Self::Response;

    /// The answer to a request that couldn't be read: from a newer radio,
    /// say, with a request this server doesn't know yet
    fn unreadable(&self, error: &io::Error) -> Self::Response;
}

/// Answer connections on `listener`, for ever
pub fn serve<S: Service>(listener: TcpListener, service: Arc<S>, key: Arc<ServerKey>) {
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(e) => {
                eprintln!("Accept failed: {}", e);
                continue;
            }
        };
        let (service, key) = (service.clone(), key.clone());
        thread::spawn(move || {
            let address = stream.peer_addr().map(|address| address.to_string());
            let address = address.unwrap_or_else(|_| "?".into());
            match answer(&*service, &key, stream, &address) {
                Ok(()) => println!("{} hung up", address),
                Err(e) => println!("{} dropped: {}", address, e),
            }
        });
    }
}

/// The handshake, then requests in, responses out, until the radio hangs up
/// (Ok) or the connection fails, idles out or is refused (Err)
fn answer<S: Service>(
    service: &S,
    key: &ServerKey,
    stream: TcpStream,
    address: &str,
) -> io::Result<()> {
    stream.set_read_timeout(Some(IDLE))?;
    stream.set_nodelay(true)?;
    let (mut link, radio_key) = NoiseLink::accept(stream, Box::new(DefaultResolver), &key.private)
        .map_err(|e| io::Error::other(format!("handshake: {}", e)))?;
    let peer = match service.admit(&radio_key) {
        Ok(peer) => peer,
        Err(why) => {
            link.send(&service.refused())?;
            return Err(io::Error::other(format!(
                "refused key {}: {}",
                key_to_hex(&radio_key),
                why
            )));
        }
    };
    println!("{} is {}", address, peer);
    loop {
        let response = match link.receive::<S::Request>() {
            Ok(request) => {
                println!("{} {}", address, short(&request));
                service.handle(&peer, request)
            }
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => return Ok(()),
            // The frame arrived whole, but isn't a request we know
            Err(e) if e.kind() == ErrorKind::InvalidData => {
                println!("{} unreadable: {}", address, e);
                service.unreadable(&e)
            }
            Err(e) => return Err(e),
        };
        println!("{} -> {}", address, short(&response));
        link.send(&response)?;
    }
}

/// A message for the log: its start, as a big one would fill the screen
fn short(message: &impl Debug) -> String {
    const MAX: usize = 100;
    let text = format!("{:?}", message);
    match text.char_indices().nth(MAX) {
        Some((cut, _)) => format!("{}...", &text[..cut]),
        None => text,
    }
}
