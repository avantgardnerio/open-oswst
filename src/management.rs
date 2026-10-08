//! Talking to the management server (issue #34; server/ in this repo):
//! connect, send one request, wait for its response. Blocking, so it runs
//! on a thread of its own: the net thread for the menu's requests (net.rs),
//! the HTTP API's for /api/management/echo (http.rs). Where the server is:
//! [server] in config.toml (devices/settings.rs). Plain TCP for now: Noise
//! (noise.rs) comes next, around each frame.

use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use open_oswst_core::management::{receive, send, Request, Response};

use crate::devices::settings;

/// For the connection, and then for the answer. The menu waits longer
/// (app.rs UPDATE_TIMEOUT), so it hears which one ran out
const TIMEOUT: Duration = Duration::from_secs(10);

/// Send `request` to the management server, and wait for its response. Err
/// says why there's none in a few words, for the screen (the Update page's
/// title: "timed out", "connection refused"). The server's address stays out
/// of it, and so out of the log
pub fn call(request: &Request) -> Result<Response, String> {
    let (host, port) = settings::server().ok_or("No [server] in config")?;
    let address = (host.as_str(), port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut addresses| addresses.next())
        .ok_or("Server not found")?;
    let mut stream =
        TcpStream::connect_timeout(&address, TIMEOUT).map_err(|e| e.kind().to_string())?;
    let io = |e: std::io::Error| e.kind().to_string();
    stream.set_read_timeout(Some(TIMEOUT)).map_err(io)?;
    stream.set_write_timeout(Some(TIMEOUT)).map_err(io)?;
    stream.set_nodelay(true).map_err(io)?;
    send(&mut stream, request).map_err(io)?;
    receive(&mut stream).map_err(io)
}
