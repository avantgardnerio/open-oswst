use std::future::Future;

use esp_idf_svc::hal::gpio::AnyIOPin;
use esp_idf_svc::hal::i2s::config::{
    Config as I2sChannelConfig, DataBitWidth, SlotMode, StdClkConfig, StdConfig, StdGpioConfig,
    StdSlotConfig, StdSlotMask,
};
use esp_idf_svc::hal::i2s::{I2sDriver, I2sTx, I2S0};

// The queues and volume the app talks to; this driver plays them over I2S
pub use open_oswst_core::devices::speaker::{
    note_underrun, scale, set_volume, volume, FRAME_SAMPLES, MAX_VOLUME, SPK_AUDIO,
};

pub struct Peripherals {
    pub i2s: I2S0<'static>,
    pub spk_bclk: AnyIOPin<'static>,
    pub spk_din: AnyIOPin<'static>,
    pub spk_ws: AnyIOPin<'static>,
}

/// Codec2 and the app run at 8 kHz; I2S runs at twice that, because the
/// PCM5102A's PLL (SCK grounded) can't lock below 16 kHz (datasheet table
/// 11). The MAX98357A takes 16 kHz too, so one firmware drives either
const I2S_RATE_HZ: u32 = 16000;
const UPSAMPLE: usize = 2;

fn pcm_as_bytes(pcm: &[i32]) -> &[u8] {
    unsafe { core::slice::from_raw_parts(pcm.as_ptr() as *const u8, pcm.len() * 4) }
}

/// 8 kHz 16-bit samples -> 16 kHz in 32-bit slots (the sample in the top
/// half): each sample, then the midpoint to the next. `previous` carries the
/// last sample across frames, so frame edges interpolate too
fn upsample(pcm: &[i16], previous: &mut i16, out: &mut Vec<i32>) {
    out.clear();
    for &sample in pcm {
        let midpoint = (*previous as i32 + sample as i32) / 2;
        out.push(midpoint << 16);
        out.push((sample as i32) << 16);
        *previous = sample;
    }
}

pub async fn init(p: Peripherals) -> impl Future<Output = ()> {
    // 2 DMA buffers: one playing, one being filled. write_async on the 2nd
    // blocks until DMA finishes the 1st — gives us 40ms pacing.
    let i2s_chan_cfg = I2sChannelConfig::new()
        .dma_buffer_count(2)
        .frames_per_buffer((FRAME_SAMPLES * UPSAMPLE) as u32)
        .auto_clear(true);
    let std_config = StdConfig::new(
        i2s_chan_cfg,
        StdClkConfig::from_sample_rate_hz(I2S_RATE_HZ),
        // Mono, sent on both slots: the I2S hardware repeats each sample on
        // left and right, so our buffers hold mono, half the memory. Not the
        // default mono mask (Left only), which sends zeros on the right: the
        // MAX98357A plays (L + R) / 2, and it came out 6 dB quiet. 32-bit
        // slots: BCLK = 64 x the rate, which the PCM5102A's PLL needs
        StdSlotConfig::philips_slot_default(DataBitWidth::Bits32, SlotMode::Mono)
            .slot_mode_mask(SlotMode::Mono, StdSlotMask::Both),
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
    log::info!("I2S TX configured (16kHz mono on both slots, 32-bit Philips, 2 DMA bufs)");

    async move {
        i2s_tx.tx_enable().unwrap();
        log::info!("I2S TX enabled");
        speaker_loop(i2s_tx).await;
    }
}

/// Play SPK_AUDIO back to back, a 40 ms frame at a time: each write waits
/// for a DMA buffer to come free, so the I2S clock sets the pace. Dry, it
/// waits for the next audio, and the DMA plays silence meanwhile
async fn speaker_loop(mut i2s_tx: I2sDriver<'_, I2sTx>) {
    let mut last_frame = std::time::Instant::now();
    let mut scaled: Vec<i16> = Vec::new();
    let mut upsampled: Vec<i32> = Vec::new();
    let mut previous: i16 = 0;
    loop {
        // Underrun: mid-stream (a frame played recently) but nothing was
        // queued, so the DMA ran dry waiting for the next audio
        let starved = SPK_AUDIO.is_empty();
        let wait_start = std::time::Instant::now();
        let audio = SPK_AUDIO.receive().await;
        let waited = wait_start.elapsed().as_millis();
        if starved && last_frame.elapsed().as_millis() < 500 && waited > 5 {
            log::warn!("SPK underrun: waited {}ms for next frame", waited);
            note_underrun(waited as u32);
        }

        for frame in audio.chunks(FRAME_SAMPLES) {
            scale(frame, &mut scaled);
            upsample(&scaled, &mut previous, &mut upsampled);
            i2s_tx.write_async(pcm_as_bytes(&upsampled)).await.unwrap();
        }
        last_frame = std::time::Instant::now();
    }
}
