use crate::devices::gps::{Fix, Gps};
use crate::devices::knob::{Event as Knob, Knob as _};
use crate::devices::mic::Mic;
use crate::devices::network::Network;
use crate::devices::ptt::Ptt;
use crate::devices::radio::{Listen, RxPacket, TxRequest, LISTEN, RX_CHAN, TX_CHAN};
use crate::devices::screen::{Frame, Screen};
use crate::devices::speaker::{self, MAX_VOLUME, SPK_FRAMES, SPK_REQ};
use crate::logger;
use crate::platform::Platform;
use crate::playback_timing::PlaybackTiming;
use core::fmt::Write as _;
use embassy_futures::join::join;
use embassy_futures::select::{select, select5, Either, Either5};
use embassy_time::Ticker;
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

use crate::codec::{
    CodecRequest, CodecResponse, CODEC2_FRAME_SAMPLES, CODEC_REPLY, FRAMES_PER_PACKET,
    HEADER_BYTES, PACKET_BYTES, PAYLOAD_BYTES, STEREO_PACKET_SAMPLES,
};
use crate::config;
use crate::echo::{self, Recorder};
use crate::menu::{Menu, Outcome, Setting};
use crate::mode::{self, Mode};
use crate::packet::{self, Header, TYPE_ECHO, TYPE_VOICE, TYPE_WAKE};
use crate::rx_buffer::{Next, RxBuffer, Verdict};

/// Audio per packet: FRAMES_PER_PACKET × 40ms
const PACKET_MS: u128 = FRAMES_PER_PACKET as u128 * 40;

/// Stereo samples per Codec2 frame (320 mono × 2 channels)
const STEREO_FRAME_SAMPLES: usize = CODEC2_FRAME_SAMPLES * 2;

/// How often `housekeeping()` runs
const HOUSEKEEPING_PERIOD: embassy_time::Duration = embassy_time::Duration::from_millis(250);
/// No packet from the current talker for this long: they're gone. And with
/// nothing heard or sent for this long, the radio may sweep again
const RX_TIMEOUT: Duration = Duration::from_millis(500);
/// Air quiet this long before log lines are written to flash
const LOG_FLUSH_IDLE: Duration = Duration::from_secs(3);
/// How often the GPS fix goes in the log (also whenever it's gained or lost)
const GPS_LOG_PERIOD: Duration = Duration::from_secs(10);

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
            line_buf: heapless::String::new(),
            // Last 3 bytes are enough to tell our boards apart, and leave room for the time
            short_mac: heapless::String::try_from(&mac_str[mac_str.len() - 8..]).unwrap(),
            shown_fix: None,
            shown_network: Network::Off,
        },
        rx: Receiving {
            buffer: RxBuffer::default(),
            last_heard: Instant::now(),
            // 3KB: on the heap, once. In the App it overflowed the main task's stack
            playback: Box::default(),
            own_txid: None,
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
    line_buf: heapless::String<64>,
    short_mac: heapless::String<8>, // e.g. A2:C6:2C
    shown_fix: Option<Fix>,         // what the screen shows, to redraw when it changes
    shown_network: Network,         // likewise
}

/// The transmission we're hearing
struct Receiving {
    buffer: RxBuffer<Arc<[i16]>>,  // the talker we're hearing, in seq order
    last_heard: Instant,           // their last packet, for RX_TIMEOUT
    playback: Box<PlaybackTiming>, // per received transmission, logged at its end
    own_txid: Option<u8>,          // our last transmission's, so we ignore it relayed back
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
        // Show initial RX state
        self.draw_rx_screen();

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
            // A repeater's "Repeating" screen ends with the transmission
            if mode::get() == Mode::Repeater {
                self.draw_rx_screen();
            }
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
                || P::network() != self.display.shown_network)
        {
            self.draw_rx_screen();
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

        // Only wakes up radios that are sweeping. Nothing sweeps yet
        if pkt_type == TYPE_WAKE {
            log::info!(
                "RX wake txid={} rssi={} snr={}",
                txid,
                rx_pkt.rssi,
                rx_pkt.snr
            );
            return;
        }

        if pkt_type != TYPE_VOICE && pkt_type != TYPE_ECHO {
            log::warn!(
                "RX unknown pkt_type={} raw=[0x{:02X},0x{:02X}]",
                pkt_type,
                rx_pkt.data[0],
                rx_pkt.data[1]
            );
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

        // Echo mode records live voice instead of playing it. Echoes are never
        // echoed, so they fall through and play like voice.
        if mode::get() == Mode::Echo && pkt_type == TYPE_VOICE {
            self.on_echo_packet(&rx_pkt, txid, seq).await;
            return;
        }

        // Header-only = end of transmission — relay if repeater, else squelch.
        // A repeater never plays the voice, so a tail on its own would be noise.
        if rx_pkt.data.len() == HEADER_BYTES {
            if mode::get() == Mode::Repeater {
                let mut relay = heapless::Vec::new();
                let _ = relay.extend_from_slice(&rx_pkt.data);
                TX_CHAN
                    .send(TxRequest {
                        data: relay,
                        preamble: None,
                    })
                    .await;
                log::info!("RELAY EOT txid={}", txid);
            } else {
                send_to_speaker(&self.sounds.squelch);
            }
            log::info!("RX EOT from txid={}", txid);
            log_worst_alloc();
            self.rx.playback.log_and_reset();
            self.rx.buffer.end();
            if mode::get() == Mode::Repeater {
                self.draw_rx_screen();
            }
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
            let mut relay = heapless::Vec::new();
            let _ = relay.extend_from_slice(&rx_pkt.data);
            TX_CHAN
                .send(TxRequest {
                    data: relay,
                    preamble: None,
                })
                .await;
            log::info!("RELAY [{}B] txid={} seq={}", rx_pkt.data.len(), txid, seq);
            self.rx.buffer.relayed(seq);
            // Drawn after the relay is queued, so it doesn't delay it
            self.draw_relay_screen(rx_pkt.rssi, rx_pkt.snr);
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

        self.draw_rx_audio_screen(rx_pkt.rssi, rx_pkt.snr);
    }

    /// Echo mode: record voice packets, and replay once the talker's EOT
    /// arrives. (If the EOT is lost, housekeeping() replays on a timeout.)
    async fn on_echo_packet(&mut self, rx_pkt: &RxPacket, txid: u8, seq: u8) {
        if rx_pkt.data.len() == HEADER_BYTES {
            if self.echo.txid() == Some(txid) {
                self.replay_echo().await;
            }
            return;
        }
        if rx_pkt.data.len() != PACKET_BYTES {
            return;
        }
        self.echo.record(txid, seq, &rx_pkt.data[HEADER_BYTES..]);
        log::info!(
            "ECHO rec txid={} seq={} rssi={} snr={}",
            txid,
            seq,
            rx_pkt.rssi,
            rx_pkt.snr
        );
        self.draw_rx_audio_screen(rx_pkt.rssi, rx_pkt.snr);
    }

    async fn replay_echo(&mut self) {
        let packets = self.echo.take();
        LISTEN.signal(Listen::Hold);
        log::info!("ECHO replaying {} packets", packets.len());
        self.draw_tx_screen();
        // Drop anything heard while we transmit it, as it comes: like on_ptt
        let txid = random_txid::<P>();
        self.rx.own_txid = Some(txid);
        let replay = echo::replay(packets, txid, config::WAKE_PREAMBLE.is_on());
        if let Either::Second(()) = select(replay, discard_rx()).await {
            unreachable!("discard_rx never returns");
        }
        self.logs.last_activity = Instant::now();
        self.draw_rx_screen();
    }

    async fn on_ptt(&mut self) {
        // PTT pressed — reset RX state
        self.rx.buffer.end();
        LISTEN.signal(Listen::Hold);

        let txid = random_txid::<P>();
        self.rx.own_txid = Some(txid);
        log::info!("PTT pressed — streaming (txid={})", txid);

        self.draw_tx_screen();

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

        // Redraw RX screen
        self.draw_rx_screen();
    }

    /// Send voice while PTT is held, then an EOT. Returns the packets sent.
    async fn stream(&mut self, txid: u8) -> usize {
        // Pipelined: each pass captures packet N+1 while packet N is encoded and
        // sent, so the mic is drained continuously
        let mut seq: u8 = 0;
        let mic = &mut self.devices.mic;
        let codec_tx = &self.codec_tx;
        let mut pending: Option<([u8; 2], Box<[i16]>)> = None;
        let mut packets = 0usize;
        // Goes out while the first packet's audio is captured: no delay
        if config::WAKE_PREAMBLE.is_on() {
            TX_CHAN.send(packet::wake(txid)).await;
        }
        while self.devices.ptt.is_pressed() {
            let header = packet::pack(TYPE_VOICE, txid, seq);
            let (pcm, ()) = join(capture_packet(mic), async {
                if let Some((header, pcm)) = pending.take() {
                    encode_and_send(codec_tx, header, pcm).await;
                }
            })
            .await;
            pending = Some((header, pcm));
            packets += 1;
            seq = (seq + 1) & 0x0F; // wrap at 16
        }
        if let Some((header, pcm)) = pending {
            encode_and_send(codec_tx, header, pcm).await;
        }

        // Send header-only EOT packet
        let mut eot_data = heapless::Vec::new();
        let _ = eot_data.extend_from_slice(&packet::pack(TYPE_VOICE, txid, seq));
        TX_CHAN
            .send(TxRequest {
                data: eot_data,
                preamble: None,
            })
            .await;
        packets
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
        // Mid-reception the next packet redraws within 160ms; don't flash "Listening"
        if self.rx.buffer.txid().is_none() {
            self.draw_rx_screen();
        }
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
        self.draw_rx_screen();
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

    fn draw_rx_screen(&mut self) {
        let mut frame = self.devices.screen.frame();
        self.draw_header(&mut frame);
        Text::new("RX Listening", Point::new(28, 40), self.display.style)
            .draw(&mut frame)
            .unwrap();

        let network = P::network();
        let mut line = heapless::String::<40>::new();
        let _ = match &network {
            Network::Off => write!(line, "WiFi: off"),
            Network::Searching => write!(line, "WiFi: searching"),
            Network::Joined(ssid) => write!(line, "WiFi: {}", ssid),
        };
        Text::new(&line, Point::new(1, 51), self.display.style)
            .draw(&mut frame)
            .unwrap();
        self.display.shown_network = network;

        self.draw_status(&mut frame);
        self.devices.screen.show(frame);
    }

    /// Top two lines of every radio screen: short MAC and UTC time, then the
    /// GPS position (or why there isn't one).
    fn draw_header(&mut self, frame: &mut Frame) {
        let fix = self.devices.gps.latest();
        self.display.shown_fix = fix;

        let mut line = heapless::String::<24>::new();
        let _ = write!(line, "{}", self.display.short_mac);
        if let Some((h, m, s)) = fix.and_then(|fix| fix.time) {
            let _ = write!(line, "   {:02}:{:02}:{:02}Z", h, m, s);
        }
        Text::new(&line, Point::new(1, 10), self.display.style)
            .draw(frame)
            .unwrap();

        line.clear();
        let _ = match fix {
            Some(Fix {
                position: Some((lat, lon)),
                ..
            }) => write!(line, "{:.5},{:.5}", lat, lon),
            Some(fix) => write!(line, "No fix, {} sats", fix.satellites),
            None => write!(line, "No GPS"),
        };
        Text::new(&line, Point::new(1, 22), self.display.style)
            .draw(frame)
            .unwrap();
    }

    fn draw_rx_audio_screen(&mut self, rssi: i16, snr: i16) {
        self.draw_signal_screen("RX Audio", rssi, snr);
    }

    /// A repeater relaying: shows it's working, and how well it hears.
    fn draw_relay_screen(&mut self, rssi: i16, snr: i16) {
        self.draw_signal_screen("Repeating", rssi, snr);
    }

    /// A title, centred, with the last packet's RSSI and SNR under it.
    fn draw_signal_screen(&mut self, title: &str, rssi: i16, snr: i16) {
        let mut frame = self.devices.screen.frame();
        self.draw_header(&mut frame);
        // FONT_6X10: 6px per character on the 128px wide screen
        let x = (128 - 6 * title.len() as i32) / 2;
        Text::new(title, Point::new(x, 36), self.display.style)
            .draw(&mut frame)
            .unwrap();

        self.display.line_buf.clear();
        let _ = core::fmt::write(
            &mut self.display.line_buf,
            format_args!("RSSI:{} SNR:{}", rssi, snr),
        );
        Text::new(
            &self.display.line_buf,
            Point::new(0, 48),
            self.display.style,
        )
        .draw(&mut frame)
        .unwrap();

        self.draw_status(&mut frame);
        self.devices.screen.show(frame);
    }

    /// Bottom line of the RX screens: volume (and LOCKED when locked), and
    /// the mode on the right.
    fn draw_status(&self, frame: &mut Frame) {
        let mut left = heapless::String::<16>::new();
        let _ = write!(left, "Vol {}", speaker::volume());
        if self.locked {
            let _ = write!(left, " LOCKED");
        }
        Text::new(&left, Point::new(1, 62), self.display.style)
            .draw(frame)
            .unwrap();

        // The mode, right-aligned (6px per character)
        let mut mode = heapless::String::<8>::new();
        for c in mode::get().name().chars() {
            let _ = mode.push(c.to_ascii_uppercase());
        }
        let x = 128 - 6 * mode.len() as i32 - 1;
        Text::new(&mode, Point::new(x, 62), self.display.style)
            .draw(frame)
            .unwrap();
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

    fn draw_tx_screen(&mut self) {
        let mut frame = self.devices.screen.frame();
        self.draw_header(&mut frame);
        Text::new("TX Streaming", Point::new(28, 40), self.display.style)
            .draw(&mut frame)
            .unwrap();
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

/// Encode one packet on the codec thread and queue it for the radio.
async fn encode_and_send(codec_tx: &SyncSender<CodecRequest>, header: [u8; 2], pcm: Box<[i16]>) {
    let started = Instant::now();
    codec_tx.send(CodecRequest::encode(header, pcm)).unwrap();
    if let CodecResponse::Encoded { packet } = CODEC_REPLY.receive().await {
        TX_CHAN
            .send(TxRequest {
                data: packet,
                preamble: None,
            })
            .await;
    }
    // Longer than a packet's capture stalls the pipeline, so the mic goes undrained
    let ms = started.elapsed().as_millis();
    if ms > PACKET_MS {
        log::warn!(
            "TX encode+send took {}ms, longer than a {}ms packet",
            ms,
            PACKET_MS
        );
    }
}
