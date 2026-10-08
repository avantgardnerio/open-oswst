//! The management server (issue #34). Radios dial out to it; it never
//! reaches them. Plain TCP for now: Noise comes next (src/noise.rs on the
//! radio), wrapped around each frame.
//!
//! Run: cargo run -- [ADDRESS:PORT], default 0.0.0.0:3101
//! Try it: cargo run --example echo -- HOST:PORT

mod oswst;
mod service;

use std::net::TcpListener;
use std::sync::Arc;

const DEFAULT_LISTEN: &str = "0.0.0.0:3101";

fn main() {
    let address = std::env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_LISTEN.into());
    let listener = TcpListener::bind(&address).unwrap_or_else(|e| {
        eprintln!("Can't listen on {}: {}", address, e);
        std::process::exit(1);
    });
    println!("Listening on {}", address);
    service::serve(listener, Arc::new(oswst::Oswst));
}
