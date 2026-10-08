//! Talking to the management server (issue #34; server/ in this repo):
//! connect, the Noise handshake (core's management.rs, on mbedTLS:
//! noise.rs), one request, wait for its response. Blocking, so it runs on a
//! thread of its own: the net thread for the menu's requests (net.rs), the
//! HTTP API's for /api/management/echo (http.rs). Where the server is and
//! the keys: secrets.toml (devices/secrets.rs).

use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use open_oswst_core::management::{NoiseLink, Request, Response};

use crate::devices::secrets;
use crate::noise::MbedtlsResolver;

/// For the connection, and then for each answer (the handshake's too). The
/// menu waits longer (app.rs UPDATE_TIMEOUT), so it hears which one ran out
const TIMEOUT: Duration = Duration::from_secs(10);

/// Send `request` to the management server, and wait for its response. Err
/// says why there's none in a few words, for the screen (the Update page's
/// title: "timed out", "Wrong key or version"). The server's address stays
/// out of it, and so out of the log
pub fn call(request: &Request) -> Result<Response, String> {
    let server = secrets::server()?;
    let ours = secrets::private_key().ok_or("No radio key yet")?;
    let address = (server.host.as_str(), server.port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut addresses| addresses.next())
        .ok_or("Server not found")?;
    let io = |e: std::io::Error| e.kind().to_string();
    let stream = TcpStream::connect_timeout(&address, TIMEOUT).map_err(io)?;
    stream.set_read_timeout(Some(TIMEOUT)).map_err(io)?;
    stream.set_write_timeout(Some(TIMEOUT)).map_err(io)?;
    stream.set_nodelay(true).map_err(io)?;
    let mut link = NoiseLink::connect(stream, Box::new(MbedtlsResolver), &ours, &server.public_key)
        .map_err(|e| e.to_string())?;
    link.send(request).map_err(io)?;
    link.receive().map_err(io)
}
