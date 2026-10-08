//! Microphone via continuous (DMA) ADC at 8kHz. Hands out signed 16-bit PCM
//! frames; knows nothing about codecs or packets.
//!
//! Samples only between start() and stop() (the app: while PTT is held).
//! ADC1 is shared with the battery reading (adc1.rs), so the ADC driver is
//! made on start and dropped on stop.

use esp_idf_svc::hal::adc::continuous::config::Config as AdcContConfig;
use esp_idf_svc::hal::adc::continuous::{AdcDriver, AdcMeasurement, Attenuated};
use esp_idf_svc::hal::adc::ADC1;
use esp_idf_svc::hal::gpio::Gpio4;
use esp_idf_svc::hal::units::Hertz;

use super::adc1;

/// Samples per frame: 40ms at 8kHz
pub const FRAME_SAMPLES: usize = 320;

pub struct Peripherals {
    pub adc: ADC1<'static>,
    pub pin: Gpio4<'static>, // must stay concrete — ADCPin trait is pin-specific
}

pub struct Mic {
    // Held as ownership only: each start steals them for its driver (adc1.rs)
    _adc: ADC1<'static>,
    _pin: Gpio4<'static>,
    // Sampling, while started. The driver drops before the lease
    running: Option<(AdcDriver<'static>, adc1::Lease)>,
    buf: Box<[AdcMeasurement]>,
}

/// Convert 12-bit unsigned ADC sample to signed 16-bit PCM centered at 0.
fn adc_to_pcm(sample: &AdcMeasurement) -> i16 {
    (sample.data() as i16 - 2048) * 16
}

pub fn init(p: Peripherals) -> Mic {
    Mic {
        _adc: p.adc,
        _pin: p.pin,
        running: None,
        buf: vec![AdcMeasurement::new(); FRAME_SAMPLES].into_boxed_slice(),
    }
}

impl Mic {
    /// Take ADC1 and start sampling: the next read is fresh audio.
    pub fn start(&mut self) {
        if self.running.is_some() {
            return;
        }
        let Some(lease) = adc1::lease() else {
            log::warn!("Mic: ADC1 busy, not started");
            return;
        };
        let config = AdcContConfig::new()
            .sample_freq(Hertz(8000))
            .frame_measurements(FRAME_SAMPLES)
            .frames_count(2); // double buffer

        // SAFETY: the lease makes this the only ADC1 driver until stop()
        let (adc, pin) = unsafe { (ADC1::steal(), Gpio4::steal()) };
        let mut driver = AdcDriver::new(adc, &config, Attenuated::db12(pin)).unwrap();
        driver.start().unwrap();
        self.running = Some((driver, lease));
    }

    /// Stop sampling and give ADC1 back.
    pub fn stop(&mut self) {
        self.running = None;
    }

    /// Fill `pcm` with the next frame. A short read, or a read while
    /// stopped, is zero-padded.
    pub async fn read(&mut self, pcm: &mut [i16]) {
        let count = match &mut self.running {
            Some((driver, _)) => driver.read_async(&mut self.buf).await.unwrap_or(0),
            None => 0,
        };
        let count = count.min(pcm.len());
        for (dst, sample) in pcm.iter_mut().zip(&self.buf[..count]) {
            *dst = adc_to_pcm(sample);
        }
        pcm[count..].fill(0);
    }
}

impl open_oswst_core::devices::mic::Mic for Mic {
    fn start(&mut self) {
        Mic::start(self)
    }

    fn stop(&mut self) {
        Mic::stop(self)
    }

    async fn read(&mut self, pcm: &mut [i16]) {
        Mic::read(self, pcm).await
    }
}
