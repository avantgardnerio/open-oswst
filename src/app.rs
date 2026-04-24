use codec2::{Codec2, Codec2Mode};
use embassy_futures::select::{select, Either};
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
use esp_idf_svc::hal::i2s::config::{
    Config as I2sChannelConfig, DataBitWidth, SlotMode, StdClkConfig, StdConfig, StdGpioConfig,
    StdSlotConfig,
};
use esp_idf_svc::hal::i2s::{I2sDriver, I2sTx, I2S0};
use esp_idf_svc::hal::units::Hertz;
use ssd1306::mode::BufferedGraphicsMode;
use ssd1306::prelude::*;
use ssd1306::{I2CDisplayInterface, Ssd1306};
use std::future::Future;
use std::thread;
use std::time::{Duration, Instant};

use std::sync::atomic::Ordering;

use crate::{TxRequest, CHAN_USE_PCT, RX_CHAN, TX_CHAN};

/// Codec2 MODE_1200: 320 samples → 6 bytes per frame
const CODEC2_FRAME_BYTES: usize = 6;
const CODEC2_FRAME_SAMPLES: usize = 320;

/// Pack 4 Codec2 frames per LoRa packet (24 bytes payload, 160ms audio).
/// 2-byte header for repeater dedup/reorder: |5b type|7b txid|4b seq| = 16 bits.
/// 26 bytes total sits in the same SF8 symbol bin as 24 — zero air time cost.
const FRAMES_PER_PACKET: usize = 4;
const HEADER_BYTES: usize = 2;
const PAYLOAD_BYTES: usize = CODEC2_FRAME_BYTES * FRAMES_PER_PACKET; // 24
const PACKET_BYTES: usize = HEADER_BYTES + PAYLOAD_BYTES; // 26

/// Packet type constants (5 bits, upper bits of header)
const PKT_TYPE_VOICE: u8 = 0x00;

/// Convert i16 PCM slice to &[u8] for I2S write.
fn pcm_as_bytes(pcm: &[i16]) -> &[u8] {
    unsafe { core::slice::from_raw_parts(pcm.as_ptr() as *const u8, pcm.len() * 2) }
}

/// Convert 12-bit unsigned ADC sample to signed 16-bit PCM centered at 0.
fn adc_to_pcm(sample: &AdcMeasurement) -> i16 {
    (sample.data() as i16 - 2048) * 16
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
    pub i2s: I2S0<'static>,
    pub spk_bclk: AnyIOPin<'static>,
    pub spk_din: AnyIOPin<'static>,
    pub spk_ws: AnyIOPin<'static>,
}

pub async fn init(p: Peripherals, mac_str: heapless::String<18>) -> impl Future<Output = ()> {
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

    // I2S TX for speaker (MAX98357A, Philips format, 8kHz mono 16-bit)
    // Larger DMA buffers + auto_clear to avoid underrun clicks
    let i2s_chan_cfg = I2sChannelConfig::new()
        .dma_buffer_count(8)
        .frames_per_buffer(320) // 40ms per buffer = one Codec2 frame
        .auto_clear(true);
    let std_config = StdConfig::new(
        i2s_chan_cfg,
        StdClkConfig::from_sample_rate_hz(8000),
        StdSlotConfig::philips_slot_default(DataBitWidth::Bits16, SlotMode::Stereo),
        StdGpioConfig::default(),
    );
    let mut i2s_tx = I2sDriver::<I2sTx>::new_std_tx(
        p.i2s,
        &std_config,
        p.spk_bclk,
        p.spk_din,
        None::<AnyIOPin>,
        p.spk_ws,
    )
    .unwrap();
    // Don't tx_enable yet — Codec2 init blocks for seconds and would starve DMA
    log::info!("I2S TX configured (8kHz mono 16-bit Philips)");

    // Codec2 encoder + decoder (MODE_1200: 320 samples → 6 bytes)
    // Box to avoid bloating the async future's stack frame
    let encoder = Box::new(Codec2::new(Codec2Mode::MODE_1200));
    log::info!("Codec2 encoder initialized");
    let decoder = Box::new(Codec2::new(Codec2Mode::MODE_1200));
    log::info!("Codec2 decoder initialized ({}B/frame)", CODEC2_FRAME_BYTES);

    // Start continuous ADC (DMA)
    adc.start().unwrap();
    log::info!("ADC DMA started at 8kHz");

    async move {
        // Keep oled_rst alive so the pin doesn't float low (holding OLED in reset)
        let _oled_rst = oled_rst;
        // Enable I2S TX now — after Codec2 init so DMA doesn't starve
        i2s_tx.tx_enable().unwrap();
        log::info!("I2S TX enabled");
        app_loop(button, adc, i2s_tx, display, &mac_str, encoder, decoder).await;
    }
}

async fn app_loop(
    mut button: PinDriver<'_, Input>,
    mut adc: AdcContDriver<'_>,
    mut i2s_tx: I2sDriver<'_, I2sTx>,
    mut display: Display<'_>,
    mac_str: &str,
    mut encoder: Box<Codec2>,
    mut decoder: Box<Codec2>,
) {
    let style = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(BinaryColor::On)
        .build();

    let mut tx_count: u32 = 0;
    let mut line_buf = heapless::String::<64>::new();
    // Heap-allocate audio buffers — keeps async future small, ready for codec thread
    let mut mic_buf = vec![AdcMeasurement::new(); 320].into_boxed_slice();
    let mut pcm_buf = vec![0i16; CODEC2_FRAME_SAMPLES].into_boxed_slice();
    let mut decode_buf = vec![0i16; CODEC2_FRAME_SAMPLES].into_boxed_slice();

    // Track current transmitter for seq ordering
    let mut cur_txid: Option<u8> = None;
    let mut last_played_seq: u8 = 0;
    // Buffered decoded stereo PCM per seq slot (4 frames × 320 samples × 2 channels)
    const STEREO_PACKET_SAMPLES: usize = FRAMES_PER_PACKET * CODEC2_FRAME_SAMPLES * 2;
    let mut seq_buf: [Option<Box<[i16]>>; 16] = Default::default();
    let mut last_rx_time = Instant::now();

    // Show initial RX state
    draw_rx_screen(&mut display, mac_str, &style, &mut line_buf);

    loop {
        // Auto-reset if no packet from current txid in 500ms
        if cur_txid.is_some() && last_rx_time.elapsed() > Duration::from_millis(500) {
            log::info!("RX timeout, resetting txid lock");
            reset_rx_state(&mut cur_txid, &mut last_played_seq, &mut seq_buf);
        }

        match select(RX_CHAN.receive(), button.wait_for_low()).await {
            Either::First(rx_pkt) => {
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

                // Header-only = end of transmission
                if rx_pkt.data.len() == HEADER_BYTES {
                    log::info!("RX EOT from txid={}", txid);
                    reset_rx_state(&mut cur_txid, &mut last_played_seq, &mut seq_buf);
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
                        // Decode all frames now, buffer stereo PCM
                        let mut pcm = vec![0i16; STEREO_PACKET_SAMPLES].into_boxed_slice();
                        for i in 0..FRAMES_PER_PACKET {
                            let coded =
                                &payload[i * CODEC2_FRAME_BYTES..(i + 1) * CODEC2_FRAME_BYTES];
                            decoder.decode(&mut decode_buf, coded);
                            let offset = i * CODEC2_FRAME_SAMPLES * 2;
                            for (j, &sample) in decode_buf.iter().enumerate() {
                                pcm[offset + j * 2] = sample;
                                pcm[offset + j * 2 + 1] = sample;
                            }
                        }
                        seq_buf[seq as usize] = Some(pcm);
                    }
                    _ => {
                        log::warn!("RX seq={} unexpected (diff={}), resetting", seq, diff);
                        reset_rx_state(&mut cur_txid, &mut last_played_seq, &mut seq_buf);
                        continue;
                    }
                }

                let mut play_seq = (last_played_seq.wrapping_add(1)) & 0x0F;
                while let Some(pcm) = seq_buf[play_seq as usize].take() {
                    let _ = i2s_tx.write_all(
                        pcm_as_bytes(&pcm),
                        esp_idf_svc::hal::delay::TickType::new_millis(5).into(),
                    );
                    last_played_seq = play_seq;
                    play_seq = (play_seq.wrapping_add(1)) & 0x0F;
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
            Either::Second(_) => {
                // PTT pressed — stream: read+encode 4 frames, send packet, repeat
                reset_rx_state(&mut cur_txid, &mut last_played_seq, &mut seq_buf);
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

                let mut pkt_buf = [0u8; PACKET_BYTES];
                while button.is_low() {
                    // Pack 2-byte header: |5b type|7b txid|4b seq|
                    let header: u16 =
                        (PKT_TYPE_VOICE as u16) << 11 | (txid as u16) << 4 | seq as u16;
                    let [h0, h1] = header.to_be_bytes();
                    pkt_buf[0] = h0;
                    pkt_buf[1] = h1;

                    // Read and encode FRAMES_PER_PACKET frames into payload area
                    for i in 0..FRAMES_PER_PACKET {
                        let count = adc.read_async(&mut mic_buf).await.unwrap_or(0);
                        for (j, sample) in mic_buf[..count].iter().enumerate() {
                            pcm_buf[j] = adc_to_pcm(sample);
                        }
                        for s in pcm_buf[count..].iter_mut() {
                            *s = 0;
                        }
                        let offset = HEADER_BYTES + i * CODEC2_FRAME_BYTES;
                        encoder.encode(&mut pkt_buf[offset..offset + CODEC2_FRAME_BYTES], &pcm_buf);
                    }

                    // Send 26-byte packet (2B header + 24B voice) to radio
                    let mut data = heapless::Vec::new();
                    let _ = data.extend_from_slice(&pkt_buf);
                    TX_CHAN.send(TxRequest { data }).await;
                    tx_count += 1;
                    seq = (seq + 1) & 0x0F; // wrap at 16
                }

                // Send header-only EOT packet
                let eot_header: u16 =
                    (PKT_TYPE_VOICE as u16) << 11 | (txid as u16) << 4 | seq as u16;
                let mut eot_data = heapless::Vec::new();
                let _ = eot_data.extend_from_slice(&eot_header.to_be_bytes());
                TX_CHAN.send(TxRequest { data: eot_data }).await;

                log::info!("PTT released — {} packets sent + EOT", tx_count);

                // Redraw RX screen
                draw_rx_screen(&mut display, mac_str, &style, &mut line_buf);
            }
        }
    }
}

fn reset_rx_state(
    cur_txid: &mut Option<u8>,
    last_played_seq: &mut u8,
    seq_buf: &mut [Option<Box<[i16]>>; 16],
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

    let pct = CHAN_USE_PCT.load(Ordering::Relaxed);
    line_buf.clear();
    let _ = core::fmt::write(line_buf, format_args!("CH:{}%", pct));
    Text::new(line_buf, Point::new(0, 60), *style)
        .draw(display)
        .unwrap();
    display.flush().unwrap();
}

fn draw_rx_screen(
    display: &mut Display<'_>,
    mac_str: &str,
    style: &embedded_graphics::mono_font::MonoTextStyle<BinaryColor>,
    line_buf: &mut heapless::String<64>,
) {
    let pct = CHAN_USE_PCT.load(Ordering::Relaxed);
    display.clear_buffer();
    Text::new(mac_str, Point::new(1, 10), *style)
        .draw(display)
        .unwrap();
    Text::new("RX Listening", Point::new(16, 36), *style)
        .draw(display)
        .unwrap();
    line_buf.clear();
    let _ = core::fmt::write(line_buf, format_args!("CH:{}%", pct));
    Text::new(line_buf, Point::new(1, 54), *style)
        .draw(display)
        .unwrap();
    display.flush().unwrap();
}
