use embassy_futures::select::{select3, Either3};
use embedded_graphics::mono_font::ascii::FONT_6X10;
use embedded_graphics::mono_font::{MonoTextStyle, MonoTextStyleBuilder};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use esp_idf_svc::hal::gpio::AnyIOPin;
use esp_idf_svc::hal::gpio::{Input, PinDriver, Pull};
use open_oswst::devices::mic::Mic;
use open_oswst::devices::radio::{RxPacket, TxRequest, RX_CHAN, TX_CHAN};
use open_oswst::devices::screen::Screen;
use open_oswst::devices::speaker::{SPK_FRAMES, SPK_REQ};
use std::future::Future;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use std::sync::atomic::Ordering;

use crate::IS_REPEATER;
use open_oswst::codec::{
    CodecRequest, CodecResponse, CODEC2_FRAME_SAMPLES, CODEC_REPLY, FRAMES_PER_PACKET,
    HEADER_BYTES, PACKET_BYTES, PAYLOAD_BYTES, STEREO_PACKET_SAMPLES,
};

/// Packet type constants (5 bits, upper bits of header)
const PKT_TYPE_VOICE: u8 = 0x00;

/// Stereo samples per Codec2 frame (320 mono × 2 channels)
const STEREO_FRAME_SAMPLES: usize = CODEC2_FRAME_SAMPLES * 2;

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

pub struct Peripherals {
    pub ptt: AnyIOPin<'static>,
}

pub async fn init(
    p: Peripherals,
    mic: Mic,
    screen: Screen,
    mac_str: heapless::String<18>,
    codec_tx: SyncSender<CodecRequest>,
) -> impl Future<Output = ()> {
    // PRG button on GPIO0 — active LOW with internal pull-up
    let button = PinDriver::input(p.ptt, Pull::Up).unwrap();

    log::info!("MAC: {}", mac_str);

    let app = App {
        button,
        mic,
        screen,
        mac_str,
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
    };

    async move {
        let mut app = app;
        app.run().await;
    }
}

struct App {
    button: PinDriver<'static, Input>,
    mic: Mic,
    screen: Screen,
    mac_str: heapless::String<18>,
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

            match select3(
                RX_CHAN.receive(),
                self.button.wait_for_low(),
                SPK_REQ.receive(),
            )
            .await
            {
                Either3::First(rx_pkt) => self.on_rx_packet(rx_pkt).await,
                Either3::Second(_) => self.on_ptt().await,
                Either3::Third(_) => self.on_speaker_request(),
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

        // Pipelined: the codec thread encodes packet N while we capture N+1, so
        // the mic is drained continuously. At most one encode is outstanding.
        let mut encoding = false;
        let mut packets = 0usize;
        while self.button.is_low() {
            // Pack 2-byte header: |5b type|7b txid|4b seq|
            let header: u16 = (PKT_TYPE_VOICE as u16) << 11 | (txid as u16) << 4 | seq as u16;
            let header_bytes = header.to_be_bytes();

            // Read FRAMES_PER_PACKET frames of PCM
            let total_samples = FRAMES_PER_PACKET * CODEC2_FRAME_SAMPLES;
            let mut pcm = vec![0i16; total_samples].into_boxed_slice();
            for i in 0..FRAMES_PER_PACKET {
                let start = i * CODEC2_FRAME_SAMPLES;
                self.mic
                    .read(&mut pcm[start..start + CODEC2_FRAME_SAMPLES])
                    .await;
            }

            // Collect the previous packet's encode (ran during this capture) and send it
            if encoding {
                self.send_encoded().await;
            }

            // Hand this packet to the codec thread; collected next iteration
            self.codec_tx
                .send(CodecRequest::encode(header_bytes, pcm))
                .unwrap();
            encoding = true;
            packets += 1;
            seq = (seq + 1) & 0x0F; // wrap at 16
        }

        // Flush the last packet still in the encoder
        if encoding {
            self.send_encoded().await;
        }

        // Send header-only EOT packet
        let eot_header: u16 = (PKT_TYPE_VOICE as u16) << 11 | (txid as u16) << 4 | seq as u16;
        let mut eot_data = heapless::Vec::new();
        let _ = eot_data.extend_from_slice(&eot_header.to_be_bytes());
        TX_CHAN.send(TxRequest { data: eot_data }).await;

        log::info!("PTT released — {} packets sent + EOT", packets);

        // Redraw RX screen
        self.draw_rx_screen();
    }

    /// Await the outstanding encode and queue it for the radio.
    async fn send_encoded(&mut self) {
        let t_wait = Instant::now();
        let reply = CODEC_REPLY.receive().await;
        // Any wait here means encoding took longer than a packet's capture, so
        // the mic went undrained and audio was lost
        let wait_ms = t_wait.elapsed().as_millis();
        if wait_ms > 10 {
            log::warn!("TX encoder behind: waited {}ms", wait_ms);
        }
        if let CodecResponse::Encoded { packet } = reply {
            TX_CHAN.send(TxRequest { data: packet }).await;
        }
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

    fn reset_rx_state(&mut self) {
        self.cur_txid = None;
        self.last_played_seq = 0;
        self.seq_buf.iter_mut().for_each(|s| *s = None);
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

        self.screen.show(frame);
    }

    fn draw_rx_screen(&self) {
        let mut frame = self.screen.frame();
        Text::new(&self.mac_str, Point::new(1, 10), self.style)
            .draw(&mut frame)
            .unwrap();
        Text::new("RX Listening", Point::new(16, 36), self.style)
            .draw(&mut frame)
            .unwrap();
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
