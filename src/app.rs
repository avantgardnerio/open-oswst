use embassy_futures::join::join;
use embassy_futures::select::{select4, Either4};
use embedded_graphics::mono_font::ascii::FONT_6X10;
use embedded_graphics::mono_font::{MonoTextStyle, MonoTextStyleBuilder};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::{Baseline, Text};
use esp_idf_svc::hal::gpio::AnyIOPin;
use esp_idf_svc::hal::gpio::{Input, PinDriver, Pull};
use open_oswst::devices::encoder::{Encoder, Event};
use open_oswst::devices::mic::Mic;
use open_oswst::devices::radio::{RxPacket, TxRequest, RX_CHAN, TX_CHAN};
use open_oswst::devices::screen::{Frame, Screen};
use open_oswst::devices::speaker::{self, MAX_VOLUME, SPK_FRAMES, SPK_REQ};
use std::future::Future;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use std::sync::atomic::Ordering;

use esp_idf_svc::nvs::{EspNvs, NvsCustom};

use crate::menu::{Menu, Outcome, Setting};
use crate::IS_REPEATER;
use open_oswst::codec::{
    CodecRequest, CodecResponse, CODEC2_FRAME_SAMPLES, CODEC_REPLY, FRAMES_PER_PACKET,
    HEADER_BYTES, PACKET_BYTES, PAYLOAD_BYTES, STEREO_PACKET_SAMPLES,
};

/// Packet type constants (5 bits, upper bits of header)
const PKT_TYPE_VOICE: u8 = 0x00;

/// Audio per packet: FRAMES_PER_PACKET × 40ms
const PACKET_MS: u128 = FRAMES_PER_PACKET as u128 * 40;

/// Stereo samples per Codec2 frame (320 mono × 2 channels)
const STEREO_FRAME_SAMPLES: usize = CODEC2_FRAME_SAMPLES * 2;

pub struct Peripherals {
    pub ptt: AnyIOPin<'static>,
}

pub async fn init(
    p: Peripherals,
    mic: Mic,
    encoder: Encoder,
    screen: Screen,
    mac_str: heapless::String<18>,
    nvs: Option<EspNvs<NvsCustom>>,
    codec_tx: SyncSender<CodecRequest>,
) -> impl Future<Output = ()> {
    // PRG button on GPIO0 — active LOW with internal pull-up
    let button = PinDriver::input(p.ptt, Pull::Up).unwrap();

    log::info!("MAC: {}", mac_str);

    let app = App {
        button,
        mic,
        encoder,
        screen,
        mac_str,
        nvs,
        codec_tx,
        // Pre-generate audio buffers
        silence: vec![0i16; STEREO_PACKET_SAMPLES].into(),
        squelch: generate_squelch(),
        style: MonoTextStyleBuilder::new()
            .font(&FONT_6X10)
            .text_color(BinaryColor::On)
            .build(),
        line_buf: heapless::String::new(),
        cur_txid: None,
        last_played_seq: 0,
        seq_buf: Default::default(),
        last_rx_time: Instant::now(),
        spk_active: false,
        locked: false,
    };

    async move {
        let mut app = app;
        app.run().await;
    }
}

struct App {
    button: PinDriver<'static, Input>,
    mic: Mic,
    encoder: Encoder,
    screen: Screen,
    mac_str: heapless::String<18>,
    nvs: Option<EspNvs<NvsCustom>>, // saved settings; None if the partition couldn't open
    codec_tx: SyncSender<CodecRequest>,
    silence: Arc<[i16]>,
    squelch: Arc<[i16]>,
    style: MonoTextStyle<'static, BinaryColor>,
    line_buf: heapless::String<64>,

    // Track current transmitter for seq ordering
    cur_txid: Option<u8>,
    last_played_seq: u8,
    seq_buf: [Option<Arc<[i16]>>; 16],
    last_rx_time: Instant,
    spk_active: bool, // true once speaker has been kicked

    locked: bool, // PTT ignored; the menu still opens, so it can be unlocked
}

impl App {
    async fn run(&mut self) {
        // Show initial RX state
        self.draw_rx_screen();

        loop {
            // Auto-reset if no packet from current txid in 500ms
            if self.cur_txid.is_some() && self.last_rx_time.elapsed() > Duration::from_millis(500) {
                log::info!("RX timeout, resetting txid lock");
                send_to_speaker(&self.squelch);
                self.reset_rx_state();
                self.spk_active = false;
            }

            let locked = self.locked;
            let button = &mut self.button;
            let ptt = async {
                if locked {
                    core::future::pending::<()>().await;
                }
                let _ = button.wait_for_low().await;
            };
            match select4(
                RX_CHAN.receive(),
                ptt,
                SPK_REQ.receive(),
                self.encoder.next(),
            )
            .await
            {
                Either4::First(rx_pkt) => self.on_rx_packet(rx_pkt).await,
                Either4::Second(_) => self.on_ptt().await,
                Either4::Third(_) => self.on_speaker_request(),
                Either4::Fourth(Event::Click) => self.run_menu().await,
                Either4::Fourth(Event::Cw) => self.change_volume(1),
                Either4::Fourth(Event::Ccw) => self.change_volume(-1),
            }
        }
    }

    async fn on_rx_packet(&mut self, rx_pkt: RxPacket) {
        if rx_pkt.data.len() < HEADER_BYTES {
            log::warn!(
                "RX [{}B] too short, rssi={} snr={}",
                rx_pkt.data.len(),
                rx_pkt.rssi,
                rx_pkt.snr
            );
            return;
        }

        // Parse 2-byte header: |5b type|7b txid|4b seq|
        let header = u16::from_be_bytes([rx_pkt.data[0], rx_pkt.data[1]]);
        let pkt_type = (header >> 11) as u8;
        let txid = ((header >> 4) & 0x7F) as u8;
        let seq = (header & 0x0F) as u8;

        if pkt_type != PKT_TYPE_VOICE {
            log::warn!(
                "RX unknown pkt_type={} header=0x{:04X} raw=[0x{:02X},0x{:02X}]",
                pkt_type,
                header,
                rx_pkt.data[0],
                rx_pkt.data[1]
            );
            return;
        }

        // Header-only = end of transmission — relay if repeater, then squelch
        if rx_pkt.data.len() == HEADER_BYTES {
            if IS_REPEATER.load(Ordering::Relaxed) {
                let mut relay = heapless::Vec::new();
                let _ = relay.extend_from_slice(&rx_pkt.data);
                TX_CHAN.send(TxRequest { data: relay }).await;
                log::info!("RELAY EOT txid={}", txid);
            }
            log::info!("RX EOT from txid={}", txid);
            send_to_speaker(&self.squelch);
            self.reset_rx_state();
            self.spk_active = false;
            return;
        }

        if rx_pkt.data.len() != PACKET_BYTES {
            log::warn!("RX [{}B] unexpected size, ignoring", rx_pkt.data.len());
            return;
        }

        let payload = &rx_pkt.data[HEADER_BYTES..];

        if self.cur_txid.is_none() {
            self.cur_txid = Some(txid);
            self.last_played_seq = seq.wrapping_sub(1) & 0x0F;
        }

        if self.cur_txid != Some(txid) {
            log::warn!(
                "RX ignoring txid={} (locked to {})",
                txid,
                self.cur_txid.unwrap()
            );
            return;
        }

        self.last_rx_time = Instant::now();
        let expected_seq = (self.last_played_seq.wrapping_add(1)) & 0x0F;
        let diff = (seq.wrapping_sub(expected_seq) & 0x0F) as i8;
        let diff = if diff > 7 { diff - 16 } else { diff };

        match diff {
            -2..=-1 => {
                log::info!("RX seq={} old (diff={}), dropping", seq, diff);
                return;
            }
            0..=2 => {
                // Repeater: relay after dedup (non-duplicate voice)
                if IS_REPEATER.load(Ordering::Relaxed) {
                    let mut relay = heapless::Vec::new();
                    let _ = relay.extend_from_slice(&rx_pkt.data);
                    TX_CHAN.send(TxRequest { data: relay }).await;
                    log::info!("RELAY [{}B] txid={} seq={}", rx_pkt.data.len(), txid, seq);
                    self.last_played_seq = seq;
                    return; // skip decode — fast turnaround
                }
                // Send to codec thread for decode, await reply
                let mut payload_arr = [0u8; PAYLOAD_BYTES];
                payload_arr.copy_from_slice(payload);
                self.codec_tx
                    .send(CodecRequest::decode(seq, txid, payload_arr))
                    .unwrap();
                if let CodecResponse::Decoded { seq, txid, pcm } = CODEC_REPLY.receive().await {
                    if self.cur_txid == Some(txid) {
                        self.seq_buf[seq as usize] = Some(pcm.into());

                        // Kick speaker once we have 2 consecutive packets
                        if !self.spk_active {
                            let next = (self.last_played_seq.wrapping_add(1)) & 0x0F;
                            let next2 = (next.wrapping_add(1)) & 0x0F;
                            if self.seq_buf[next as usize].is_some()
                                && self.seq_buf[next2 as usize].is_some()
                            {
                                let pcm = self.seq_buf[next as usize].take().unwrap();
                                self.last_played_seq = next;
                                self.spk_active = true;
                                send_to_speaker(&pcm);
                            }
                        }
                    }
                }
            }
            _ => {
                log::warn!("RX seq={} unexpected (diff={}), resetting", seq, diff);
                self.reset_rx_state();
                return;
            }
        }

        log::info!(
            "RX [{}B] txid={} seq={} played_to={} rssi={} snr={}",
            rx_pkt.data.len(),
            txid,
            seq,
            self.last_played_seq,
            rx_pkt.rssi,
            rx_pkt.snr,
        );

        self.draw_rx_audio_screen(rx_pkt.rssi, rx_pkt.snr);
    }

    async fn on_ptt(&mut self) {
        // PTT pressed — reset RX state
        self.reset_rx_state();
        self.spk_active = false;

        // Generate random 7-bit txid for this PTT press (dedup key)
        let txid = (unsafe { esp_idf_svc::sys::esp_random() } & 0x7F) as u8;
        let mut seq: u8 = 0;
        log::info!("PTT pressed — streaming (txid={})", txid);

        self.draw_tx_screen();

        self.mic.drain(); // discard stale

        // Pipelined: each pass captures packet N+1 while packet N is encoded and
        // sent, so the mic is drained continuously
        let mic = &mut self.mic;
        let codec_tx = &self.codec_tx;
        let mut pending: Option<([u8; 2], Box<[i16]>)> = None;
        let mut packets = 0usize;
        while self.button.is_low() {
            let header = voice_header(txid, seq);
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
        let _ = eot_data.extend_from_slice(&voice_header(txid, seq));
        TX_CHAN.send(TxRequest { data: eot_data }).await;

        log::info!("PTT released — {} packets sent + EOT", packets);

        // Redraw RX screen
        self.draw_rx_screen();
    }

    fn on_speaker_request(&mut self) {
        // Speaker wants next audio
        let next = (self.last_played_seq.wrapping_add(1)) & 0x0F;
        if let Some(pcm) = self.seq_buf[next as usize].take() {
            self.last_played_seq = next;
            send_to_speaker(&pcm);
        } else if self.cur_txid.is_some() {
            // Gap — skip this seq, send silence
            // self.last_played_seq = next;
            log::info!("SPK gap at seq={}, sending silence", next);
            send_to_speaker(&self.silence);
        }
        // else: not receiving, nothing to send — DMA auto_clear handles silence
    }

    fn change_volume(&mut self, delta: i8) {
        let level = speaker::volume()
            .saturating_add_signed(delta)
            .min(MAX_VOLUME);
        speaker::set_volume(level);
        log::info!("Volume {}", level);
        // Mid-reception the next packet redraws within 160ms; don't flash "Listening"
        if self.cur_txid.is_none() {
            self.draw_rx_screen();
        }
    }

    /// Menu mode is only menuing: audio and RX stop until we leave, and PTT
    /// is ignored.
    async fn run_menu(&mut self) {
        self.reset_rx_state();
        self.spk_active = false;
        let mut menu = Menu::new();
        loop {
            self.draw_menu(&menu);
            match self.encoder.next().await {
                Event::Cw => menu.rotate(1),
                Event::Ccw => menu.rotate(-1),
                Event::Click => match menu.click() {
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

    fn apply(&mut self, setting: Setting, value: bool) {
        log::info!("Menu: {:?} = {}", setting, value);
        match setting {
            Setting::Lock => self.locked = value,
            Setting::Repeater => {
                IS_REPEATER.store(value, Ordering::Relaxed);
                self.save_u8("repeater", value as u8);
            }
        }
    }

    /// Persist a setting so it survives a reboot.
    fn save_u8(&mut self, key: &str, value: u8) {
        let Some(nvs) = self.nvs.as_mut() else {
            return;
        };
        if let Err(e) = nvs.set_u8(key, value) {
            log::warn!("NVS save {}={} failed: {}", key, value, e);
        }
    }

    fn setting(&self, setting: Setting) -> bool {
        match setting {
            Setting::Lock => self.locked,
            Setting::Repeater => IS_REPEATER.load(Ordering::Relaxed),
        }
    }

    fn reset_rx_state(&mut self) {
        self.cur_txid = None;
        self.last_played_seq = 0;
        self.seq_buf.iter_mut().for_each(|s| *s = None);
    }

    fn draw_rx_screen(&self) {
        let mut frame = self.screen.frame();
        Text::new(&self.mac_str, Point::new(1, 10), self.style)
            .draw(&mut frame)
            .unwrap();
        Text::new("RX Listening", Point::new(16, 36), self.style)
            .draw(&mut frame)
            .unwrap();
        self.draw_status(&mut frame);
        self.screen.show(frame);
    }

    fn draw_rx_audio_screen(&mut self, rssi: i16, snr: i16) {
        let mut frame = self.screen.frame();
        Text::new(&self.mac_str, Point::new(1, 10), self.style)
            .draw(&mut frame)
            .unwrap();
        Text::new("RX Audio", Point::new(28, 32), self.style)
            .draw(&mut frame)
            .unwrap();

        self.line_buf.clear();
        let _ = core::fmt::write(
            &mut self.line_buf,
            format_args!("RSSI:{} SNR:{}", rssi, snr),
        );
        Text::new(&self.line_buf, Point::new(0, 48), self.style)
            .draw(&mut frame)
            .unwrap();

        self.draw_status(&mut frame);
        self.screen.show(frame);
    }

    /// Bottom line of the RX screens: volume, and LOCKED when locked.
    fn draw_status(&self, frame: &mut Frame) {
        let mut vol = heapless::String::<8>::new();
        let _ = core::fmt::write(&mut vol, format_args!("Vol {}", speaker::volume()));
        Text::new(&vol, Point::new(1, 62), self.style)
            .draw(frame)
            .unwrap();
        if self.locked {
            Text::new("LOCKED", Point::new(91, 62), self.style)
                .draw(frame)
                .unwrap();
        }
    }

    /// Title, then up to 4 rows; the cursor row is inverted, current values get a *.
    fn draw_menu(&self, menu: &Menu) {
        const TOP: i32 = 13;
        const ROW_H: i32 = 11;
        const VISIBLE: usize = 4;

        let mut frame = self.screen.frame();
        Text::new(menu.title(), Point::new(1, 9), self.style)
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
                self.style
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
        self.screen.show(frame);
    }

    fn draw_tx_screen(&self) {
        let mut frame = self.screen.frame();
        Text::new(&self.mac_str, Point::new(1, 10), self.style)
            .draw(&mut frame)
            .unwrap();
        Text::new("TX Streaming", Point::new(10, 36), self.style)
            .draw(&mut frame)
            .unwrap();
        self.screen.show(frame);
    }
}

/// Generate 160ms squelch tail (white noise with fade-out), packet-sized.
fn generate_squelch() -> Arc<[i16]> {
    const MONO_SAMPLES: usize = FRAMES_PER_PACKET * CODEC2_FRAME_SAMPLES; // 1280
    const AMPLITUDE: i32 = 8000;
    let mut buf = vec![0i16; STEREO_PACKET_SAMPLES].into_boxed_slice();
    for i in 0..MONO_SAMPLES {
        let fade = (MONO_SAMPLES - i) as i32 * AMPLITUDE / MONO_SAMPLES as i32;
        let noise = ((unsafe { esp_idf_svc::sys::esp_random() } % (2 * fade as u32 + 1)) as i32
            - fade) as i16;
        buf[i * 2] = noise;
        buf[i * 2 + 1] = noise;
    }
    buf.into()
}

/// Split a packet (4 × 40ms stereo frames) into individual frames and send to speaker.
fn send_to_speaker(packet: &[i16]) {
    for i in 0..FRAMES_PER_PACKET {
        let offset = i * STEREO_FRAME_SAMPLES;
        let frame: Arc<[i16]> = packet[offset..offset + STEREO_FRAME_SAMPLES].into();
        if SPK_FRAMES.try_send(frame).is_err() {
            log::warn!("SPK queue full, dropped frame {} of packet", i);
        }
    }
}

/// Pack the 2-byte voice header: |5b type|7b txid|4b seq|
fn voice_header(txid: u8, seq: u8) -> [u8; 2] {
    ((PKT_TYPE_VOICE as u16) << 11 | (txid as u16) << 4 | seq as u16).to_be_bytes()
}

/// Capture one packet's worth of PCM (FRAMES_PER_PACKET frames, 160ms).
async fn capture_packet(mic: &mut Mic) -> Box<[i16]> {
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
        TX_CHAN.send(TxRequest { data: packet }).await;
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
