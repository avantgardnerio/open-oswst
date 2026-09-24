use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use std::future::Future;
use std::sync::Arc;

use esp_idf_svc::hal::gpio::AnyIOPin;
use esp_idf_svc::hal::i2s::config::{
    Config as I2sChannelConfig, DataBitWidth, SlotMode, StdClkConfig, StdConfig, StdGpioConfig,
    StdSlotConfig,
};
use esp_idf_svc::hal::i2s::{I2sDriver, I2sTx, I2S0};

/// Speaker requests next audio packet from app
pub static SPK_REQ: Channel<CriticalSectionRawMutex, (), 1> = Channel::new();

/// Audio frames for speaker — each is one 40ms stereo frame (640 i16).
/// Capacity 8 = 2 packets worth of frames.
pub static SPK_FRAMES: Channel<CriticalSectionRawMutex, Arc<[i16]>, 8> = Channel::new();

pub struct Peripherals {
    pub i2s: I2S0<'static>,
    pub spk_bclk: AnyIOPin<'static>,
    pub spk_din: AnyIOPin<'static>,
    pub spk_ws: AnyIOPin<'static>,
}

fn pcm_as_bytes(pcm: &[i16]) -> &[u8] {
    unsafe { core::slice::from_raw_parts(pcm.as_ptr() as *const u8, pcm.len() * 2) }
}

pub async fn init(p: Peripherals) -> impl Future<Output = ()> {
    // 2 DMA buffers: one playing, one being filled. write_async on the 2nd
    // blocks until DMA finishes the 1st — gives us 40ms pacing.
    let i2s_chan_cfg = I2sChannelConfig::new()
        .dma_buffer_count(2)
        .frames_per_buffer(320)
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
    log::info!("I2S TX configured (8kHz stereo 16-bit Philips, 2 DMA bufs)");

    async move {
        i2s_tx.tx_enable().unwrap();
        log::info!("I2S TX enabled");
        speaker_loop(i2s_tx).await;
    }
}

async fn speaker_loop(mut i2s_tx: I2sDriver<'_, I2sTx>) {
    let mut last_frame = std::time::Instant::now();
    loop {
        // Underrun: mid-stream (a frame played recently) but the queue was
        // empty, so the DMA ran dry waiting for the next frame
        let starved = SPK_FRAMES.is_empty();
        let wait_start = std::time::Instant::now();
        let frame = SPK_FRAMES.receive().await;
        let waited = wait_start.elapsed().as_millis();
        if starved && last_frame.elapsed().as_millis() < 500 && waited > 5 {
            log::warn!("SPK underrun: waited {}ms for next frame", waited);
        }
        last_frame = std::time::Instant::now();
        i2s_tx.write_async(pcm_as_bytes(&frame)).await.unwrap();

        if SPK_FRAMES.len() <= 1 {
            let _ = SPK_REQ.try_send(());
        }
    }
}
