use crate::devices::gps::{Fix, Gps};
use crate::devices::knob::{Event as Knob, Knob as _};
use crate::devices::mic::Mic;
use crate::devices::network::Network;
use crate::devices::ptt::Ptt;
use crate::devices::radio::{Listen, RxPacket, TxRequest, LISTEN, RX_CHAN, TX_CHAN};
use crate::devices::screen::Screen;
use crate::devices::speaker::{self, MAX_VOLUME, SPK_FRAMES, SPK_REQ};
use crate::logger;
use crate::platform::Platform;
use crate::playback_timing::PlaybackTiming;
use core::fmt::Write as _;
use embassy_futures::join::join;
use embassy_futures::select::{select, select5, Either, Either5};
use embassy_time::{Ticker, Timer};
use embedded_graphics::mono_font::ascii::FONT_6X10;
use embedded_graphics::mono_font::{MonoTextStyle, MonoTextStyleBuilder};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::{Baseline, Text};
use std::future::Future;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::air;
use crate::codec::{
    CodecRequest, CodecResponse, CODEC2_FRAME_SAMPLES, CODEC_REPLY, FRAMES_PER_PACKET,
    HEADER_BYTES, PACKET_BYTES, PAYLOAD_BYTES, STEREO_PACKET_SAMPLES,
};
use crate::config;
use crate::echo::{self, Recorder};
use crate::menu::{Menu, Outcome, Setting};
use crate::mode::{self, Mode};
use crate::packet::{self, Header, Ident, PacketType, NAME_BYTES};
use crate::rx_buffer::{Next, RxBuffer, Verdict};
use crate::screen_text::{self, Activity, Heard};
use crate::utc;

/// Each packet's time to encode: from the end of its capture to the start of
/// its bin. Codec2 took up to ~110 ms per packet on the walk of 2026-10-05;
/// this leaves ~40 ms for a slow one (the "TX bins" line logs the slowest
/// encode and the least room left). Under 160 ms, or a late send would hold
/// up the next capture. Bin 0 (the wake-up packet) comes this long after the
/// PTT press, so packet n's bin, 2(n + 1), starts this long after its capture
/// ends
const ENCODE_DEADLINE: embassy_time::Duration = embassy_time::Duration::from_millis(150);

/// Stereo samples per Codec2 frame (320 mono × 2 channels)
const STEREO_FRAME_SAMPLES: usize = CODEC2_FRAME_SAMPLES * 2;

/// How often `housekeeping()` runs
const HOUSEKEEPING_PERIOD: embassy_time::Duration = embassy_time::Duration::from_millis(250);
/// No packet from the current talker for this long: they're gone. And with
/// nothing heard or sent for this long, the radio may sweep again
const RX_TIMEOUT: Duration = Duration::from_millis(500);
/// Air quiet this long before log lines are written to flash
const LOG_FLUSH_IDLE: Duration = Duration::from_secs(3);
/// How often the GPS fix goes in the log (also whenever it's gained or
/// lost). Walking, 30s is ~40m: enough to place a transmission
const GPS_LOG_PERIOD: Duration = Duration::from_secs(30);

/// What woke the app up
// Rx is ~260B bigger than the rest, but an event lives only until it's matched,
// so boxing it would cost a heap alloc per packet for nothing.
#[allow(clippy::large_enum_variant)]
enum AppEvent {
    Rx(RxPacket),
    Ptt,
    Speaker, // the speaker wants its next frame
    Knob(Knob),
    Tick, // time for housekeeping()
}

/// The already-started devices the app runs on
pub struct Devices<P: Platform> {
    pub mic: P::Mic,
    pub ptt: P::Ptt,
    pub knob: P::Knob,
    pub screen: Screen,
    pub gps: P::Gps,
    pub settings: Option<P::Settings>, // None if they couldn't be opened
}

pub async fn init<P: Platform>(
    devices: Devices<P>,
    mac_str: heapless::String<18>,
    codec_tx: SyncSender<CodecRequest>,
) -> impl Future<Output = ()> {
    log::info!("MAC: {}", mac_str);

    // One packet of encoded silence, for echo mode to fill gaps with
    let silent_pcm = vec![0i16; FRAMES_PER_PACKET * CODEC2_FRAME_SAMPLES].into_boxed_slice();
    codec_tx
        .send(CodecRequest::encode([0; 2], silent_pcm))
        .unwrap();
    let mut silence = [0u8; PAYLOAD_BYTES];
    if let CodecResponse::Encoded { packet } = CODEC_REPLY.receive().await {
        silence.copy_from_slice(&packet[HEADER_BYTES..]);
    }

    let app = App::<P> {
        devices,
        codec_tx,
        display: Display {
            style: MonoTextStyleBuilder::new()
                .font(&FONT_6X10)
                .text_color(BinaryColor::On)
                .build(),
            name: own_name(&mac_str[mac_str.len() - 8..]),
            // Last 3 bytes are enough to tell our boards apart
            short_mac: heapless::String::try_from(&mac_str[mac_str.len() - 8..]).unwrap(),
            heard: None,
            shown_fix: None,
            shown_network: Network::Off,
            shown_time: (0, 0),
            shown_activity: Activity::Idle,
        },
        rx: Receiving {
            buffer: RxBuffer::default(),
            last_heard: Instant::now(),
            // 3KB: on the heap, once. In the App it overflowed the main task's stack
            playback: Box::default(),
            own_txid: None,
            relayed_wake: None,
        },
        sounds: Sounds {
            silence: vec![0i16; STEREO_PACKET_SAMPLES].into(),
            squelch: generate_squelch(P::random),
        },
        echo: Recorder::new(silence),
        locked: false,
        logs: LogTimes {
            last_activity: Instant::now(),
            last_gps_log: None,
        },
        swept_after: None,
    };

    async move {
        let mut app = app;
        app.run().await;
    }
}

struct App<P: Platform> {
    devices: Devices<P>,
    codec_tx: SyncSender<CodecRequest>,
    display: Display,
    rx: Receiving,
    sounds: Sounds,
    echo: Recorder, // only used in echo mode
    locked: bool,   // PTT ignored; the menu still opens, so it can be unlocked
    logs: LogTimes,
    swept_after: Option<Instant>, // the quiet we last told the radio to sweep in
}

/// What the screen needs to draw, and what it last showed
struct Display {
    style: MonoTextStyle<'static, BinaryColor>,
    name: heapless::String<NAME_BYTES>, // ours: config's name, else the short MAC
    short_mac: heapless::String<8>,     // e.g. A2:C6:2C
    heard: Option<Heard>,               // the last transmission heard to its end
    shown_fix: Option<Fix>,             // what the screen shows, to redraw when it changes
    shown_network: Network,             // likewise
    shown_time: (u8, u8),               // likewise: the clock's hh:mm
    shown_activity: Activity,           // likewise
}

/// The transmission we're hearing
struct Receiving {
    buffer: RxBuffer<Arc<[i16]>>,  // the talker we're hearing, in seq order
    last_heard: Instant,           // their last packet, for RX_TIMEOUT
    playback: Box<PlaybackTiming>, // per received transmission, logged at its end
    own_txid: Option<u8>,          // our last transmission's, so we ignore it relayed back
    relayed_wake: Option<u8>, // repeater: the last wake-up relayed (its txid), to relay each once
}

/// Audio built once at boot, played as-is
struct Sounds {
    silence: Arc<[i16]>, // fills a missing packet
    squelch: Arc<[i16]>, // tail after a received EOT
}

/// When logs were last due
struct LogTimes {
    last_activity: Instant, // last packet heard or sent: gates log flushes
    last_gps_log: Option<(Instant, bool)>, // when, and whether it had a position
}

impl<P: Platform> App<P> {
    async fn run(&mut self) {
        self.draw_screen(Activity::Idle);

        let mut ticker = Ticker::every(HOUSEKEEPING_PERIOD);
        loop {
            match self.next_event(&mut ticker).await {
                AppEvent::Rx(rx_pkt) => self.on_rx_packet(rx_pkt).await,
                AppEvent::Ptt => self.on_ptt().await,
                AppEvent::Speaker => self.on_speaker_request(),
                AppEvent::Knob(Knob::Click) => self.run_menu().await,
                AppEvent::Knob(Knob::Cw) => self.change_volume(1),
                AppEvent::Knob(Knob::Ccw) => self.change_volume(-1),
                AppEvent::Tick => self.housekeeping().await,
            }
        }
    }

    /// Wait for whatever happens first. Only things that need a fast response
    /// are events; anything that just needs checking now and then goes in
    /// `housekeeping()`, run on the tick.
    async fn next_event(&mut self, ticker: &mut Ticker) -> AppEvent {
        let locked = self.locked;
        let ptt = &mut self.devices.ptt;
        let ptt = async {
            if locked {
                core::future::pending::<()>().await;
            }
            ptt.pressed().await;
        };
        match select5(
            RX_CHAN.receive(),
            ptt,
            SPK_REQ.receive(),
            self.devices.knob.next(),
            ticker.next(),
        )
        .await
        {
            Either5::First(rx_pkt) => AppEvent::Rx(rx_pkt),
            Either5::Second(()) => AppEvent::Ptt,
            Either5::Third(()) => AppEvent::Speaker,
            Either5::Fourth(knob) => AppEvent::Knob(knob),
            Either5::Fifth(()) => AppEvent::Tick,
        }
    }

    /// Timed jobs, checked every tick. TX and echo replay run to completion
    /// inside their handlers, so none of this ever runs while transmitting.
    async fn housekeeping(&mut self) {
        // RX went quiet without an EOT (lost, or the talker went out of range).
        // No squelch tail here: at the edge this fires mid-transmission, and a
        // tail in the middle of broken-up audio sounds like the talker let go.
        // Only a received EOT plays the tail.
        if self.rx.buffer.txid().is_some() && self.rx.last_heard.elapsed() > RX_TIMEOUT {
            log::info!("RX timeout, resetting txid lock");
            log_worst_alloc();
            self.rx.playback.log_and_reset();
            self.rx.buffer.end();
            self.draw_screen(Activity::Idle);
        }

        // Echo mode: the talker went quiet without an EOT. Replay anyway
        if self.echo.timed_out() {
            self.replay_echo().await;
        }

        // Nothing heard or sent for a while: the radio may sweep again (if
        // it sweeps at all). Once per quiet spell
        let quiet_since = self.logs.last_activity;
        if quiet_since.elapsed() > RX_TIMEOUT && self.swept_after != Some(quiet_since) {
            LISTEN.signal(Listen::Sweep);
            self.swept_after = Some(quiet_since);
        }

        self.log_gps();

        // Idle: keep the clock and position on screen current
        let receiving = self.rx.buffer.txid().is_some() || self.echo.txid().is_some();
        if !receiving
            && (self.devices.gps.latest() != self.display.shown_fix
                || P::network() != self.display.shown_network
                || utc::now_hm() != self.display.shown_time)
        {
            self.draw_screen(Activity::Idle);
        }

        // Flash writes stall the chip, so logs only go to the file once the
        // air has been quiet a while, and one chunk per tick. Between chunks,
        // any packet or PTT press gets handled first.
        // Never on a repeater: a packet can arrive at any moment, and one
        // landing during the stall misses its relay slot.
        let repeater = mode::get() == Mode::Repeater;
        if !repeater
            && !receiving
            && self.logs.last_activity.elapsed() > LOG_FLUSH_IDLE
            && logger::pending()
        {
            logger::flush_chunk();
        }
    }

    /// Log the fix every GPS_LOG_PERIOD, and straight away when a position
    /// is gained or lost. These lines tie the log to real time and place.
    fn log_gps(&mut self) {
        let fix = self.devices.gps.latest();
        let has_position = fix.is_some_and(|fix| fix.position.is_some());
        let due = match self.logs.last_gps_log {
            None => true,
            Some((at, had_position)) => {
                at.elapsed() >= GPS_LOG_PERIOD || had_position != has_position
            }
        };
        if due {
            match fix {
                Some(fix) => log::info!("GPS {}", fix),
                None => log::info!("GPS not responding"),
            }
            self.logs.last_gps_log = Some((Instant::now(), has_position));
        }
    }

    async fn on_rx_packet(&mut self, rx_pkt: RxPacket) {
        self.logs.last_activity = Instant::now();
        // A failed CRC: the bytes are garbage, even txid and seq, so nothing
        // below may see them. It still counts as activity (above): it was on
        // the air, and the radio may have stayed locked on it, so the air
        // isn't quiet and the Sweep waits
        if !rx_pkt.crc_ok {
            return;
        }
        if rx_pkt.data.len() < HEADER_BYTES {
            log::warn!(
                "RX [{}B] too short, rssi={} snr={}",
                rx_pkt.data.len(),
                rx_pkt.rssi,
                rx_pkt.snr
            );
            return;
        }

        let Header {
            pkt_type,
            txid,
            seq,
        } = packet::unpack([rx_pkt.data[0], rx_pkt.data[1]]);

        let pkt_type = match pkt_type {
            Ok(pkt_type) => pkt_type,
            Err(bits) => {
                log::warn!(
                    "RX unknown pkt_type={} raw=[0x{:02X},0x{:02X}]",
                    bits,
                    rx_pkt.data[0],
                    rx_pkt.data[1]
                );
                return;
            }
        };

        // Only wakes up radios that are sweeping: nothing to play. A repeater
        // passes it on (once), so radios that only hear the repeater find
        // the transmission on the channel it relays on
        if pkt_type == PacketType::Wake {
            log::info!(
                "RX wake txid={} rssi={} snr={} ch={}",
                txid,
                rx_pkt.rssi,
                rx_pkt.snr,
                rx_pkt.channel
            );
            let ours = Some(txid) == self.rx.own_txid;
            if mode::get() == Mode::Repeater && !ours && self.rx.relayed_wake != Some(txid) {
                self.rx.relayed_wake = Some(txid);
                let channel = relay_channel(rx_pkt.channel);
                let mut relay = heapless::Vec::new();
                let _ = relay.extend_from_slice(&rx_pkt.data);
                TX_CHAN
                    .send(TxRequest {
                        data: relay,
                        preamble: Some(air::wake_preamble_symbols()),
                        channel,
                        clear_air_first: true,
                    })
                    .await;
                log::info!("RELAY wake txid={} ch={}", txid, channel);
            }
            return;
        }

        // Another radio's timing run (src/bin/radio_timing.rs)
        if pkt_type == PacketType::Bench {
            return;
        }

        // Our own transmission, relayed back by a repeater
        if Some(txid) == self.rx.own_txid {
            log::info!(
                "RX txid={} seq={} is our own, relayed back: dropping",
                txid,
                seq
            );
            return;
        }

        if matches!(pkt_type, PacketType::VoiceEnd | PacketType::EchoEnd) {
            self.on_end(&rx_pkt, pkt_type, txid).await;
            return;
        }

        // Echo mode records live voice instead of playing it. Echoes are never
        // echoed, so they fall through and play like voice.
        if mode::get() == Mode::Echo && pkt_type == PacketType::Voice {
            self.on_echo_packet(&rx_pkt, txid, seq).await;
            return;
        }

        if rx_pkt.data.len() != PACKET_BYTES {
            log::warn!("RX [{}B] unexpected size, ignoring", rx_pkt.data.len());
            return;
        }

        let payload = &rx_pkt.data[HEADER_BYTES..];

        let verdict = self.rx.buffer.check(txid, seq);
        if let Verdict::OtherTxid { locked } = verdict {
            log::warn!("RX ignoring txid={} (locked to {})", txid, locked);
            return;
        }
        self.rx.last_heard = Instant::now();
        match verdict {
            Verdict::Old(diff) => {
                log::info!("RX seq={} old (diff={}), dropping", seq, diff);
                return;
            }
            Verdict::Duplicate => {
                log::info!("RX seq={} duplicate, dropping", seq);
                return;
            }
            Verdict::Corrupt(diff) => {
                log::warn!(
                    "RX seq={} impossible (diff={}), dropping as corrupt",
                    seq,
                    diff
                );
                return;
            }
            Verdict::Resync(diff) => {
                log::warn!("RX seq={} impossible again (diff={}), resyncing", seq, diff);
                return;
            }
            Verdict::OtherTxid { .. } | Verdict::Take => {}
        }

        // Repeater: relay after dedup (non-duplicate voice)
        if mode::get() == Mode::Repeater {
            let channel = relay_channel(rx_pkt.channel);
            let mut relay = heapless::Vec::new();
            let _ = relay.extend_from_slice(&rx_pkt.data);
            TX_CHAN
                .send(TxRequest {
                    data: relay,
                    preamble: None,
                    channel,
                    clear_air_first: true,
                })
                .await;
            log::info!(
                "RELAY [{}B] txid={} seq={} ch={}",
                rx_pkt.data.len(),
                txid,
                seq,
                channel
            );
            self.rx.buffer.relayed(seq);
            // Drawn after the relay is queued, so it doesn't delay it
            self.show(Activity::Repeating);
            return; // skip decode — fast turnaround
        }
        // Send to codec thread for decode, await reply
        let mut payload_arr = [0u8; PAYLOAD_BYTES];
        payload_arr.copy_from_slice(payload);
        let asked = Instant::now();
        self.codec_tx
            .send(CodecRequest::decode(seq, txid, payload_arr))
            .unwrap();
        if let CodecResponse::Decoded {
            seq,
            txid,
            pcm,
            decode_us,
        } = CODEC_REPLY.receive().await
        {
            let waited_us = asked.elapsed().as_micros() as u32;
            self.rx
                .playback
                .record(decode_us, waited_us, SPK_FRAMES.len());
            // The speaker starts once two in a row are here
            if let Some(first) = self.rx.buffer.insert(txid, seq, timed_alloc(|| pcm.into())) {
                send_to_speaker(&first);
            }
        }

        log::info!(
            "RX [{}B] txid={} seq={} played_to={} rssi={} snr={}",
            rx_pkt.data.len(),
            txid,
            seq,
            self.rx.buffer.last_played(),
            rx_pkt.rssi,
            rx_pkt.snr,
        );

        self.show(Activity::Receiving);
    }

    /// The end of a transmission: who sent it and where they were. A
    /// repeater passes it on; an echo station replays what it recorded;
    /// everyone else plays the squelch tail (a repeater never plays the
    /// voice, so a tail on its own would be noise). The screen shows who it
    /// was from the first copy heard (direct before relayed)
    async fn on_end(&mut self, rx_pkt: &RxPacket, pkt_type: PacketType, txid: u8) {
        let ident = packet::read_end(&rx_pkt.data);
        match &ident {
            Some(Ident {
                name,
                position: Some((lat, lon)),
            }) => log::info!(
                "RX EOT from txid={} name={:?} at {:.5},{:.5} ch={}",
                txid,
                name.as_str(),
                lat,
                lon,
                rx_pkt.channel
            ),
            Some(Ident {
                name,
                position: None,
            }) => log::info!(
                "RX EOT from txid={} name={:?} no-fix ch={}",
                txid,
                name.as_str(),
                rx_pkt.channel
            ),
            None => log::warn!("RX EOT from txid={}: unreadable ident", txid),
        }

        let echo_this = mode::get() == Mode::Echo && pkt_type == PacketType::VoiceEnd;
        if mode::get() == Mode::Repeater {
            let channel = relay_channel(rx_pkt.channel);
            let mut relay = heapless::Vec::new();
            let _ = relay.extend_from_slice(&rx_pkt.data);
            TX_CHAN
                .send(TxRequest {
                    data: relay,
                    preamble: None,
                    channel,
                    clear_air_first: true,
                })
                .await;
            log::info!("RELAY EOT txid={} ch={}", txid, channel);
        } else if !echo_this {
            send_to_speaker(&self.sounds.squelch);
        }
        log_worst_alloc();
        self.rx.playback.log_and_reset();
        self.rx.buffer.end();

        let first_copy = self.display.heard.as_ref().map(|heard| heard.txid) != Some(txid);
        if let (Some(ident), true) = (ident, first_copy) {
            self.display.heard = Some(Heard {
                txid,
                ident,
                rssi: rx_pkt.rssi,
                relayed: rx_pkt.channel != 0,
                // The system clock (NTP, else the GPS): it has the time
                // without a fix, or any GPS at all
                at: Some(utc::now_hm()),
            });
        }
        self.draw_screen(Activity::Idle);

        if echo_this && self.echo.txid() == Some(txid) {
            self.replay_echo().await;
        }
    }

    /// Echo mode: record voice packets; on_end replays them once the
    /// talker's end packet arrives. (If it's lost, housekeeping() replays on
    /// a timeout.)
    async fn on_echo_packet(&mut self, rx_pkt: &RxPacket, txid: u8, seq: u8) {
        if rx_pkt.data.len() != PACKET_BYTES {
            return;
        }
        // The second copy of a packet (direct and relayed) isn't stored
        if !self.echo.record(txid, seq, &rx_pkt.data[HEADER_BYTES..]) {
            return;
        }
        log::info!(
            "ECHO rec txid={} seq={} rssi={} snr={}",
            txid,
            seq,
            rx_pkt.rssi,
            rx_pkt.snr
        );
        self.show(Activity::Receiving);
    }

    async fn replay_echo(&mut self) {
        let packets = self.echo.take();
        LISTEN.signal(Listen::Hold);
        log::info!("ECHO replaying {} packets", packets.len());
        self.draw_screen(Activity::Transmitting);
        // Drop anything heard while we transmit it, as it comes: like on_ptt
        let txid = random_txid::<P>();
        self.rx.own_txid = Some(txid);
        // The replay ends with the echo station's own Ident: whoever hears it
        // learns how far away the station is
        let ident = self.ident();
        let end = |seq| packet::end(PacketType::EchoEnd, txid, seq, &ident);
        let replay = echo::replay(packets, txid, config::WAKE_PREAMBLE.is_on(), end);
        if let Either::Second(()) = select(replay, discard_rx()).await {
            unreachable!("discard_rx never returns");
        }
        self.logs.last_activity = Instant::now();
        self.draw_screen(Activity::Idle);
    }

    async fn on_ptt(&mut self) {
        // PTT pressed — reset RX state
        self.rx.buffer.end();
        LISTEN.signal(Listen::Hold);

        let txid = random_txid::<P>();
        self.rx.own_txid = Some(txid);
        log::info!("PTT pressed — streaming (txid={})", txid);

        self.draw_screen(Activity::Transmitting);

        self.devices.mic.drain(); // discard stale

        // Half-duplex: anything heard while we talk can't be played, so throw
        // it away as it comes. Left in the queue it fills up, and the radio
        // stalls handing over the next packet, unable to send ours.
        let packets = match select(self.stream(txid), discard_rx()).await {
            Either::First(packets) => packets,
            Either::Second(()) => unreachable!("discard_rx never returns"),
        };

        log::info!("PTT released — {} packets sent + EOT", packets);
        self.logs.last_activity = Instant::now();
        self.draw_screen(Activity::Idle);
    }

    /// Send voice while PTT is held, then our end packet. Returns the
    /// packets sent.
    async fn stream(&mut self, txid: u8) -> usize {
        // Every packet goes on the air at the start of its bin, not the
        // moment it's ready: the air is a conveyor of 80 ms bins, a talker
        // sends in the even ones and a repeater relays in the odd ones
        // between (air::bin_us). Codec2 takes a different time on every
        // packet; sent as each encode finished, packets landed up to 65 ms
        // out of their bins (walk of 2026-10-05), too far for a receiver to
        // tell them from someone else's. The echo replay keeps time the same
        // way. Pipelined: each pass captures packet n + 1 while packet n is
        // encoded and waits for its bin, so the mic is drained continuously
        let ptt_pressed_at = embassy_time::Instant::now();
        let bin_0 = ptt_pressed_at + ENCODE_DEADLINE;
        let bin_start =
            |bin: u64| bin_0 + embassy_time::Duration::from_micros(air::bin_us() as u64 * bin);
        let wake = config::WAKE_PREAMBLE.is_on();
        let mic = &mut self.devices.mic;
        let codec_tx = &self.codec_tx;
        let mut seq: u8 = 0;
        // TODO: pipeline per Codec2 frame, for latency: a packet is 4 frames
        // of 40 ms. Encode each frame as soon as it's captured, while the
        // next is captured, so a packet is ready one frame's encode after its
        // capture ends, not four, and ENCODE_DEADLINE can shrink
        let mut pending_pkt: Option<([u8; 2], Box<[i16]>)> = None;
        let mut packets: u64 = 0;
        let mut timing = SendTiming::default();
        while self.devices.ptt.is_pressed() {
            let header = packet::pack(PacketType::Voice, txid, seq);
            // Voice packet n goes in bin 2(n + 1); the wake-up packet in bin
            // 0. The first on the air (the wake-up packet, else voice packet
            // 0) waits for clear air; after that the bins are ours
            let (pcm, ()) = join(capture_packet(mic), async {
                match pending_pkt.take() {
                    Some((header, pcm)) => {
                        let first_on_air = packets == 1 && !wake;
                        let bin = bin_start(2 * packets);
                        encode_and_send(codec_tx, header, pcm, bin, first_on_air, &mut timing)
                            .await;
                    }
                    None if wake => {
                        Timer::at(bin_start(0)).await;
                        TX_CHAN.send(packet::wake(txid)).await;
                    }
                    None => {}
                }
            })
            .await;
            pending_pkt = Some((header, pcm));
            packets += 1;
            seq = (seq + 1) & 0x0F; // wrap at 16
        }
        if let Some((header, pcm)) = pending_pkt {
            let first_on_air = packets == 1 && !wake;
            let bin = bin_start(2 * packets);
            encode_and_send(codec_tx, header, pcm, bin, first_on_air, &mut timing).await;
        }

        // Who we are and where we are, as the transmission ends: in the next
        // even bin, like any packet. Sent straight after the last packet, it
        // was on the air while a repeater relayed that one, and the repeater
        // never heard it (relayed 1 of 16 EOTs, 3 desk runs 2026-10-04)
        Timer::at(bin_start(2 * (packets + 1))).await;
        let end = packet::end(PacketType::VoiceEnd, txid, seq, &self.ident());
        TX_CHAN
            .send(TxRequest {
                data: end,
                preamble: None,
                channel: 0,
                clear_air_first: false,
            })
            .await;
        log::info!("TX bins: {}", timing);
        packets as usize
    }

    /// Us, for our end packets: our name, and where we are now
    fn ident(&self) -> Ident {
        Ident {
            name: self.display.name.clone(),
            position: self.devices.gps.latest().and_then(|fix| fix.position),
        }
    }

    fn on_speaker_request(&mut self) {
        match self.rx.buffer.for_speaker() {
            Next::Audio(pcm) => send_to_speaker(&pcm),
            Next::Gap(seq) => {
                log::info!("SPK gap at seq={}, sending silence", seq);
                send_to_speaker(&self.sounds.silence);
            }
            // Not receiving, nothing to send — DMA auto_clear handles silence
            Next::Idle => {}
        }
    }

    fn change_volume(&mut self, delta: i8) {
        let level = speaker::volume()
            .saturating_add_signed(delta)
            .min(MAX_VOLUME);
        speaker::set_volume(level);
        log::info!("Volume {}", level);
        self.draw_screen(self.display.shown_activity);
    }

    /// Menu mode is only menuing: audio and RX stop until we leave, and PTT
    /// is ignored.
    async fn run_menu(&mut self) {
        self.rx.buffer.end();
        let mut menu = Menu::new();
        loop {
            self.draw_menu(&menu);
            match self.devices.knob.next().await {
                Knob::Cw => menu.rotate(1),
                Knob::Ccw => menu.rotate(-1),
                Knob::Click => match menu.click() {
                    Outcome::Stay => {}
                    Outcome::Exit => break,
                    Outcome::Set(setting, value) => self.apply(setting, value),
                },
            }
        }
        // Drop whatever arrived while we were menuing
        while RX_CHAN.try_receive().is_ok() {}
        let _ = SPK_REQ.try_receive();
        self.draw_screen(Activity::Idle);
    }

    fn apply(&mut self, setting: Setting, value: u8) {
        log::info!("Menu: {:?} = {}", setting, value);
        match setting {
            Setting::Lock => self.locked = value != 0,
            Setting::Mode => config::MODE.set(value as i32, self.devices.settings.as_mut()),
        }
    }

    fn setting(&self, setting: Setting) -> u8 {
        match setting {
            Setting::Lock => self.locked as u8,
            Setting::Mode => mode::get() as u8,
        }
    }

    /// Redraw only if what we're doing changed: per packet, that's once
    /// per transmission, not every 160ms
    fn show(&mut self, activity: Activity) {
        if activity != self.display.shown_activity {
            self.draw_screen(activity);
        }
    }

    /// The radio screen (screen_text has the layout): us, our position, who
    /// we last heard, WiFi, then volume, what we're doing, time and mode.
    /// FONT_6X10 rows, 21 characters each
    fn draw_screen(&mut self, activity: Activity) {
        const ROWS_Y: [i32; 6] = [9, 20, 31, 42, 52, 62];
        let fix = self.devices.gps.latest();
        let network = P::network();
        let mut rows: [screen_text::Row; 6] = Default::default();

        rows[0] = screen_text::us(&self.display.name, &self.display.short_mac);
        let _ = match fix {
            Some(Fix {
                position: Some((lat, lon)),
                ..
            }) => write!(rows[1], "{:.5},{:.5}", lat, lon),
            // Used / in view: in view climbs while it acquires, used stays
            // 0 until the fix (so it alone never showed progress)
            Some(fix) => write!(rows[1], "No fix, {}/{} sats", fix.satellites, fix.in_view()),
            None => write!(rows[1], "No GPS"),
        };
        if let Some(heard) = &self.display.heard {
            let our_position = fix.and_then(|fix| fix.position);
            (rows[2], rows[3]) = screen_text::heard(heard, our_position);
        }
        rows[4] = network.screen_line(screen_text::WIDTH);
        rows[5] = screen_text::status(
            speaker::volume(),
            self.locked,
            activity,
            Some(utc::now_hm()),
            mode::get().name(),
        );

        let mut frame = self.devices.screen.frame();
        for (row, y) in rows.iter().zip(ROWS_Y) {
            Text::new(row, Point::new(1, y), self.display.style)
                .draw(&mut frame)
                .unwrap();
        }
        self.devices.screen.show(frame);
        self.display.shown_fix = fix;
        self.display.shown_network = network;
        self.display.shown_time = utc::now_hm();
        self.display.shown_activity = activity;
    }

    /// Title, then up to 4 rows; the cursor row is inverted, current values get a *.
    fn draw_menu(&self, menu: &Menu) {
        const TOP: i32 = 13;
        const ROW_H: i32 = 11;
        const VISIBLE: usize = 4;

        let mut frame = self.devices.screen.frame();
        Text::new(menu.title(), Point::new(1, 9), self.display.style)
            .draw(&mut frame)
            .unwrap();

        let inverted = MonoTextStyleBuilder::new()
            .font(&FONT_6X10)
            .text_color(BinaryColor::Off)
            .build();
        let rows = menu.rows(|setting, value| self.setting(setting) == value);
        let first = menu.cursor().saturating_sub(VISIBLE - 1);
        for (i, (label, current)) in rows.iter().enumerate().skip(first).take(VISIBLE) {
            let y = TOP + (i - first) as i32 * ROW_H;
            let style = if i == menu.cursor() {
                Rectangle::new(Point::new(0, y), Size::new(128, ROW_H as u32))
                    .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
                    .draw(&mut frame)
                    .unwrap();
                inverted
            } else {
                self.display.style
            };
            Text::with_baseline(label, Point::new(4, y + 1), style, Baseline::Top)
                .draw(&mut frame)
                .unwrap();
            if *current {
                Text::with_baseline("*", Point::new(118, y + 1), style, Baseline::Top)
                    .draw(&mut frame)
                    .unwrap();
            }
        }
        self.devices.screen.show(frame);
    }
}

/// Generate 160ms squelch tail (white noise with fade-out), packet-sized.
fn generate_squelch(random: fn() -> u32) -> Arc<[i16]> {
    const MONO_SAMPLES: usize = FRAMES_PER_PACKET * CODEC2_FRAME_SAMPLES; // 1280
    const AMPLITUDE: i32 = 8000;
    let mut buf = vec![0i16; STEREO_PACKET_SAMPLES].into_boxed_slice();
    for i in 0..MONO_SAMPLES {
        let fade = (MONO_SAMPLES - i) as i32 * AMPLITUDE / MONO_SAMPLES as i32;
        let noise = ((random() % (2 * fade as u32 + 1)) as i32 - fade) as i16;
        buf[i * 2] = noise;
        buf[i * 2 + 1] = noise;
    }
    buf.into()
}

/// Slowest heap allocation on the audio path this transmission, in µs.
/// Logged and reset when the transmission ends.
// TODO: borrow frames from a pool allocated at boot instead of allocating
// per frame, so the audio path never touches the heap.
static WORST_ALLOC_US: AtomicU32 = AtomicU32::new(0);

/// Allocate `make()`, recording how long the heap took.
fn timed_alloc<T>(make: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let made = make();
    WORST_ALLOC_US.fetch_max(started.elapsed().as_micros() as u32, Ordering::Relaxed);
    made
}

fn log_worst_alloc() {
    let us = WORST_ALLOC_US.swap(0, Ordering::Relaxed);
    log::info!("Audio heap alloc: worst {}us this transmission", us);
}

/// Split a packet (4 × 40ms stereo frames) into individual frames and send to speaker.
fn send_to_speaker(packet: &[i16]) {
    for i in 0..FRAMES_PER_PACKET {
        let offset = i * STEREO_FRAME_SAMPLES;
        let frame: Arc<[i16]> =
            timed_alloc(|| packet[offset..offset + STEREO_FRAME_SAMPLES].into());
        if SPK_FRAMES.try_send(frame).is_err() {
            log::warn!("SPK queue full, dropped frame {} of packet", i);
        }
    }
}

/// Our name: the config's, else the short MAC
fn own_name(short_mac: &str) -> heapless::String<NAME_BYTES> {
    let name = config::NAME.get();
    if name.is_empty() {
        short_mac.try_into().unwrap_or_default()
    } else {
        name
    }
}

/// The channel a repeater relays on: the next hop channel after the one it
/// heard the packet on (wrapping at rx_hops), so a relay never lands on the
/// channel the packet came in on. Without the sweep flag every radio
/// listens on the start slot only, so the relay stays where it was heard
fn relay_channel(heard_on: u8) -> u8 {
    if !config::SWEEP.is_on() {
        return heard_on;
    }
    ((heard_on as u32 + 1) % config::RX_HOPS.get() as u32) as u8
}

/// Random 7-bit id for one transmission — the dedup key.
fn random_txid<P: Platform>() -> u8 {
    (P::random() & 0x7F) as u8
}

/// Receive and drop packets, forever: for while we're talking.
async fn discard_rx() {
    loop {
        let _ = RX_CHAN.receive().await;
    }
}

/// Capture one packet's worth of PCM (FRAMES_PER_PACKET frames, 160ms).
async fn capture_packet(mic: &mut impl Mic) -> Box<[i16]> {
    let mut pcm = vec![0i16; FRAMES_PER_PACKET * CODEC2_FRAME_SAMPLES].into_boxed_slice();
    for frame in pcm.chunks_mut(CODEC2_FRAME_SAMPLES) {
        mic.read(frame).await;
    }
    pcm
}

/// Encode one packet on the codec thread, then queue it for the radio at the
/// start of its bin (at once, if the encode ran past it).
async fn encode_and_send(
    codec_tx: &SyncSender<CodecRequest>,
    header: [u8; 2],
    pcm: Box<[i16]>,
    bin_start: embassy_time::Instant,
    first_on_air: bool,
    timing: &mut SendTiming,
) {
    let asked = embassy_time::Instant::now();
    codec_tx.send(CodecRequest::encode(header, pcm)).unwrap();
    if let CodecResponse::Encoded { packet } = CODEC_REPLY.receive().await {
        timing.note(asked, embassy_time::Instant::now(), bin_start);
        // TODO: hand the radio the bin's start with the packet and let it
        // send at that microsecond (timed TX, step 3 of #36). Waiting here, the
        // send lands as late as the app task and the radio's queue make it
        Timer::at(bin_start).await;
        TX_CHAN
            .send(TxRequest {
                data: packet,
                preamble: None,
                channel: 0,
                clear_air_first: first_on_air,
            })
            .await;
    }
}

/// How one transmission's packets made their bins, for a log line at its end
#[derive(Default)]
struct SendTiming {
    packets: u32,
    fastest_encode_us: u64,
    slowest_encode_us: u64,
    /// The least time any packet was ready before its bin started. Negative:
    /// it was late, and went out when ready
    least_room_us: i64,
    late: u32,
}

impl SendTiming {
    fn note(
        &mut self,
        asked: embassy_time::Instant,
        encoded: embassy_time::Instant,
        bin_start: embassy_time::Instant,
    ) {
        let encode_us = encoded.duration_since(asked).as_micros();
        let room_us = bin_start.as_micros() as i64 - encoded.as_micros() as i64;
        if self.packets == 0 {
            self.fastest_encode_us = encode_us;
            self.slowest_encode_us = encode_us;
            self.least_room_us = room_us;
        } else {
            self.fastest_encode_us = self.fastest_encode_us.min(encode_us);
            self.slowest_encode_us = self.slowest_encode_us.max(encode_us);
            self.least_room_us = self.least_room_us.min(room_us);
        }
        if room_us < 0 {
            self.late += 1;
            log::warn!(
                "TX packet ready {}ms after its bin started (encode {}ms)",
                -room_us / 1000,
                encode_us / 1000
            );
        }
        self.packets += 1;
    }
}

impl core::fmt::Display for SendTiming {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(
            f,
            "{} voice packets, encode {}..{}ms, least room before a bin {}ms, late {}",
            self.packets,
            self.fastest_encode_us / 1000,
            self.slowest_encode_us / 1000,
            self.least_room_us / 1000,
            self.late
        )
    }
}
