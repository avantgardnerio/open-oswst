//! Which FEC buys the most dB? Sends voice packets through a simulated LoRa
//! link and counts how many arrive, for three packets that all take ~62 ms:
//!
//! - **today**: 26 bytes, SF7/125 kHz, CR 4/5, explicit header, radio CRC
//! - **A** (QMesh): RS + convolutional + interleaver, 72 bytes at SF7/250 kHz
//! - **B**: Reed-Solomon only, 72 bytes at SF7/250 kHz
//!
//! The SNR axis is the SNR a 125 kHz radio would see, so every column has
//! the same received power. The 250 kHz schemes see 3 dB less SNR: twice the
//! bandwidth lets in twice the noise. Their coding has to win that back first.
//!
//!     cd core && cargo run --release --example fec_sim [packets per point]

mod channel;
mod lora;

use channel::{Burst, Channel, Rng};
use lora::Format;
use open_oswst_core::fec::{self, CODED_BYTES, VOICE_BYTES};

const TODAY_FORMAT: Format = Format {
    explicit_header: true,
    crc: true,
};
/// Fixed length, so no header; the CRC-8 inside the FEC replaces the radio's
const CODED_FORMAT: Format = Format {
    explicit_header: false,
    crc: false,
};

struct Scheme {
    name: &'static str,
    bandwidth_hz: f64,
    send: fn(&[u8; VOICE_BYTES], &mut Channel) -> Outcome,
}

const SCHEMES: [Scheme; 3] = [
    Scheme {
        name: "today",
        bandwidth_hz: 125e3,
        send: send_today,
    },
    Scheme {
        name: "A: RS+conv",
        bandwidth_hz: 250e3,
        send: send_a,
    },
    Scheme {
        name: "B: RS only",
        bandwidth_hz: 250e3,
        send: send_b,
    },
];

/// A guess, not a measurement: noise bursts 10 dB up, 10 ms long, every
/// 150 ms on average. Short enough to land inside one packet, which is the
/// case FEC can do something about (the field's multi-packet dropouts are a
/// whole packet below threshold: that's the white-noise table's job)
const BURSTS: Burst = Burst {
    mean_gap_ms: 150.0,
    mean_len_ms: 10.0,
    extra_noise_db: 10.0,
};

#[derive(PartialEq)]
enum Outcome {
    Good,
    /// Nothing reached the codec
    Lost,
    /// Wrong bytes reached the codec: the CRC missed it
    Wrong,
}

fn main() {
    let packets: usize = std::env::args()
        .nth(1)
        .map(|n| n.parse().expect("packets per point"))
        .unwrap_or(2000);

    println!("Airtime per voice packet (one every 160 ms):");
    println!(
        "  today {:.1} ms, A and B {:.1} ms",
        lora::airtime_ms(VOICE_BYTES, &TODAY_FORMAT, 125e3),
        lora::airtime_ms(CODED_BYTES, &CODED_FORMAT, 250e3)
    );
    println!("{packets} packets per point. SNR = what a 125 kHz radio would see.\n");

    println!("== White noise ==");
    sweep(packets, None);
    println!(
        "\n== Noise bursts: +{} dB, {} ms long, every {} ms on average ==",
        BURSTS.extra_noise_db, BURSTS.mean_len_ms, BURSTS.mean_gap_ms
    );
    sweep(packets, Some(&BURSTS));
}

/// Packet loss at each SNR, then where each scheme crosses 10% and 1% loss
fn sweep(packets: usize, burst: Option<&Burst>) {
    let snrs: Vec<f64> = (0..=32).map(|i| -18.0 + 0.5 * i as f64).collect();
    let mut loss = vec![Vec::new(); SCHEMES.len()];

    print!("{:>6}", "SNR");
    for scheme in &SCHEMES {
        print!("{:>14}", scheme.name);
    }
    println!("   (% lost; +N = wrong voice delivered)");

    let mut voices = Rng(0x5EED);
    for (point, &snr) in snrs.iter().enumerate() {
        print!("{snr:>6.1}");
        for (s, scheme) in SCHEMES.iter().enumerate() {
            let channel_snr = snr - 10.0 * (scheme.bandwidth_hz / 125e3).log10();
            let symbol_ms = lora::symbol_ms(scheme.bandwidth_hz);
            let seed = (point * 10 + s) as u64 + 1;
            let mut channel = Channel::new(seed, lora::SF as u32, channel_snr, symbol_ms, burst);

            let (mut lost, mut wrong) = (0, 0);
            for _ in 0..packets {
                let voice: [u8; VOICE_BYTES] = std::array::from_fn(|_| voices.next() as u8);
                match (scheme.send)(&voice, &mut channel) {
                    Outcome::Good => {}
                    Outcome::Lost => lost += 1,
                    Outcome::Wrong => wrong += 1,
                }
            }
            let rate = (lost + wrong) as f64 / packets as f64;
            loss[s].push(rate);
            let wrong = if wrong > 0 {
                format!("+{wrong}")
            } else {
                String::new()
            };
            print!("{:>9.1}{wrong:<5}", rate * 100.0);
        }
        println!();
    }

    for target in [0.10, 0.01] {
        println!("\nSNR for {:.0}% loss:", target * 100.0);
        let today = crossing(&snrs, &loss[0], target);
        for (s, scheme) in SCHEMES.iter().enumerate() {
            match crossing(&snrs, &loss[s], target) {
                Some(at) => {
                    let gain = today.map(|t| format!("  {:+.1} dB vs today", t - at));
                    println!(
                        "  {:<12}{at:>6.1} dB{}",
                        scheme.name,
                        gain.unwrap_or_default()
                    );
                }
                None => println!("  {:<12}  not reached in this range", scheme.name),
            }
        }
    }
}

/// The SNR where the loss rate falls through `target`, interpolated
fn crossing(snrs: &[f64], loss: &[f64], target: f64) -> Option<f64> {
    (1..snrs.len()).find_map(|i| {
        let (above, below) = (loss[i - 1], loss[i]);
        (above >= target && below < target)
            .then(|| snrs[i - 1] + (snrs[i] - snrs[i - 1]) * (above - target) / (above - below))
    })
}

fn send_today(voice: &[u8; VOICE_BYTES], channel: &mut Channel) -> Outcome {
    judge(
        voice,
        lora::send(voice, &TODAY_FORMAT, channel).map(|v| v.try_into().unwrap()),
    )
}

fn send_a(voice: &[u8; VOICE_BYTES], channel: &mut Channel) -> Outcome {
    let received = lora::send(&fec::encode_a(voice), &CODED_FORMAT, channel);
    judge(
        voice,
        received.and_then(|r| fec::decode_a(&r.try_into().unwrap())),
    )
}

fn send_b(voice: &[u8; VOICE_BYTES], channel: &mut Channel) -> Outcome {
    let received = lora::send(&fec::encode_b(voice), &CODED_FORMAT, channel);
    judge(
        voice,
        received.and_then(|r| fec::decode_b(&r.try_into().unwrap())),
    )
}

fn judge(sent: &[u8; VOICE_BYTES], received: Option<[u8; VOICE_BYTES]>) -> Outcome {
    match received {
        None => Outcome::Lost,
        Some(r) if r == *sent => Outcome::Good,
        Some(_) => Outcome::Wrong,
    }
}
