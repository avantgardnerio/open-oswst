use crate::devices::battery::Battery;
use crate::devices::gps::{Fix, Gps};
use crate::devices::knob::{Event as Knob, Knob as _};
use crate::devices::management::{self, Installing, Job};
use crate::devices::mic::Mic;
use crate::devices::network::Network;
use crate::devices::ptt::Ptt;
use crate::devices::radio::{Listen, Rotation, RxPacket, TxRequest, LISTEN, RX_CHAN, TX_CHAN};
use crate::devices::screen::{Screen, Snapshot};
use crate::devices::settings::Settings as _;
use crate::devices::speaker::{self, MAX_VOLUME, SPK_AUDIO};
use crate::devices::wifi;
use crate::logger;
use crate::platform::Platform;
use crate::playback_timing::PlaybackTiming;
use core::fmt::Write as _;
use embassy_futures::join::join;
use embassy_futures::select::{select, select3, select5, Either, Either3, Either5};
use embassy_time::{Ticker, Timer};
use embedded_graphics::mono_font::ascii::FONT_6X10;
use embedded_graphics::mono_font::{MonoTextStyle, MonoTextStyleBuilder};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::{Baseline, Text};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::air;
use crate::codec::{
    CodecRequest, Decoded, CODEC2_FRAME_SAMPLES, DECODED, ENCODED, FRAMES_PER_PACKET, HEADER_BYTES,
    PACKET_BYTES, PACKET_SAMPLES, PAYLOAD_BYTES,
};
use crate::config;
use crate::conveyor::{Arrival, Landing, Transmission};
use crate::double_click::DoubleClick;
use crate::echo::{self, Recorder};
use crate::management::{Offer, Response};
use crate::menu::{Action, Menu, Outcome, Page, Setting, BACK};
use crate::mode::{self, Mode};
use crate::packet::{self, Header, Ident, PacketType, NAME_BYTES};
use crate::password_entry::{Click, PasswordEntry};
use crate::playout::{Play, Playout};
use crate::scan_list::ScanList;
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

/// How long the menu's Update waits for the management server: the
/// firmware's own timeouts (connect, then the answer) come first
const UPDATE_TIMEOUT: embassy_time::Duration = embassy_time::Duration::from_secs(25);

/// An install with no news this long has stalled: the firmware's own
/// timeouts (10 s a chunk) should have said so first
const INSTALL_QUIET: embassy_time::Duration = embassy_time::Duration::from_secs(30);

/// How often the battery is read, while idle (never while we transmit: the
/// mic has the ADC then). Its last reading goes on each GPS log line
const BATTERY_EVERY: Duration = Duration::from_secs(60);

/// How often `housekeeping()` runs
const HOUSEKEEPING_PERIOD: embassy_time::Duration = embassy_time::Duration::from_millis(250);
/// Nothing heard or sent for this long, with no transmission on the conveyor
/// (whose end is the conveyor's to say: conveyor.rs): the radio may sweep again
const RX_TIMEOUT: Duration = Duration::from_millis(500);
/// After our own transmission, the start slot is held this long for a reply
/// before the radio may sweep: an echo station's replay arrives 1.25-1.65 s
/// after PTT release (walk of 2026-10-06), and a sweeping radio at the edge
/// lost it hopping between channels. Long enough for a person's answer to
/// begin too
const REPLY_WAIT: Duration = Duration::from_secs(3);
/// After our own transmission, our txid counts as ours this long: its copies
/// relayed back by repeaters arrive within a bin or two. After that it's
/// anyone's, so a talker who draws the same txid isn't ignored
const OWN_RELAYS_FOR: Duration = Duration::from_secs(2);
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
    PttReleased,      // after a refused press: the next press counts again
    Decoded(Decoded), // a packet's audio, back from the codec
    Knob(Knob),
    Tick, // time for housekeeping()
}

/// The already-started devices the app runs on
pub struct Devices<P: Platform> {
    pub mic: P::Mic,
    pub battery: P::Battery,
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
    let packet = ENCODED.receive().await;
    silence.copy_from_slice(&packet[HEADER_BYTES..]);

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
            shown_screen: Snapshot::default(),
            battery_mv: None,
            battery_read_at: None,
            shown_battery_cv: None,
        },
        rx: Receiving {
            playout: Playout::default(),
            playing_txid: None,
            transmission: None,
            // 3KB: on the heap, once. In the App it overflowed the main task's stack
            playback: Box::default(),
            own_txid: None,
            own_txid_until: Instant::now(),
            reply_until: Instant::now(),
            last_heard_txid: None,
            rotation: None,
        },
        sounds: Sounds {
            silence: vec![0i16; PACKET_SAMPLES].into(),
            squelch: generate_squelch(P::random),
        },
        echo: Recorder::new(silence),
        locked: false,
        menu_click: DoubleClick::default(),
        ptt_refused: false,
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
    echo: Recorder,          // only used in echo mode
    locked: bool,            // PTT ignored; the menu still opens, so it can be unlocked
    menu_click: DoubleClick, // the knob's double click that opens the menu
    ptt_refused: bool,       // a PTT press refused (someone else talking): ignored until let go
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
    shown_screen: Snapshot,             // the radio screen last drawn: what a screenshot saves
    battery_mv: Option<u32>,            // the last battery reading
    battery_read_at: Option<Instant>,   // when; None until the first
    shown_battery_cv: Option<u32>,      // what the screen shows, in hundredths of a volt
}

/// The transmission we're hearing
struct Receiving {
    playout: Playout<Arc<[i16]>>, // decoded audio on its way to the speaker, in order
    playing_txid: Option<u8>, // whose audio the playout is for: later decodes of others' are dropped
    transmission: Option<Transmission>, // the transmission on its conveyor: says when it's over
    playback: Box<PlaybackTiming>, // per received transmission, logged at its end
    own_txid: Option<u8>,     // our last transmission's, so we ignore it relayed back
    own_txid_until: Instant,  // until then: our relayed copies only come back for a moment
    reply_until: Instant,     // after our transmission: the start slot held for a reply until then
    last_heard_txid: Option<u8>, // the last transmission heard: a new one of ours never takes it
    rotation: Option<Rotation>, // the last rotation the radio was told to listen on, if it still is
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
                AppEvent::PttReleased => self.ptt_refused = false,
                AppEvent::Decoded(decoded) => self.on_decoded(decoded),
                // The menu opens on a quick double click, so a stray one
                // (a knob bumped in a pocket) can't
                AppEvent::Knob(Knob::Press) => self.menu_click.press(Instant::now()),
                AppEvent::Knob(Knob::Release) => {
                    if self.menu_click.release(Instant::now()) {
                        self.run_menu().await;
                    }
                }
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
        let refused = self.ptt_refused;
        let ptt = &mut self.devices.ptt;
        // The button reads as down for as long as it's held: after a refused
        // press, wait for it to come up, or every pass would refuse it again
        let ptt = async {
            if locked {
                core::future::pending::<()>().await;
            }
            if refused {
                ptt.released().await;
                return AppEvent::PttReleased;
            }
            ptt.pressed().await;
            AppEvent::Ptt
        };
        match select5(
            RX_CHAN.receive(),
            ptt,
            DECODED.receive(),
            self.devices.knob.next(),
            ticker.next(),
        )
        .await
        {
            Either5::First(rx_pkt) => AppEvent::Rx(rx_pkt),
            Either5::Second(event) => event,
            Either5::Third(decoded) => AppEvent::Decoded(decoded),
            Either5::Fourth(knob) => AppEvent::Knob(knob),
            Either5::Fifth(()) => AppEvent::Tick,
        }
    }

    /// Timed jobs, checked every tick. TX and echo replay run to completion
    /// inside their handlers, so none of this ever runs while transmitting.
    async fn housekeeping(&mut self) {
        // The transmission's conveyor has brought nothing for ~1 s and its end
        // packet never came (lost, or the talker went out of range). No
        // squelch tail here: at the edge this can come mid-transmission, and a
        // tail in the middle of broken-up audio sounds like the talker let go.
        // Only a received EOT plays the tail.
        let over = self
            .rx
            .transmission
            .as_ref()
            .is_some_and(|transmission| transmission.over(P::now_us()));
        if over {
            if let Some(transmission) = self.rx.transmission.take() {
                log::info!(
                    "CONVEYOR over without an EOT: txid={} {}",
                    transmission.txid,
                    transmission.tally()
                );
            }
            if self.rx.playing_txid.take().is_some() {
                log::info!("RX timeout, resetting txid lock");
                self.rx.playback.log_and_reset();
                self.rx.playout.reset();
                self.draw_screen(Activity::Idle);
            }
            // Echo mode: replay what was recorded anyway
            if self.echo.txid().is_some() {
                self.replay_echo().await;
            }
        }

        // Nothing heard or sent for a while, no transmission going, and no
        // reply to ours still due: the radio may sweep again (if it sweeps at
        // all). Once per quiet spell
        let quiet_since = self.logs.last_activity;
        if self.rx.transmission.is_none()
            && quiet_since.elapsed() > RX_TIMEOUT
            && Instant::now() >= self.rx.reply_until
            && self.swept_after != Some(quiet_since)
        {
            self.listen(Listen::Sweep);
            self.swept_after = Some(quiet_since);
        }

        self.read_battery().await;
        self.log_gps();

        // Idle: keep the clock, position and battery on screen current
        let receiving = self.rx.transmission.is_some() || self.echo.txid().is_some();
        if !receiving
            && (self.devices.gps.latest() != self.display.shown_fix
                || P::network() != self.display.shown_network
                || utc::now_hm() != self.display.shown_time
                || self.display.battery_mv.map(|mv| mv / 10) != self.display.shown_battery_cv)
        {
            self.draw_screen(Activity::Idle);
        }

        // Flash writes stall the chip, so logs only go to the file once the
        // air has been quiet a while, and one chunk per tick. Between chunks,
        // any packet or PTT press gets handled first.
        // Never with log_to_flash off (config::LOG_TO_FLASH): a repeater runs
        // that way, since a packet can arrive at any moment and one landing
        // during the stall misses its relay slot
        if config::LOG_TO_FLASH.is_on()
            && !receiving
            && self.logs.last_activity.elapsed() > LOG_FLUSH_IDLE
            && logger::pending()
        {
            logger::flush_chunk();
        }
    }

    /// Log the fix (and the battery) every GPS_LOG_PERIOD, and straight away when a position
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
            // The battery rides along: one line every GPS_LOG_PERIOD
            let battery = BatteryLog(self.display.battery_mv);
            match fix {
                Some(fix) => log::info!("GPS {} {}", fix, battery),
                None => log::info!("GPS not responding {}", battery),
            }
            self.logs.last_gps_log = Some((Instant::now(), has_position));
        }
    }

    async fn on_rx_packet(&mut self, rx_pkt: RxPacket) {
        self.logs.last_activity = Instant::now();
        // A failed CRC: the bytes are garbage, even txid and hops, so nothing
        // below may see them. It still counts as activity (above): it was on
        // the air, and the radio may have stayed locked on it, so the air
        // isn't quiet and the Sweep waits. On the conveyor it was part of the
        // transmission, so that keeps going
        if !rx_pkt.crc_ok {
            if let Some(transmission) = &mut self.rx.transmission {
                transmission.heard_garbled(rx_pkt.end_us, rx_pkt.timing_ok);
            }
            if config::PLAY_GARBLED.is_on() {
                self.play_garbled(&rx_pkt);
            }
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
            hops,
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

        // Where it landed on its transmission's conveyor (the first good
        // packet starts one). Not our own relayed back, nor a timing run
        let ours = Some(txid) == self.rx.own_txid && Instant::now() < self.rx.own_txid_until;
        let landing = if ours || pkt_type == PacketType::Bench {
            None
        } else {
            self.ride_conveyor(&rx_pkt, pkt_type, txid, hops)
        };
        self.listen_to_conveyor();

        // Only wakes up radios that are sweeping: nothing to play. A repeater
        // passes it on (its first copy: it's the transmission's packet 0), so
        // radios that only hear the repeater find the transmission on the
        // channel it relays on
        if pkt_type == PacketType::Wake {
            log::info!(
                "RX wake txid={} hops={} rssi={} snr={} ch={}",
                txid,
                hops,
                rx_pkt.rssi,
                rx_pkt.snr,
                rx_pkt.channel
            );
            if mode::get() == Mode::Repeater && self.take_first_copy(landing) {
                if let Some(channel) = relay(
                    &rx_pkt,
                    Some(air::wake_preamble_symbols()),
                    self.relay_channel(&rx_pkt, hops),
                )
                .await
                {
                    log::info!("RELAY wake txid={} hops={} ch={}", txid, hops + 1, channel);
                }
            }
            return;
        }

        // Another radio's timing run (src/bin/radio_timing.rs)
        if pkt_type == PacketType::Bench {
            return;
        }

        // Our own transmission, relayed back by a repeater
        if ours {
            log::info!(
                "RX txid={} hops={} is our own, relayed back: dropping",
                txid,
                hops
            );
            return;
        }

        if matches!(pkt_type, PacketType::VoiceEnd | PacketType::EchoEnd) {
            self.on_end(&rx_pkt, pkt_type, txid, hops, landing).await;
            return;
        }

        // Echo mode records live voice instead of playing it. Echoes are never
        // echoed, so they fall through and play like voice.
        if mode::get() == Mode::Echo && pkt_type == PacketType::Voice {
            self.on_echo_packet(&rx_pkt, txid, hops, landing).await;
            return;
        }

        if rx_pkt.data.len() != PACKET_BYTES {
            log::warn!("RX [{}B] unexpected size, ignoring", rx_pkt.data.len());
            return;
        }

        // Someone else's, while another transmission is on the conveyor
        let Some(landing) = landing else {
            let locked = self
                .rx
                .transmission
                .as_ref()
                .map(|transmission| transmission.txid);
            log::warn!("RX ignoring txid={} (locked to {:?})", txid, locked);
            return;
        };
        // A good packet whose bin doesn't fit its hops: its timing or its hops
        // are wrong, so it can't be numbered (the CONVEYOR tally counts these)
        let Some(packet) = landing.packet else {
            log::warn!(
                "RX txid={} hops={} in the wrong bin {} for its hops, dropping",
                txid,
                hops,
                landing.bin
            );
            return;
        };
        // The second copy of a packet (any hops, even round a loop of
        // repeaters), or an old one: already played or relayed
        if !self.take_first_copy(Some(landing)) {
            log::info!("RX packet={} hops={} duplicate, dropping", packet, hops);
            return;
        }

        // Repeater: relay the first copy of each packet
        if mode::get() == Mode::Repeater {
            if let Some(channel) = relay(&rx_pkt, None, self.relay_channel(&rx_pkt, hops)).await {
                log::info!(
                    "RELAY [{}B] txid={} packet={} hops={} ch={}",
                    rx_pkt.data.len(),
                    txid,
                    packet,
                    hops + 1,
                    channel
                );
            }
            // Drawn after the relay is queued, so it doesn't delay it
            self.show(Activity::Repeating);
            return; // skip decode — fast turnaround
        }

        self.decode(txid, packet, &rx_pkt.data[HEADER_BYTES..]);

        log::info!(
            "RX [{}B] txid={} packet={} hops={} rssi={} snr={}",
            rx_pkt.data.len(),
            txid,
            packet,
            hops,
            rx_pkt.rssi,
            rx_pkt.snr,
        );

        self.show(Activity::Receiving);
    }

    /// A packet's audio to the codec thread. It comes back as an event of its
    /// own (on_decoded): waiting for it here, the app couldn't keep the
    /// speaker fed, playback fell behind and the backlog filled the heap
    fn decode(&mut self, txid: u8, packet: i64, audio: &[u8]) {
        let mut payload = [0u8; PAYLOAD_BYTES];
        payload.copy_from_slice(audio);
        if self.rx.playing_txid != Some(txid) {
            self.rx.playout.reset();
            self.rx.playing_txid = Some(txid);
        }
        match self
            .codec_tx
            .try_send(CodecRequest::decode(txid, packet, payload))
        {
            Ok(()) => self.rx.playout.decoding(),
            Err(_) => log::warn!("RX packet={} dropped: the codec is behind", packet),
        }
    }

    /// A packet whose CRC failed, played in place of a silence
    /// (config::PLAY_GARBLED): Codec2 makes most corrupted audio intelligible.
    /// Only when the conveyor can say which packet it was, and it's the last
    /// copy of it that can come (Transmission::garbled_packet). Its length
    /// came from its header, so a wrong one can't be audio. An echo station
    /// records it, to go out again in the replay; a repeater never relays
    /// what it can't vouch for
    fn play_garbled(&mut self, rx_pkt: &RxPacket) {
        if rx_pkt.data.len() != PACKET_BYTES {
            return;
        }
        let Some(transmission) = &mut self.rx.transmission else {
            return;
        };
        let Some(packet) = transmission.garbled_packet(rx_pkt.end_us, rx_pkt.timing_ok) else {
            return;
        };
        let txid = transmission.txid;
        let audio = &rx_pkt.data[HEADER_BYTES..];
        match mode::get() {
            Mode::Repeater => {}
            Mode::Echo => {
                if self.echo.record(txid, packet, audio) {
                    log::info!(
                        "ECHO rec txid={} packet={} garbled rssi={} snr={}",
                        txid,
                        packet,
                        rx_pkt.rssi,
                        rx_pkt.snr
                    );
                }
            }
            Mode::Normal => {
                if !transmission.first_copy(packet) {
                    return;
                }
                self.decode(txid, packet, audio);
                log::info!(
                    "RX [{}B] txid={} packet={} garbled, played rssi={} snr={}",
                    rx_pkt.data.len(),
                    txid,
                    packet,
                    rx_pkt.rssi,
                    rx_pkt.snr
                );
                self.show(Activity::Receiving);
            }
        }
    }

    /// A packet's audio, back from the codec: on to the speaker, in order,
    /// with silence for any lost before it
    fn on_decoded(&mut self, decoded: Decoded) {
        // From a transmission we've stopped playing (a new one, our PTT, the menu)
        if self.rx.playing_txid != Some(decoded.txid) {
            return;
        }
        let waited_us = decoded.asked.elapsed().as_micros() as u32;
        self.rx
            .playback
            .record(decoded.decode_us, waited_us, SPK_AUDIO.len());
        let sounds = &self.sounds;
        self.rx
            .playout
            .decoded(decoded.packet, decoded.pcm, |play| to_speaker(play, sounds));
    }

    /// A good packet: where it landed on its transmission's conveyor, if
    /// it's that transmission's (None for anyone else's). With no conveyor
    /// going, it starts one, unless it's an end packet: a repeater's copy of
    /// one comes after the direct copy has already ended the transmission
    fn ride_conveyor(
        &mut self,
        rx_pkt: &RxPacket,
        pkt_type: PacketType,
        txid: u8,
        hops: u8,
    ) -> Option<Landing> {
        if let Some(transmission) = &mut self.rx.transmission {
            if transmission.txid != txid {
                return None;
            }
            return Some(transmission.heard(arrival(rx_pkt, hops)));
        }
        if matches!(pkt_type, PacketType::VoiceEnd | PacketType::EchoEnd) {
            return None;
        }
        // Its hops say where it is on the belt, whatever channel it was heard
        // on: at point-blank range a radio hears the next channel too
        let (transmission, landing) =
            Transmission::start(txid, channel_count(), arrival(rx_pkt, hops));
        self.rx.last_heard_txid = Some(txid);
        log::info!(
            "CONVEYOR start txid={} from a packet with {} hops, on ch{}",
            txid,
            hops,
            rx_pkt.channel
        );
        self.rx.transmission = Some(transmission);
        Some(landing)
    }

    /// The channel a repeater relays a packet on: its next hop's, from the
    /// transmission's base channel. Before a trusted packet has told us
    /// that, the channel after the one it was heard on. A relay never lands
    /// on the channel the packet came in on. Without the sweep flag there's
    /// one channel, so the relay stays where it was heard
    fn relay_channel(&self, rx_pkt: &RxPacket, hops: u8) -> u8 {
        let next_hop = hops.saturating_add(1);
        let from_base = self
            .rx
            .transmission
            .as_ref()
            .and_then(|transmission| transmission.channel_of(next_hop));
        from_base.unwrap_or(((rx_pkt.channel as u32 + 1) % channel_count() as u32) as u8)
    }

    /// Tell the radio how to listen, and remember it
    fn listen(&mut self, listen: Listen) {
        self.rx.rotation = match listen {
            Listen::Rotation(rotation) => Some(rotation),
            Listen::Hold | Listen::Sweep => None,
        };
        LISTEN.signal(listen);
    }

    /// The radio listens on the conveyor's rotation: a new one whenever it
    /// changes (the conveyor started, its time or base channel settled).
    /// Only a few times a transmission: the radio turns at each bin itself
    fn listen_to_conveyor(&mut self) {
        let repeater = mode::get() == Mode::Repeater;
        let rotation = self
            .rx
            .transmission
            .as_ref()
            .and_then(|transmission| transmission.rotation(repeater));
        if let Some(rotation) = rotation {
            if self.rx.rotation != Some(rotation) {
                self.listen(Listen::Rotation(rotation));
            }
        }
    }

    /// Is `landing` the first copy of its packet on the conveyor? Then it's
    /// taken (to play or relay). Not for a packet that isn't the
    /// transmission's, or has no number (the wrong bin for its hops)
    fn take_first_copy(&mut self, landing: Option<Landing>) -> bool {
        let Some(packet) = landing.and_then(|landing| landing.packet) else {
            return false;
        };
        self.rx
            .transmission
            .as_mut()
            .is_some_and(|transmission| transmission.first_copy(packet))
    }

    /// The end of a transmission: who sent it and where they were. A
    /// repeater passes it on; an echo station replays what it recorded;
    /// everyone else plays the squelch tail (a repeater never plays the
    /// voice, so a tail on its own would be noise). The screen shows who it
    /// was from the first copy heard (direct before relayed)
    async fn on_end(
        &mut self,
        rx_pkt: &RxPacket,
        pkt_type: PacketType,
        txid: u8,
        hops: u8,
        landing: Option<Landing>,
    ) {
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
            // Its first copy only: the transmission ends with it, so a later
            // copy (another repeater's, or one round a loop) finds no
            // conveyor and isn't relayed again
            if self.take_first_copy(landing) {
                if let Some(channel) = relay(rx_pkt, None, self.relay_channel(rx_pkt, hops)).await {
                    log::info!("RELAY EOT txid={} hops={} ch={}", txid, hops + 1, channel);
                }
            }
        } else if !echo_this {
            // The tail, after whatever of the transmission is still being
            // decoded (a repeater's copy of the end packet plays it again)
            if self.rx.playing_txid != Some(txid) {
                self.rx.playout.reset();
                self.rx.playing_txid = Some(txid);
            }
            let sounds = &self.sounds;
            self.rx.playout.ended(|play| to_speaker(play, sounds));
        }
        self.rx.playback.log_and_reset();
        if self
            .rx
            .transmission
            .as_ref()
            .is_some_and(|transmission| transmission.txid == txid)
        {
            if let Some(transmission) = self.rx.transmission.take() {
                log::info!(
                    "CONVEYOR ended by its EOT: txid={} {}",
                    txid,
                    transmission.tally()
                );
            }
        }

        let first_copy = self.display.heard.as_ref().map(|heard| heard.txid) != Some(txid);
        if let (Some(ident), true) = (ident, first_copy) {
            self.display.heard = Some(Heard {
                txid,
                ident,
                rssi: rx_pkt.rssi,
                snr: rx_pkt.snr,
                hops,
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
    /// talker's end packet arrives. (If it's lost, housekeeping() replays
    /// once the conveyor says the transmission is over.) `landing`: where it
    /// landed on the conveyor, None if it's not the transmission on it
    async fn on_echo_packet(
        &mut self,
        rx_pkt: &RxPacket,
        txid: u8,
        hops: u8,
        landing: Option<Landing>,
    ) {
        let Some(packet) = landing.and_then(|landing| landing.packet) else {
            return;
        };
        if rx_pkt.data.len() != PACKET_BYTES {
            return;
        }
        // The second copy of a packet (any hops) isn't stored
        if !self.echo.record(txid, packet, &rx_pkt.data[HEADER_BYTES..]) {
            return;
        }
        log::info!(
            "ECHO rec txid={} packet={} hops={} rssi={} snr={}",
            txid,
            packet,
            hops,
            rx_pkt.rssi,
            rx_pkt.snr
        );
        self.show(Activity::Receiving);
    }

    async fn replay_echo(&mut self) {
        let talker_txid = self.echo.txid();
        let packets = self.echo.take();
        self.listen(Listen::Hold);
        log::info!("ECHO replaying {} packets", packets.len());
        self.draw_screen(Activity::Transmitting);
        // Drop anything heard while we transmit it, as it comes: like on_ptt.
        // A txid of its own, never the talker's: the talker drops anything
        // with its own txid as its transmission relayed back, and would play
        // none of the replay (1 in 128, bench 2026-10-05)
        let txid = self.new_txid(talker_txid);
        self.rx.own_txid = Some(txid);
        // The replay ends with the echo station's own Ident: whoever hears it
        // learns how far away the station is
        let ident = self.ident();
        let end = packet::end(PacketType::EchoEnd, txid, &ident);
        let replay = echo::replay(
            packets,
            txid,
            config::WAKE_PREAMBLE.is_on(),
            end,
            P::now_us(),
        );
        if let Either::Second(()) = select(replay, discard_rx()).await {
            unreachable!("discard_rx never returns");
        }
        self.rx.own_txid_until = Instant::now() + OWN_RELAYS_FOR;
        self.rx.reply_until = Instant::now() + REPLY_WAIT;
        self.logs.last_activity = Instant::now();
        self.draw_screen(Activity::Idle);
    }

    async fn on_ptt(&mut self) {
        // Someone else is talking: we'd only talk over them. Nothing is
        // sent, and the talker can tell: their voice carries on
        if let Some(transmission) = &self.rx.transmission {
            log::info!("PTT refused: txid={} is on the air", transmission.txid);
            self.ptt_refused = true;
            return;
        }
        // PTT pressed — reset RX state
        self.rx.playout.reset();
        self.rx.playing_txid = None;
        self.rx.transmission = None;
        self.listen(Listen::Hold);

        let txid = self.new_txid(None);
        self.rx.own_txid = Some(txid);
        log::info!("PTT pressed — streaming (txid={})", txid);

        self.draw_screen(Activity::Transmitting);

        // The mic has the ADC only while we talk; the battery reading has it
        // the rest of the time
        self.devices.mic.start();

        // Half-duplex: anything heard while we talk can't be played, so throw
        // it away as it comes. Left in the queue it fills up, and the radio
        // stalls handing over the next packet, unable to send ours.
        let packets = match select(self.stream(txid), discard_rx()).await {
            Either::First(packets) => packets,
            Either::Second(()) => unreachable!("discard_rx never returns"),
        };
        self.devices.mic.stop();

        log::info!("PTT released — {} packets sent + EOT", packets);
        self.rx.own_txid_until = Instant::now() + OWN_RELAYS_FOR;
        self.rx.reply_until = Instant::now() + REPLY_WAIT;
        self.logs.last_activity = Instant::now();
        self.draw_screen(Activity::Idle);
    }

    /// Send voice while PTT is held, then our end packet. Returns the
    /// packets sent.
    async fn stream(&mut self, txid: u8) -> usize {
        // Every packet goes on the air in the middle of its bin, at a time the
        // radio keeps to the microsecond, not the moment it's ready: the air
        // is a conveyor of 80 ms bins, a talker sends in the even ones and a
        // repeater relays in the odd ones between (air::bin_us). Codec2 takes
        // a different time on every packet; sent as each encode finished,
        // packets landed up to 65 ms out of their bins (walk of 2026-10-05),
        // too far for a receiver to tell them from someone else's. The echo
        // replay keeps time the same way. Pipelined: each pass captures
        // packet n + 1 while packet n is encoded and handed to the radio
        // with its send time, so the mic is drained continuously
        let bin_0_us = P::now_us() + ENCODE_DEADLINE.as_micros() as i64;
        let send_at_us =
            |bin: u64| bin_0_us + bin as i64 * air::bin_us() as i64 + air::guard_us() as i64;
        let wake = config::WAKE_PREAMBLE.is_on();
        let mic = &mut self.devices.mic;
        let codec_tx = &self.codec_tx;
        // Every packet's header is the same: its place on the conveyor (its
        // bin) says which packet it is
        let header = packet::pack(PacketType::Voice, txid);
        // TODO: pipeline per Codec2 frame, for latency: a packet is 4 frames
        // of 40 ms. Encode each frame as soon as it's captured, while the
        // next is captured, so a packet is ready one frame's encode after its
        // capture ends, not four, and ENCODE_DEADLINE can shrink
        let mut pending_pkt: Option<Box<[i16]>> = None;
        let mut packets: u64 = 0;
        let mut timing = SendTiming::default();
        while self.devices.ptt.is_pressed() {
            // Voice packet n goes in bin 2(n + 1); the wake-up packet in bin
            // 0. The first on the air (the wake-up packet, else voice packet
            // 0) waits for clear air; after that the bins are ours
            let (pcm, ()) = join(capture_packet(mic), async {
                match pending_pkt.take() {
                    Some(pcm) => {
                        let first_on_air = packets == 1 && !wake;
                        let send_at = send_at_us(2 * packets);
                        encode_and_send(
                            codec_tx,
                            header,
                            pcm,
                            send_at,
                            first_on_air,
                            &mut timing,
                            P::now_us,
                        )
                        .await;
                    }
                    None if wake => {
                        let mut wake = packet::wake(txid);
                        wake.send_at_us = Some(send_at_us(0));
                        TX_CHAN.send(wake).await;
                    }
                    None => {}
                }
            })
            .await;
            pending_pkt = Some(pcm);
            packets += 1;
        }
        if let Some(pcm) = pending_pkt {
            let first_on_air = packets == 1 && !wake;
            let send_at = send_at_us(2 * packets);
            encode_and_send(
                codec_tx,
                header,
                pcm,
                send_at,
                first_on_air,
                &mut timing,
                P::now_us,
            )
            .await;
        }

        // Who we are and where we are, as the transmission ends: in the next
        // even bin, like any packet. Sent straight after the last packet, it
        // was on the air while a repeater relayed that one, and the repeater
        // never heard it (relayed 1 of 16 EOTs, 3 desk runs 2026-10-04)
        let end = packet::end(PacketType::VoiceEnd, txid, &self.ident());
        TX_CHAN
            .send(TxRequest {
                data: end,
                preamble: None,
                channel: 0,
                clear_air_first: false,
                send_at_us: Some(send_at_us(2 * (packets + 1))),
            })
            .await;
        // Back once it's on the air: until then we're still transmitting
        let sent_us = send_at_us(2 * (packets + 1) + 1);
        Timer::at(embassy_time::Instant::from_micros(sent_us.max(0) as u64)).await;
        log::info!("TX bins: {}", timing);
        packets as usize
    }

    /// A txid for a new transmission of ours: random, but never the last one
    /// heard (whoever sent it still counts it as its own for a moment and
    /// would drop ours, bench 2026-10-05), nor `also_not`
    fn new_txid(&self, also_not: Option<u8>) -> u8 {
        loop {
            let txid = random_txid::<P>();
            if Some(txid) != self.rx.last_heard_txid && Some(txid) != also_not {
                return txid;
            }
        }
    }

    /// The battery, every BATTERY_EVERY. Logged on the GPS line (log_gps),
    /// so a run's log is its discharge curve: the way to a percent that
    /// means "how long will my radio keep working". Housekeeping never runs
    /// while we transmit, so the mic never has the ADC here
    async fn read_battery(&mut self) {
        let due = self
            .display
            .battery_read_at
            .is_none_or(|read_at| read_at.elapsed() >= BATTERY_EVERY);
        if !due {
            return;
        }
        self.display.battery_read_at = Some(Instant::now());
        self.display.battery_mv = self.devices.battery.millivolts().await;
    }

    /// Us, for our end packets: our name, and where we are now if our user
    /// has chosen to send it (config::SEND_POSITION)
    fn ident(&self) -> Ident {
        let position = if config::SEND_POSITION.is_on() {
            self.devices.gps.latest().and_then(|fix| fix.position)
        } else {
            None
        };
        Ident {
            name: self.display.name.clone(),
            position,
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
        self.rx.playout.reset();
        self.rx.playing_txid = None;
        self.rx.transmission = None;
        let mut menu = Menu::new();
        loop {
            self.draw_menu(&menu);
            match self.devices.knob.next().await {
                Knob::Cw => menu.rotate(1),
                Knob::Ccw => menu.rotate(-1),
                Knob::Release => {} // a press is a click, in here
                Knob::Press => match menu.click() {
                    Outcome::Stay => {}
                    Outcome::Exit => break,
                    Outcome::Set(setting, value) => self.apply(setting, value),
                    Outcome::ExitAndDo(Action::Screenshot) => {
                        self.save_screenshot();
                        break;
                    }
                    Outcome::Open(Page::AddWifi) => self.run_add_wifi().await,
                    Outcome::Open(Page::ForgetWifi) => self.run_forget_wifi().await,
                    Outcome::Open(Page::Connected) => self.run_connected().await,
                    Outcome::Open(Page::Update) => self.run_update().await,
                },
            }
        }
        // Drop whatever arrived while we were menuing
        while RX_CHAN.try_receive().is_ok() {}
        self.draw_screen(Activity::Idle);
    }

    /// The networks in range, from scans back to back while this list is on
    /// the screen (and only then: scans transmit). Append only, so a row
    /// never moves under the cursor. Picking one asks its password (an open
    /// network has none) and saves it; the radio joins it when it's next
    /// looking for a network. Saved ones get a *
    async fn run_add_wifi(&mut self) {
        let Some(settings) = self.devices.settings.as_ref() else {
            self.pick("No storage to save to", &[]).await;
            return;
        };
        // Refused rather than switched on for a scan: off is a choice made
        // for privacy (a scan's probes can be direction-found)
        if !config::WIFI_ON.is_on() {
            self.pick("WiFi is off", &[]).await;
            return;
        }
        let saved = settings.wifi_ssids();
        let mut list = ScanList::default();
        let mut cursor = 0; // 0 = Back, 1.. = the list's rows
        wifi::SCANS.reset();
        wifi::SCAN_WANTED.store(true, Ordering::Relaxed);
        loop {
            let title = match (list.rows().is_empty(), list.scanned()) {
                (true, false) => "Add WiFi: scanning",
                (true, true) => "Add WiFi: none seen",
                (false, _) => "Add WiFi (dBm)",
            };
            let labels: Vec<String> = list.rows().iter().map(|row| row.label()).collect();
            let mut rows = vec![(BACK, false)];
            for (row, label) in list.rows().iter().zip(&labels) {
                rows.push((
                    label.as_str(),
                    saved.iter().any(|ssid| ssid.as_str() == row.ssid.as_str()),
                ));
            }
            self.draw_list(title, &rows, cursor);

            match select(self.devices.knob.next(), wifi::SCANS.wait()).await {
                Either::Second(found) => list.add_scan(&found),
                Either::First(Knob::Cw) => cursor = (cursor + 1).min(list.rows().len()),
                Either::First(Knob::Ccw) => cursor = cursor.saturating_sub(1),
                Either::First(Knob::Release) => {}
                Either::First(Knob::Press) => {
                    if cursor == 0 {
                        break;
                    }
                    let row = &list.rows()[cursor - 1];
                    let ssid = row.ssid.to_string();
                    // No scans while typing: the list isn't on the screen
                    wifi::SCAN_WANTED.store(false, Ordering::Relaxed);
                    let password = if row.open {
                        Some(String::new())
                    } else {
                        self.run_password(&ssid).await
                    };
                    if let (Some(password), Some(settings)) =
                        (password, self.devices.settings.as_mut())
                    {
                        settings.add_wifi(&ssid, &password);
                        log::info!("Menu: WiFi network {:?} saved", ssid);
                        break;
                    }
                    wifi::SCAN_WANTED.store(true, Ordering::Relaxed);
                }
            }
        }
        wifi::SCAN_WANTED.store(false, Ordering::Relaxed);
    }

    /// Type a password on the knob's wheel (password_entry.rs). None if
    /// given up on (DEL with nothing typed)
    async fn run_password(&mut self, ssid: &str) -> Option<String> {
        let mut entry = PasswordEntry::new();
        loop {
            self.draw_password(ssid, &entry);
            match self.devices.knob.next().await {
                Knob::Cw => entry.rotate(1),
                Knob::Ccw => entry.rotate(-1),
                Knob::Release => {}
                Knob::Press => match entry.click() {
                    Click::Typing => {}
                    Click::Done(password) => return Some(password),
                    Click::Cancel => return None,
                },
            }
        }
    }

    /// The saved networks; picking one asks first, then forgets it. If it's
    /// the one joined, the radio leaves it
    async fn run_forget_wifi(&mut self) {
        let Some(settings) = self.devices.settings.as_ref() else {
            self.pick("No storage", &[]).await;
            return;
        };
        let ssids = settings.wifi_ssids();
        let title = if ssids.is_empty() {
            "None saved"
        } else {
            "Forget"
        };
        let Some(picked) = self.pick(title, &ssids).await else {
            return;
        };
        let ssid = &ssids[picked];
        let question = format!("Forget {}?", ssid);
        if self.pick(&question, &["Yes".into()]).await.is_none() {
            return;
        }
        if let Some(settings) = self.devices.settings.as_mut() {
            settings.forget_wifi(ssid);
            log::info!("Menu: WiFi network {:?} forgotten", ssid);
        }
    }

    /// Which network we're on, and our address on it: the screen shows only
    /// the address
    async fn run_connected(&mut self) {
        match P::network() {
            Network::Joined { ssid, ip } => {
                let address = format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
                self.pick("Connected", &[ssid.as_str().into(), address])
                    .await;
            }
            _ => {
                self.pick("Not on a network", &[]).await;
            }
        }
    }

    /// Ask the management server for firmware, and install it if asked
    /// to. The server holds THE firmware: any image other than ours is
    /// offered, newer or not (management.rs Request::Hello)
    async fn run_update(&mut self) {
        if !matches!(P::network(), Network::Joined { .. }) {
            self.pick("Not on a network", &[]).await;
            return;
        }
        self.draw_list("Update: asking...", &[], 0);
        // Nothing left over from a job that timed out
        while management::JOBS.try_receive().is_ok() {}
        management::ANSWERS.reset();
        let _ = management::JOBS.try_send(Job::Check);
        let answer = select(management::ANSWERS.wait(), Timer::after(UPDATE_TIMEOUT)).await;
        let offer = match answer {
            Either::First(Ok(Response::BundleOffer(offer))) => offer,
            Either::First(Ok(Response::UpToDate)) => {
                self.pick("Up to date", &[]).await;
                return;
            }
            // Few words, meant for this screen ("Not allowed")
            Either::First(Ok(Response::Error(e))) | Either::First(Err(e)) => {
                log::warn!("Update: {}", e);
                self.pick(&e, &[]).await;
                return;
            }
            Either::First(Ok(other)) => {
                log::warn!("Update: unexpected answer {:?}", other);
                self.pick("Server: odd answer", &[]).await;
                return;
            }
            Either::Second(()) => {
                self.pick("Server: no answer", &[]).await;
                return;
            }
        };
        let title = format!(
            "{} {:.1} MB",
            offer.version,
            offer.size as f32 / 1_000_000.0
        );
        if self.pick(&title, &["Install".into()]).await.is_some() {
            self.run_install(offer).await;
        }
    }

    /// Install the bundle `offer` (the net thread does it: download, verify,
    /// extract) while showing which step and how far along; a click gives
    /// up. Done, the radio reboots into it (the firmware's job), so this
    /// never returns then
    async fn run_install(&mut self, offer: Offer) {
        log::info!("Update: installing {} ({} B)", offer.version, offer.size);
        management::CANCEL.store(false, Ordering::Relaxed);
        management::INSTALL.reset();
        let _ = management::JOBS.try_send(Job::Install(offer));
        let mut shown = "Downloading 0%".to_string();
        loop {
            self.draw_list(&shown, &[], 0);
            let news = select3(
                management::INSTALL.wait(),
                self.devices.knob.next(),
                Timer::after(INSTALL_QUIET),
            )
            .await;
            match news {
                Either3::First(Installing::Downloading(percent)) => {
                    shown = format!("Downloading {}%", percent);
                }
                Either3::First(Installing::Verifying(percent)) => {
                    shown = format!("Verifying {}%", percent);
                }
                Either3::First(Installing::Extracting(percent)) => {
                    shown = format!("Extracting {}%", percent);
                }
                Either3::First(Installing::Done) => shown = "Installed, rebooting".into(),
                Either3::First(Installing::Failed(e)) => {
                    log::warn!("Update: install failed: {}", e);
                    self.pick(&e, &[]).await;
                    return;
                }
                Either3::Second(Knob::Press) => {
                    management::CANCEL.store(true, Ordering::Relaxed);
                    shown = "Cancelling...".into();
                }
                Either3::Second(_) => {}
                Either3::Third(()) => {
                    management::CANCEL.store(true, Ordering::Relaxed);
                    self.pick("Install stalled", &[]).await;
                    return;
                }
            }
        }
    }

    /// A list to pick from, Back first (where the cursor starts): the index
    /// into `labels` of the one clicked, or None for Back
    async fn pick(&mut self, title: &str, labels: &[String]) -> Option<usize> {
        let mut cursor = 0;
        loop {
            let mut rows = vec![(BACK, false)];
            rows.extend(labels.iter().map(|label| (label.as_str(), false)));
            self.draw_list(title, &rows, cursor);
            match self.devices.knob.next().await {
                Knob::Cw => cursor = (cursor + 1).min(labels.len()),
                Knob::Ccw => cursor = cursor.saturating_sub(1),
                Knob::Release => {}
                Knob::Press => return cursor.checked_sub(1),
            }
        }
    }

    /// The radio screen as it was when the menu opened, to
    /// <data>/screenshots as a BMP (pull it over WebDAV). The menu draws only
    /// on its own frames, so the last radio screen drawn is the one it covered.
    /// The flash write stalls both cores a few ms: fine here, menuing has
    /// already stopped RX
    fn save_screenshot(&self) {
        let Some(data_dir) = P::data_dir() else {
            log::warn!("Screenshot not saved: no storage");
            return;
        };
        let dir = data_dir.join("screenshots");
        match save_numbered(&dir, "bmp", &self.display.shown_screen.bmp()) {
            Ok(path) => log::info!("Screenshot saved to {}", path.display()),
            Err(e) => log::warn!("Screenshot not saved: {}", e),
        }
    }

    fn apply(&mut self, setting: Setting, value: u8) {
        log::info!("Menu: {:?} = {}", setting, value);
        match setting {
            Setting::Lock => self.locked = value != 0,
            Setting::Mode => config::MODE.set(value as i32, self.devices.settings.as_mut()),
            Setting::Wifi => config::WIFI_ON.set(value as i32, self.devices.settings.as_mut()),
            Setting::HttpApi => {
                config::HTTP_API_ON.set(value as i32, self.devices.settings.as_mut())
            }
            Setting::SendPosition => {
                config::SEND_POSITION.set(value as i32, self.devices.settings.as_mut())
            }
            Setting::LogToFlash => {
                config::LOG_TO_FLASH.set(value as i32, self.devices.settings.as_mut())
            }
        }
    }

    fn setting(&self, setting: Setting) -> u8 {
        match setting {
            Setting::Lock => self.locked as u8,
            Setting::Mode => mode::get() as u8,
            Setting::Wifi => config::WIFI_ON.get() as u8,
            Setting::HttpApi => config::HTTP_API_ON.get() as u8,
            Setting::SendPosition => config::SEND_POSITION.get() as u8,
            Setting::LogToFlash => config::LOG_TO_FLASH.get() as u8,
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
        rows[4] =
            screen_text::heard_at_and_network(self.display.heard.as_ref(), &network.screen_text());
        rows[5] = screen_text::status(
            speaker::volume(),
            self.locked,
            activity,
            Some(utc::now_hm()),
            mode::get(),
            self.display.battery_mv,
        );

        let mut frame = self.devices.screen.frame();
        for (row, y) in rows.iter().zip(ROWS_Y) {
            Text::new(row, Point::new(1, y), self.display.style)
                .draw(&mut frame)
                .unwrap();
        }
        self.display.shown_screen.copy(&frame);
        self.devices.screen.show(frame);
        self.display.shown_fix = fix;
        self.display.shown_network = network;
        self.display.shown_time = utc::now_hm();
        self.display.shown_activity = activity;
        self.display.shown_battery_cv = self.display.battery_mv.map(|mv| mv / 10);
    }

    fn draw_menu(&self, menu: &Menu) {
        let rows = menu.rows(|setting, value| self.setting(setting) == value);
        self.draw_list(menu.title(), &rows, menu.cursor());
    }

    /// Title, then up to 4 rows; the cursor row is inverted, rows marked
    /// true get a * (a setting's current value, a saved network).
    fn draw_list(&self, title: &str, rows: &[(&str, bool)], cursor: usize) {
        const TOP: i32 = 13;
        const ROW_H: i32 = 11;
        const VISIBLE: usize = 4;

        let mut frame = self.devices.screen.frame();
        let title: String = title.chars().take(screen_text::WIDTH).collect();
        Text::new(&title, Point::new(1, 9), self.display.style)
            .draw(&mut frame)
            .unwrap();

        let first = cursor.saturating_sub(VISIBLE - 1);
        for (i, (label, current)) in rows.iter().enumerate().skip(first).take(VISIBLE) {
            let y = TOP + (i - first) as i32 * ROW_H;
            let style = if i == cursor {
                Rectangle::new(Point::new(0, y), Size::new(128, ROW_H as u32))
                    .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
                    .draw(&mut frame)
                    .unwrap();
                self.inverted_style()
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

    /// The network's name, what's typed with the cursor after it, and the
    /// wheel: the stop a click would type inverted, its neighbours either
    /// side (password_entry.rs)
    fn draw_password(&self, ssid: &str, entry: &PasswordEntry) {
        const CHAR_W: i32 = 6; // FONT_6X10
        const WHEEL_TOP: i32 = 44;
        const ROW_H: i32 = 11;

        let mut frame = self.devices.screen.frame();
        let title: String = ssid.chars().take(screen_text::WIDTH).collect();
        Text::new(&title, Point::new(1, 9), self.display.style)
            .draw(&mut frame)
            .unwrap();
        Text::new(
            &entry.typed_line(screen_text::WIDTH),
            Point::new(1, 28),
            self.display.style,
        )
        .draw(&mut frame)
        .unwrap();

        let (wheel, start, len) = entry.wheel_line(screen_text::WIDTH);
        Text::with_baseline(
            &wheel,
            Point::new(1, WHEEL_TOP + 1),
            self.display.style,
            Baseline::Top,
        )
        .draw(&mut frame)
        .unwrap();
        let x = 1 + start as i32 * CHAR_W;
        Rectangle::new(
            Point::new(x, WHEEL_TOP),
            Size::new((len as i32 * CHAR_W) as u32, ROW_H as u32),
        )
        .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
        .draw(&mut frame)
        .unwrap();
        Text::with_baseline(
            &wheel[start..start + len],
            Point::new(x, WHEEL_TOP + 1),
            self.inverted_style(),
            Baseline::Top,
        )
        .draw(&mut frame)
        .unwrap();
        self.devices.screen.show(frame);
    }

    /// Dark text, for on a lit row
    fn inverted_style(&self) -> MonoTextStyle<'static, BinaryColor> {
        MonoTextStyleBuilder::new()
            .font(&FONT_6X10)
            .text_color(BinaryColor::Off)
            .build()
    }
}

/// For the GPS log line: the last battery reading
struct BatteryLog(Option<u32>);

impl core::fmt::Display for BatteryLog {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self.0 {
            Some(mv) => write!(f, "battery={}mV", mv),
            None => write!(f, "battery=none"),
        }
    }
}

/// Write `bytes` to `dir` as the next number up: 0001.<extension>, 0002...
/// (one past the highest there, so a deleted file's number isn't reused
/// unless it was the last). Makes `dir` if it isn't there yet
fn save_numbered(dir: &Path, extension: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let mut highest = 0u32;
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|found| found == extension) {
            let number = path
                .file_stem()
                .and_then(|stem| stem.to_str()?.parse().ok());
            highest = highest.max(number.unwrap_or(0));
        }
    }
    let path = dir.join(format!("{:04}.{}", highest + 1, extension));
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// Generate 160ms squelch tail (white noise with fade-out), packet-sized.
fn generate_squelch(random: fn() -> u32) -> Arc<[i16]> {
    const AMPLITUDE: i32 = 8000;
    (0..PACKET_SAMPLES)
        .map(|i| {
            let fade = (PACKET_SAMPLES - i) as i32 * AMPLITUDE / PACKET_SAMPLES as i32;
            ((random() % (2 * fade as u32 + 1)) as i32 - fade) as i16
        })
        .collect()
}

/// One thing from the playout onto the speaker's queue. Full (the speaker is
/// 4 packets behind): dropped and logged, rather than piling up
fn to_speaker(play: Play<Arc<[i16]>>, sounds: &Sounds) {
    let audio = match play {
        Play::Audio(audio) => audio,
        // Known only once the next packet's audio is back. If the speaker
        // has run dry by then, it already sat silent through the lost
        // packet's time: more silence now would play the gap twice and
        // put everything after it later
        Play::Silence if SPK_AUDIO.is_empty() => {
            log::info!("SPK gap: the speaker already ran dry through a lost packet");
            return;
        }
        Play::Silence => {
            log::info!("SPK gap: silence for a lost packet");
            sounds.silence.clone()
        }
        Play::Squelch => sounds.squelch.clone(),
    };
    if SPK_AUDIO.try_send(audio).is_err() {
        log::warn!("SPK queue full, dropped a packet of audio");
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

/// A repeater's relay of `rx_pkt`: the same packet with one more hop, on the
/// next channel (`preamble`: a wake-up's long one), in the bin after the copy
/// it heard: it starts exactly one bin (80 ms) after that copy started, so it
/// sits in the middle of its bin too. Its bin is its own: no waiting for
/// clear air. `channel`: the channel its next hop is on. Which channel, or
/// None once it has had packet::MAX_HOPS
async fn relay(rx_pkt: &RxPacket, preamble: Option<u16>, channel: u8) -> Option<u8> {
    let Some(header) = packet::relayed([rx_pkt.data[0], rx_pkt.data[1]]) else {
        log::info!("RELAY: not relayed, it has had {} hops", packet::MAX_HOPS);
        return None;
    };
    let mut data = heapless::Vec::new();
    let _ = data.extend_from_slice(&header);
    let _ = data.extend_from_slice(&rx_pkt.data[HEADER_BYTES..]);
    // A wake-up's long preamble takes the same air as a voice packet (air.rs)
    let heard_started_us =
        rx_pkt.end_us - air::packet_us(air::preamble_symbols(), PACKET_BYTES) as i64;
    TX_CHAN
        .send(TxRequest {
            data,
            preamble,
            channel,
            clear_air_first: false,
            send_at_us: Some(heard_started_us + air::bin_us() as i64),
        })
        .await;
    Some(channel)
}

/// A good packet, for the conveyor: `hops` from its header
fn arrival(rx_pkt: &RxPacket, hops: u8) -> Arrival {
    Arrival {
        end_us: rx_pkt.end_us,
        hops,
        channel: rx_pkt.channel,
        rssi: rx_pkt.rssi,
        timing_ok: rx_pkt.timing_ok,
    }
}

/// How many hop channels a radio listens on: rx_hops if it sweeps, else
/// just the start slot
fn channel_count() -> u8 {
    if !config::SWEEP.is_on() {
        return 1;
    }
    config::RX_HOPS.get() as u8
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

/// Encode one packet on the codec thread, then hand it to the radio with its
/// send time (`send_at_us`, on the radio's clock: `now_us`). If the encode ran
/// past it, the radio drops it: a packet out of its bin is worse than none
async fn encode_and_send(
    codec_tx: &SyncSender<CodecRequest>,
    header: [u8; 2],
    pcm: Box<[i16]>,
    send_at_us: i64,
    first_on_air: bool,
    timing: &mut SendTiming,
    now_us: fn() -> i64,
) {
    let asked_us = now_us();
    codec_tx.send(CodecRequest::encode(header, pcm)).unwrap();
    let packet = ENCODED.receive().await;
    timing.note(asked_us, now_us(), send_at_us);
    TX_CHAN
        .send(TxRequest {
            data: packet,
            preamble: None,
            channel: 0,
            clear_air_first: first_on_air,
            send_at_us: Some(send_at_us),
        })
        .await;
}

/// How one transmission's packets made their send times, for a log line at
/// its end
#[derive(Default)]
struct SendTiming {
    packets: u32,
    fastest_encode_us: i64,
    slowest_encode_us: i64,
    /// The least time any packet was ready before its send time. Negative:
    /// it was late, and the radio dropped it
    least_room_us: i64,
    late: u32,
}

impl SendTiming {
    /// One packet: when its encode was asked for, when it was done, and when
    /// it was to be sent, all in µs on the radio's clock
    fn note(&mut self, asked_us: i64, encoded_us: i64, send_at_us: i64) {
        let encode_us = encoded_us - asked_us;
        let room_us = send_at_us - encoded_us;
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
                "TX packet ready {}ms after its send time (encode {}ms): the radio drops it",
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
            "{} voice packets, encode {}..{}ms, least room before a send time {}ms, late {}",
            self.packets,
            self.fastest_encode_us / 1000,
            self.slowest_encode_us / 1000,
            self.least_room_us / 1000,
            self.late
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_files_number_up_from_the_highest() {
        let dir = std::env::temp_dir().join(format!("oswst-screenshots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            save_numbered(&dir, "bmp", b"one").unwrap(),
            dir.join("0001.bmp")
        );
        assert_eq!(
            save_numbered(&dir, "bmp", b"two").unwrap(),
            dir.join("0002.bmp")
        );
        // A deleted earlier one leaves its number free; the next still goes past the highest
        std::fs::remove_file(dir.join("0001.bmp")).unwrap();
        assert_eq!(
            save_numbered(&dir, "bmp", b"three").unwrap(),
            dir.join("0003.bmp")
        );
        assert_eq!(std::fs::read(dir.join("0003.bmp")).unwrap(), b"three");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
