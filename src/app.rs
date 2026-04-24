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

use crate::{TxRequest, RX_CHAN, TX_CHAN};

/// Codec2 MODE_1200: 320 samples → 6 bytes per frame
const CODEC2_FRAME_BYTES: usize = 6;
const CODEC2_FRAME_SAMPLES: usize = 320;

/// Pack 4 Codec2 frames per LoRa packet (24 bytes, 160ms audio).
/// At SF8/125kHz a 6-byte packet takes ~61ms (>40ms audio = broken),
/// but 24 bytes takes ~113ms for 160ms audio = 47ms of slack.
const FRAMES_PER_PACKET: usize = 4;
const PACKET_BYTES: usize = CODEC2_FRAME_BYTES * FRAMES_PER_PACKET; // 24

/// Jitter buffer: accumulate this many decoded frames before starting I2S playback.
/// 4 frames × 40ms = 160ms (one packet's worth of lead time).
const JITTER_FRAMES: usize = FRAMES_PER_PACKET;

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
    let mut codec_buf = vec![0u8; CODEC2_FRAME_BYTES].into_boxed_slice();
    let mut decode_buf = vec![0i16; CODEC2_FRAME_SAMPLES].into_boxed_slice();

    // Jitter buffer: ring of stereo frames, written to I2S once we have enough lead time
    let mut jitter_buf: Vec<Box<[i16]>> = Vec::with_capacity(JITTER_FRAMES + 4);
    let mut jitter_playing = false;

    // Show initial RX state
    draw_rx_screen(&mut display, mac_str, &style);

    loop {
        match select(RX_CHAN.receive(), button.wait_for_low()).await {
            Either::First(rx_pkt) => {
                if rx_pkt.data.len() == PACKET_BYTES {
                    // Unpack and decode all frames in this packet
                    let t0 = Instant::now();
                    for i in 0..FRAMES_PER_PACKET {
                        let coded = &rx_pkt.data
                            [i * CODEC2_FRAME_BYTES..(i + 1) * CODEC2_FRAME_BYTES];
                        decoder.decode(&mut decode_buf, coded);

                        // Interleave mono→stereo into a new heap buffer
                        let mut frame =
                            vec![0i16; CODEC2_FRAME_SAMPLES * 2].into_boxed_slice();
                        for (j, &sample) in decode_buf.iter().enumerate() {
                            frame[j * 2] = sample;
                            frame[j * 2 + 1] = sample;
                        }
                        jitter_buf.push(frame);
                    }
                    let decode_ms = t0.elapsed().as_millis();

                    if !jitter_playing && jitter_buf.len() >= JITTER_FRAMES {
                        log::info!(
                            "Jitter buffer ready ({} frames), starting playback",
                            jitter_buf.len()
                        );
                        jitter_playing = true;
                    }

                    if jitter_playing {
                        for f in jitter_buf.drain(..) {
                            i2s_tx
                                .write_all(
                                    pcm_as_bytes(&f),
                                    esp_idf_svc::hal::delay::BLOCK,
                                )
                                .unwrap();
                        }
                    }

                    log::info!(
                        "RX [{}B] rssi={} snr={} dec={}ms jbuf={}",
                        rx_pkt.data.len(),
                        rx_pkt.rssi,
                        rx_pkt.snr,
                        decode_ms,
                        jitter_buf.len()
                    );

                    display.clear_buffer();
                    Text::new(mac_str, Point::new(1, 10), style)
                        .draw(&mut display)
                        .unwrap();
                    Text::new("RX Audio", Point::new(28, 32), style)
                        .draw(&mut display)
                        .unwrap();

                    line_buf.clear();
                    let _ = core::fmt::write(
                        &mut line_buf,
                        format_args!("RSSI:{} SNR:{}", rx_pkt.rssi, rx_pkt.snr),
                    );
                    Text::new(&line_buf, Point::new(0, 48), style)
                        .draw(&mut display)
                        .unwrap();
                    display.flush().unwrap();
                } else {
                    log::warn!(
                        "RX [{}B] unexpected size, rssi={} snr={}",
                        rx_pkt.data.len(),
                        rx_pkt.rssi,
                        rx_pkt.snr
                    );
                }
            }
            Either::Second(_) => {
                // PTT button pressed — record raw PCM, encode+TX on release
                log::info!("PTT pressed — recording");
                jitter_buf.clear();
                jitter_playing = false;

                display.clear_buffer();
                Text::new(mac_str, Point::new(1, 10), style)
                    .draw(&mut display)
                    .unwrap();
                Text::new("Recording...", Point::new(10, 36), style)
                    .draw(&mut display)
                    .unwrap();
                display.flush().unwrap();

                // Record raw PCM into a heap buffer (max 5s)
                const MAX_REC_SAMPLES: usize = 8000 * 5;
                let mut rec_buf = vec![0i16; MAX_REC_SAMPLES];
                let mut rec_len: usize = 0;

                let _ = adc.read(&mut mic_buf, 0); // drain stale

                while button.is_low() && rec_len < MAX_REC_SAMPLES {
                    let count = adc.read_async(&mut mic_buf).await.unwrap_or(0);
                    let n = count.min(MAX_REC_SAMPLES - rec_len);
                    for i in 0..n {
                        rec_buf[rec_len + i] = adc_to_pcm(&mic_buf[i]);
                    }
                    rec_len += n;
                }

                // Round down to whole packets (FRAMES_PER_PACKET Codec2 frames each)
                let num_frames = rec_len / CODEC2_FRAME_SAMPLES;
                let num_packets = num_frames / FRAMES_PER_PACKET;
                log::info!(
                    "PTT released — {}ms, {} frames, {} packets",
                    rec_len / 8,
                    num_frames,
                    num_packets
                );

                display.clear_buffer();
                Text::new(mac_str, Point::new(1, 10), style)
                    .draw(&mut display)
                    .unwrap();
                line_buf.clear();
                let _ = core::fmt::write(
                    &mut line_buf,
                    format_args!("TX {} pkts", num_packets),
                );
                Text::new(&line_buf, Point::new(10, 36), style)
                    .draw(&mut display)
                    .unwrap();
                display.flush().unwrap();

                // Encode FRAMES_PER_PACKET frames per LoRa packet
                let mut pkt_buf = [0u8; PACKET_BYTES];
                for p in 0..num_packets {
                    for i in 0..FRAMES_PER_PACKET {
                        let f = p * FRAMES_PER_PACKET + i;
                        let pcm = &rec_buf
                            [f * CODEC2_FRAME_SAMPLES..(f + 1) * CODEC2_FRAME_SAMPLES];
                        encoder.encode(
                            &mut pkt_buf[i * CODEC2_FRAME_BYTES..(i + 1) * CODEC2_FRAME_BYTES],
                            pcm,
                        );
                    }

                    let mut data = heapless::Vec::new();
                    let _ = data.extend_from_slice(&pkt_buf);
                    TX_CHAN.send(TxRequest { data }).await;

                    tx_count += 1;
                }

                log::info!("TX done — {} packets sent", num_packets);

                // Redraw RX screen
                draw_rx_screen(&mut display, mac_str, &style);
            }
        }
    }
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
