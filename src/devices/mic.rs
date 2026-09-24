//! Microphone via continuous (DMA) ADC at 8kHz. Hands out signed 16-bit PCM
//! frames; knows nothing about codecs or packets.

use esp_idf_svc::hal::adc::continuous::config::Config as AdcContConfig;
use esp_idf_svc::hal::adc::continuous::{AdcDriver, AdcMeasurement, Attenuated};
use esp_idf_svc::hal::adc::ADC1;
use esp_idf_svc::hal::gpio::Gpio4;
use esp_idf_svc::hal::units::Hertz;

/// Samples per frame: 40ms at 8kHz
pub const FRAME_SAMPLES: usize = 320;

pub struct Peripherals {
    pub adc: ADC1<'static>,
    pub pin: Gpio4<'static>, // must stay concrete — ADCPin trait is pin-specific
}

pub struct Mic {
    adc: AdcDriver<'static>,
    buf: Box<[AdcMeasurement]>,
}

/// Convert 12-bit unsigned ADC sample to signed 16-bit PCM centered at 0.
fn adc_to_pcm(sample: &AdcMeasurement) -> i16 {
    (sample.data() as i16 - 2048) * 16
}

/// Configure the ADC and start sampling.
pub fn init(p: Peripherals) -> Mic {
    let config = AdcContConfig::new()
        .sample_freq(Hertz(8000))
        .frame_measurements(FRAME_SAMPLES)
        .frames_count(2); // double buffer

    let mut adc = AdcDriver::new(p.adc, &config, Attenuated::db12(p.pin)).unwrap();
    adc.start().unwrap();
    log::info!("Mic ADC DMA started at 8kHz");

    Mic {
        adc,
        buf: vec![AdcMeasurement::new(); FRAME_SAMPLES].into_boxed_slice(),
    }
}

impl Mic {
    /// Discard any samples already buffered, so the next read is fresh audio.
    pub fn drain(&mut self) {
        let _ = self.adc.read(&mut self.buf, 0);
    }

    /// Fill `pcm` with the next frame. A short read is zero-padded.
    pub async fn read(&mut self, pcm: &mut [i16]) {
        let count = self.adc.read_async(&mut self.buf).await.unwrap_or(0);
        let count = count.min(pcm.len());
        for (dst, sample) in pcm.iter_mut().zip(&self.buf[..count]) {
            *dst = adc_to_pcm(sample);
        }
        pcm[count..].fill(0);
    }
}
