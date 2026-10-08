//! The management server (issue #34). Radios dial out to it; it never
//! reaches them. Every connection is Noise (core's management.rs): a radio
//! must know this server's public key, and this server must find the
//! radio's in its list (radios.toml), or it's refused.
//!
//! Its files live in one directory (DIR), outside the repo:
//!   server.key    this server's key pair (keys.rs). Private: mode 0600
//!   radios.toml   the radios allowed in: re-read at every connection, so
//!                 adding or revoking one needs no restart (keys.rs)
//!   firmware.bin  THE firmware: radios that run anything else are offered
//!                 it (firmware.rs; scripts/publish-firmware.sh)
//!
//! Run:  cargo run -- DIR [ADDRESS:PORT]   (default 0.0.0.0:3101)
//! Once: cargo run -- keygen DIR            (prints the public key, for
//!                                          each radio's secrets.toml)

mod firmware;
mod keys;
mod oswst;
mod service;

use std::net::TcpListener;
use std::path::Path;
use std::sync::Arc;

const DEFAULT_LISTEN: &str = "0.0.0.0:3101";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["keygen", dir] => keygen(Path::new(dir)),
        [dir] => run(Path::new(dir), DEFAULT_LISTEN),
        [dir, address] => run(Path::new(dir), address),
        _ => fail("usage: server DIR [ADDRESS:PORT] | server keygen DIR"),
    }
}

fn keygen(dir: &Path) {
    match keys::ServerKey::generate(dir) {
        Ok(key) => println!("Server public key: {}", key.public_hex()),
        Err(e) => fail(&e),
    }
}

fn run(dir: &Path, address: &str) {
    let key = keys::ServerKey::load(dir).unwrap_or_else(|e| fail(&e));
    let listener = TcpListener::bind(address)
        .unwrap_or_else(|e| fail(&format!("Can't listen on {}: {}", address, e)));
    println!("Listening on {}, public key {}", address, key.public_hex());
    let service = oswst::Oswst {
        radios: dir.join(keys::RADIOS_FILE),
        firmware: firmware::Firmware::new(dir.join(firmware::FIRMWARE_FILE)),
    };
    service::serve(listener, Arc::new(service), Arc::new(key));
}

fn fail(message: &str) -> ! {
    eprintln!("{}", message);
    std::process::exit(1);
}
