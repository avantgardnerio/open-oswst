//! Talk to a management server the way a radio does: Noise handshake, then
//! Echo requests of growing size, each checked and timed.
//!
//! cargo run --example echo -- HOST:PORT SERVER_PUBLIC_KEY [RADIO_PRIVATE_KEY]
//!
//! Without a private key it makes one up and prints its public key: the
//! server doesn't know it, so it should answer "Not allowed" (add it to
//! radios.toml to be let in)

use open_oswst_core::management::{
    key_from_hex, key_to_hex, NoiseLink, Request, Response, PATTERN,
};
use snow::resolvers::DefaultResolver;
use std::net::TcpStream;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (address, server) = match &args[..] {
        [address, server, ..] => (address, key_from_hex(server).expect("server key: 64 hex")),
        _ => panic!("usage: echo HOST:PORT SERVER_PUBLIC_KEY [RADIO_PRIVATE_KEY]"),
    };
    let private = match args.get(2) {
        Some(private) => key_from_hex(private).expect("radio key: 64 hex"),
        None => {
            let keys = snow::Builder::new(PATTERN.parse().unwrap())
                .generate_keypair()
                .unwrap();
            let public: [u8; 32] = keys.public.try_into().unwrap();
            println!("Made up a key: public {}", key_to_hex(&public));
            keys.private.try_into().unwrap()
        }
    };

    let started = Instant::now();
    let stream = TcpStream::connect(address).expect("connect");
    stream.set_nodelay(true).unwrap();
    let mut link = match NoiseLink::connect(stream, Box::new(DefaultResolver), &private, &server) {
        Ok(link) => link,
        Err(e) => {
            println!("Handshake failed: {}", e);
            return;
        }
    };
    println!("Connected + handshake in {:?}", started.elapsed());
    for size in [0, 16, 1024, 4096, 60_000] {
        let bytes: Vec<u8> = (0..size).map(|i| i as u8).collect();
        let started = Instant::now();
        link.send(&Request::Echo(bytes.clone())).expect("send");
        let response: Response = match link.receive() {
            Ok(response) => response,
            Err(e) => {
                println!("No answer: {}", e);
                return;
            }
        };
        match response {
            Response::Echo(got) if got == bytes => {
                println!("Echo {:>6} B: {:?} ok", size, started.elapsed())
            }
            other => {
                println!("Echo {:>6} B: got {:?}", size, other);
                return;
            }
        }
    }
}
