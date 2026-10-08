//! A server for any request and response types: one thread per connection,
//! reading a request, answering it, until the radio hangs up. What the
//! requests are, and what answers them, is the `Service`'s business
//! (oswst.rs); the frames and their encoding are core's management.rs.

use open_oswst_core::management::{receive, send};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fmt::Debug;
use std::io::{self, ErrorKind};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// A connection that sends nothing for this long is closed
const IDLE: Duration = Duration::from_secs(60);

pub trait Service: Send + Sync + 'static {
    type Request: DeserializeOwned + Debug;
    type Response: Serialize + Debug;

    fn handle(&self, peer: SocketAddr, request: Self::Request) -> Self::Response;

    /// The answer to a request that couldn't be read: from a newer radio,
    /// say, with a request this server doesn't know yet
    fn unreadable(&self, error: &io::Error) -> Self::Response;
}

/// Answer connections on `listener`, for ever
pub fn serve<S: Service>(listener: TcpListener, service: Arc<S>) {
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(e) => {
                eprintln!("Accept failed: {}", e);
                continue;
            }
        };
        let service = service.clone();
        thread::spawn(move || {
            let peer = stream.peer_addr().map(|peer| peer.to_string());
            let peer = peer.unwrap_or_else(|_| "?".into());
            println!("{} connected", peer);
            match answer(&*service, stream) {
                Ok(()) => println!("{} hung up", peer),
                Err(e) => println!("{} dropped: {}", peer, e),
            }
        });
    }
}

/// Requests in, responses out, until the radio hangs up (Ok) or the
/// connection fails or idles out (Err)
fn answer<S: Service>(service: &S, mut stream: TcpStream) -> io::Result<()> {
    stream.set_read_timeout(Some(IDLE))?;
    stream.set_nodelay(true)?;
    let peer = stream.peer_addr()?;
    loop {
        let response = match receive::<S::Request>(&mut stream) {
            Ok(request) => {
                println!("{} {}", peer, short(&request));
                service.handle(peer, request)
            }
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => return Ok(()),
            // The frame arrived whole, but isn't a request we know
            Err(e) if e.kind() == ErrorKind::InvalidData => {
                println!("{} unreadable: {}", peer, e);
                service.unreadable(&e)
            }
            Err(e) => return Err(e),
        };
        println!("{} -> {}", peer, short(&response));
        send(&mut stream, &response)?;
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
