use std::future::Future;

use esp_idf_svc::hal::gpio::AnyIOPin;
use esp_idf_svc::hal::i2s::config::{
    Config as I2sChannelConfig, DataBitWidth, SlotMode, StdClkConfig, StdConfig, StdGpioConfig,
    StdSlotConfig,
};
use esp_idf_svc::hal::i2s::{I2sDriver, I2sTx, I2S0};

use crate::{SPK_FRAMES, SPK_REQ};

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
    loop {
        let frame = SPK_FRAMES.receive().await;
        i2s_tx.write_async(pcm_as_bytes(&frame)).await.unwrap();

        if SPK_FRAMES.len() <= 1 {
            let _ = SPK_REQ.try_send(());
        }
    }
}
