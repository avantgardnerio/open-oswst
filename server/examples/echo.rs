//! Talk to a management server the way a radio will: connect, send Echo
//! requests of growing size, check each comes back, time the round trips.
//!
//! cargo run --example echo -- HOST:PORT

use open_oswst_core::management::{receive, send, Request, Response};
use std::net::TcpStream;
use std::time::Instant;

fn main() {
    let address = std::env::args().nth(1).expect("usage: echo HOST:PORT");
    let started = Instant::now();
    let mut stream = TcpStream::connect(&address).expect("connect");
    println!("Connected to {} in {:?}", address, started.elapsed());
    for size in [0, 16, 1024, 4096, 60_000] {
        let bytes: Vec<u8> = (0..size).map(|i| i as u8).collect();
        let started = Instant::now();
        send(&mut stream, &Request::Echo(bytes.clone())).expect("send");
        let response: Response = receive(&mut stream).expect("receive");
        let ok = response == Response::Echo(bytes);
        println!(
            "Echo {:>6} B: {:?} {}",
            size,
            started.elapsed(),
            if ok { "ok" } else { "WRONG" }
        );
    }
}
