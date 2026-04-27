use embassy_futures::select::{select3, Either3};
use embedded_graphics::mono_font::ascii::FONT_6X10;
use embedded_graphics::mono_font::MonoTextStyleBuilder;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use esp_idf_svc::hal::adc::continuous::config::Config as AdcContConfig;
use esp_idf_svc::hal::adc::continuous::{AdcDriver as AdcContDriver, AdcMeasurement, Attenuated};
use esp_idf_svc::hal::adc::ADC1;
use esp_idf_svc::hal::gpio::{AnyIOPin, Gpio7};
use esp_idf_svc::hal::gpio::{Input, PinDriver, Pull};
use esp_idf_svc::hal::i2c::config::Config as I2cConfig;
use esp_idf_svc::hal::i2c::{I2cDriver, I2C0};
use esp_idf_svc::hal::units::Hertz;
use ssd1306::mode::BufferedGraphicsMode;
use ssd1306::prelude::*;
use ssd1306::{I2CDisplayInterface, Ssd1306};
use std::future::Future;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use std::sync::atomic::Ordering;

use crate::codec::{
    CodecRequest, CodecResponse, CODEC2_FRAME_SAMPLES, CODEC_REPLY, FRAMES_PER_PACKET,
    HEADER_BYTES, PACKET_BYTES, PAYLOAD_BYTES, STEREO_PACKET_SAMPLES,
};
use crate::{TxRequest, IS_REPEATER, RX_CHAN, SPK_FRAMES, SPK_REQ, TX_CHAN};

/// Packet type constants (5 bits, upper bits of header)
const PKT_TYPE_VOICE: u8 = 0x00;

/// Stereo samples per Codec2 frame (320 mono × 2 channels)
const STEREO_FRAME_SAMPLES: usize = CODEC2_FRAME_SAMPLES * 2;

/// Convert 12-bit unsigned ADC sample to signed 16-bit PCM centered at 0.
fn adc_to_pcm(sample: &AdcMeasurement) -> i16 {
    (sample.data() as i16 - 2048) * 16
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
        let _ = SPK_FRAMES.try_send(frame);
    }
}

type Display<'a> = Ssd1306<
    I2CInterface<I2cDriver<'a>>,
    DisplaySize128x64,
    BufferedGraphicsMode<DisplaySize128x64>,
>;

pub struct Peripherals {
    pub ptt: AnyIOPin<'static>,
    pub audio_in: Gpio7<'static>, // must stay concrete — ADCPin trait is pin-specific
    pub adc: ADC1<'static>,
    pub i2c: I2C0<'static>,
    pub oled_sda: AnyIOPin<'static>,
    pub oled_scl: AnyIOPin<'static>,
    pub oled_rst: AnyIOPin<'static>,
}

pub async fn init(
    p: Peripherals,
    mac_str: heapless::String<18>,
    codec_tx: SyncSender<CodecRequest>,
) -> impl Future<Output = ()> {
    // PRG button on GPIO0 — active LOW with internal pull-up
    let button = PinDriver::input(p.ptt, Pull::Up).unwrap();

    // Continuous ADC for mic on GPIO7 (ADC1_CH6) — DMA at 8kHz
    let adc_config = AdcContConfig::new()
        .sample_freq(Hertz(8000))
        .frame_measurements(320) // 320 samples = 40ms at 8kHz (one Codec2 frame)
        .frames_count(2); // double buffer

    let mut adc = AdcContDriver::new(p.adc, &adc_config, Attenuated::db12(p.audio_in)).unwrap();

    // Reset OLED
    let mut oled_rst = PinDriver::output(p.oled_rst).unwrap();
    oled_rst.set_low().unwrap();
    thread::sleep(Duration::from_millis(50));
    oled_rst.set_high().unwrap();
    thread::sleep(Duration::from_millis(50));

    // I2C for OLED
    let i2c = I2cDriver::new(p.i2c, p.oled_sda, p.oled_scl, &I2cConfig::default()).unwrap();

    // OLED init
    let interface = I2CDisplayInterface::new(i2c);
    let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();
    display.init().unwrap();
    display.set_brightness(Brightness::BRIGHTEST).unwrap();
    log::info!("OLED initialized, MAC: {}", mac_str);

    // Pre-generate audio buffers
    let silence: Arc<[i16]> = vec![0i16; STEREO_PACKET_SAMPLES].into();
    let squelch = generate_squelch();

    // Start continuous ADC (DMA)
    adc.start().unwrap();
    log::info!("ADC DMA started at 8kHz");

    async move {
        // Keep oled_rst alive so the pin doesn't float low (holding OLED in reset)
        let _oled_rst = oled_rst;
        app_loop(button, adc, display, &mac_str, codec_tx, silence, squelch).await;
    }
}

async fn app_loop(
    mut button: PinDriver<'_, Input>,
    mut adc: AdcContDriver<'_>,
    mut display: Display<'_>,
    mac_str: &str,
    codec_tx: SyncSender<CodecRequest>,
    silence: Arc<[i16]>,
    squelch: Arc<[i16]>,
) {
    let style = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(BinaryColor::On)
        .build();

    let mut line_buf = heapless::String::<64>::new();
    let mut mic_buf = vec![AdcMeasurement::new(); 320].into_boxed_slice();
    let mut pcm_buf = vec![0i16; CODEC2_FRAME_SAMPLES].into_boxed_slice();

    // Track current transmitter for seq ordering
    let mut cur_txid: Option<u8> = None;
    let mut last_played_seq: u8 = 0;
    let mut seq_buf: [Option<Arc<[i16]>>; 16] = Default::default();
    let mut last_rx_time = Instant::now();
    let mut spk_active = false; // true once speaker has been kicked

    // Show initial RX state
    draw_rx_screen(&mut display, mac_str, &style);

    loop {
        // Auto-reset if no packet from current txid in 500ms
        if cur_txid.is_some() && last_rx_time.elapsed() > Duration::from_millis(500) {
            log::info!("RX timeout, resetting txid lock");
            send_to_speaker(&squelch);
            reset_rx_state(&mut cur_txid, &mut last_played_seq, &mut seq_buf);
            spk_active = false;
        }

        match select3(RX_CHAN.receive(), button.wait_for_low(), SPK_REQ.receive()).await {
            Either3::First(rx_pkt) => {
                if rx_pkt.data.len() < HEADER_BYTES {
                    log::warn!(
                        "RX [{}B] too short, rssi={} snr={}",
                        rx_pkt.data.len(),
                        rx_pkt.rssi,
                        rx_pkt.snr
                    );
                    continue;
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
                    continue;
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
                    send_to_speaker(&squelch);
                    reset_rx_state(&mut cur_txid, &mut last_played_seq, &mut seq_buf);
                    spk_active = false;
                    continue;
                }

                if rx_pkt.data.len() != PACKET_BYTES {
                    log::warn!("RX [{}B] unexpected size, ignoring", rx_pkt.data.len());
                    continue;
                }

                let payload = &rx_pkt.data[HEADER_BYTES..];

                if cur_txid.is_none() {
                    cur_txid = Some(txid);
                    last_played_seq = seq.wrapping_sub(1) & 0x0F;
                }

                if cur_txid != Some(txid) {
                    log::warn!(
                        "RX ignoring txid={} (locked to {})",
                        txid,
                        cur_txid.unwrap()
                    );
                    continue;
                }

                last_rx_time = Instant::now();
                let expected_seq = (last_played_seq.wrapping_add(1)) & 0x0F;
                let diff = (seq.wrapping_sub(expected_seq) & 0x0F) as i8;
                let diff = if diff > 7 { diff - 16 } else { diff };

                match diff {
                    -2..=-1 => {
                        log::info!("RX seq={} old (diff={}), dropping", seq, diff);
                        continue;
                    }
                    0..=2 => {
                        // Repeater: relay after dedup (non-duplicate voice)
                        if IS_REPEATER.load(Ordering::Relaxed) {
                            let mut relay = heapless::Vec::new();
                            let _ = relay.extend_from_slice(&rx_pkt.data);
                            TX_CHAN.send(TxRequest { data: relay }).await;
                            log::info!("RELAY [{}B] txid={} seq={}", rx_pkt.data.len(), txid, seq);
                            last_played_seq = seq;
                            continue; // skip decode — fast turnaround
                        }
                        // Send to codec thread for decode, await reply
                        let mut payload_arr = [0u8; PAYLOAD_BYTES];
                        payload_arr.copy_from_slice(payload);
                        codec_tx
                            .send(CodecRequest::decode(seq, txid, payload_arr))
                            .unwrap();
                        if let CodecResponse::Decoded { seq, txid, pcm } =
                            CODEC_REPLY.receive().await
                        {
                            if cur_txid == Some(txid) {
                                seq_buf[seq as usize] = Some(pcm.into());

                                // Kick speaker once we have 2 consecutive packets
                                if !spk_active {
                                    let next = (last_played_seq.wrapping_add(1)) & 0x0F;
                                    let next2 = (next.wrapping_add(1)) & 0x0F;
                                    if seq_buf[next as usize].is_some()
                                        && seq_buf[next2 as usize].is_some()
                                    {
                                        let pcm = seq_buf[next as usize].take().unwrap();
                                        last_played_seq = next;
                                        spk_active = true;
                                        send_to_speaker(&pcm);
                                    }
                                }
                            }
                        }
                    }
                    _ => {
                        log::warn!("RX seq={} unexpected (diff={}), resetting", seq, diff);
                        reset_rx_state(&mut cur_txid, &mut last_played_seq, &mut seq_buf);
                        continue;
                    }
                }

                log::info!(
                    "RX [{}B] txid={} seq={} played_to={} rssi={} snr={}",
                    rx_pkt.data.len(),
                    txid,
                    seq,
                    last_played_seq,
                    rx_pkt.rssi,
                    rx_pkt.snr,
                );

                draw_rx_audio_screen(
                    &mut display,
                    mac_str,
                    &style,
                    &mut line_buf,
                    rx_pkt.rssi,
                    rx_pkt.snr,
                );
            }
            Either3::Second(_) => {
                // PTT pressed — reset RX state
                reset_rx_state(&mut cur_txid, &mut last_played_seq, &mut seq_buf);
                spk_active = false;

                // Generate random 7-bit txid for this PTT press (dedup key)
                let txid = (unsafe { esp_idf_svc::sys::esp_random() } & 0x7F) as u8;
                let mut seq: u8 = 0;
                log::info!("PTT pressed — streaming (txid={})", txid);

                display.clear_buffer();
                Text::new(mac_str, Point::new(1, 10), style)
                    .draw(&mut display)
                    .unwrap();
                Text::new("TX Streaming", Point::new(10, 36), style)
                    .draw(&mut display)
                    .unwrap();
                display.flush().unwrap();

                let _ = adc.read(&mut mic_buf, 0); // drain stale

                while button.is_low() {
                    // Pack 2-byte header: |5b type|7b txid|4b seq|
                    let header: u16 =
                        (PKT_TYPE_VOICE as u16) << 11 | (txid as u16) << 4 | seq as u16;
                    let header_bytes = header.to_be_bytes();

                    // Read FRAMES_PER_PACKET frames of PCM
                    let total_samples = FRAMES_PER_PACKET * CODEC2_FRAME_SAMPLES;
                    let mut pcm = vec![0i16; total_samples].into_boxed_slice();
                    for i in 0..FRAMES_PER_PACKET {
                        let count = adc.read_async(&mut mic_buf).await.unwrap_or(0);
                        let start = i * CODEC2_FRAME_SAMPLES;
                        for (j, sample) in mic_buf[..count].iter().enumerate() {
                            pcm_buf[j] = adc_to_pcm(sample);
                        }
                        for s in pcm_buf[count..].iter_mut() {
                            *s = 0;
                        }
                        pcm[start..start + CODEC2_FRAME_SAMPLES]
                            .copy_from_slice(&pcm_buf[..CODEC2_FRAME_SAMPLES]);
                    }

                    // Send to codec thread, await encoded packet
                    codec_tx
                        .send(CodecRequest::encode(header_bytes, pcm))
                        .unwrap();
                    if let CodecResponse::Encoded { packet } = CODEC_REPLY.receive().await {
                        TX_CHAN.send(TxRequest { data: packet }).await;
                    }
                    seq = (seq + 1) & 0x0F; // wrap at 16
                }

                // Send header-only EOT packet
                let eot_header: u16 =
                    (PKT_TYPE_VOICE as u16) << 11 | (txid as u16) << 4 | seq as u16;
                let mut eot_data = heapless::Vec::new();
                let _ = eot_data.extend_from_slice(&eot_header.to_be_bytes());
                TX_CHAN.send(TxRequest { data: eot_data }).await;

                log::info!("PTT released — {} packets sent + EOT", seq);

                // Redraw RX screen
                draw_rx_screen(&mut display, mac_str, &style);
            }
            Either3::Third(_) => {
                // Speaker wants next audio
                let next = (last_played_seq.wrapping_add(1)) & 0x0F;
                if let Some(pcm) = seq_buf[next as usize].take() {
                    last_played_seq = next;
                    send_to_speaker(&pcm);
                } else if cur_txid.is_some() {
                    // Gap — skip this seq, send silence
                    // last_played_seq = next;
                    log::info!("SPK gap at seq={}, sending silence", next);
                    send_to_speaker(&silence);
                }
                // else: not receiving, nothing to send — DMA auto_clear handles silence
            }
        }
    }
}

fn reset_rx_state(
    cur_txid: &mut Option<u8>,
    last_played_seq: &mut u8,
    seq_buf: &mut [Option<Arc<[i16]>>; 16],
) {
    *cur_txid = None;
    *last_played_seq = 0;
    seq_buf.iter_mut().for_each(|s| *s = None);
}

fn draw_rx_audio_screen(
    display: &mut Display<'_>,
    mac_str: &str,
    style: &embedded_graphics::mono_font::MonoTextStyle<BinaryColor>,
    line_buf: &mut heapless::String<64>,
    rssi: i16,
    snr: i16,
) {
    display.clear_buffer();
    Text::new(mac_str, Point::new(1, 10), *style)
        .draw(display)
        .unwrap();
    Text::new("RX Audio", Point::new(28, 32), *style)
        .draw(display)
        .unwrap();

    line_buf.clear();
    let _ = core::fmt::write(line_buf, format_args!("RSSI:{} SNR:{}", rssi, snr));
    Text::new(line_buf, Point::new(0, 48), *style)
        .draw(display)
        .unwrap();

    display.flush().unwrap();
}

fn draw_rx_screen(
    display: &mut Display<'_>,
    mac_str: &str,
    style: &embedded_graphics::mono_font::MonoTextStyle<BinaryColor>,
) {
    display.clear_buffer();
    Text::new(mac_str, Point::new(1, 10), *style)
        .draw(display)
        .unwrap();
    Text::new("RX Listening", Point::new(16, 36), *style)
        .draw(display)
        .unwrap();
    display.flush().unwrap();
}
