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
use super::storage;
use crate::psram::PsramBuffer;
use std::io::Write;
use std::time::Instant;

/// Samples per frame: 40ms at 8kHz
pub const FRAME_SAMPLES: usize = 320;

/// Debug recorder: true = every sample read between start() and stop() goes
/// into PSRAM, and stop() writes it to /data/rec/rec-N.wav (8 kHz mono
/// 16-bit), fetched over WebDAV at /fs/rec/. It's the exact PCM the codec
/// thread gets, before the high-pass. Off by default: it holds 320 KB of
/// PSRAM and blocks stop() while the flash is written
const RECORD_TO_FLASH: bool = false;
const RECORD_RATE_HZ: u32 = 8000;
const RECORD_SECONDS: usize = 20;
const RECORD_BYTES: usize = RECORD_RATE_HZ as usize * RECORD_SECONDS * 2;

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
    // The debug recorder: little-endian i16 samples, and how many bytes of it
    // this transmission has filled. None if the PSRAM couldn't spare it
    recording: Option<PsramBuffer>,
    recorded_bytes: usize,
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
        recording: if RECORD_TO_FLASH {
            PsramBuffer::zeroed(RECORD_BYTES)
        } else {
            None
        },
        recorded_bytes: 0,
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
        self.recorded_bytes = 0;
    }

    /// Stop sampling and give ADC1 back.
    pub fn stop(&mut self) {
        let was_running = self.running.take().is_some();
        if was_running && RECORD_TO_FLASH {
            self.save_recording();
        }
    }

    /// The debug recorder: this transmission's samples to the next free
    /// /data/rec/rec-N.wav. Blocks the caller while the flash is written
    fn save_recording(&mut self) {
        let Some(recording) = &self.recording else {
            log::warn!("REC: no PSRAM buffer, nothing recorded");
            return;
        };
        let started = Instant::now();
        let folder = format!("{}/rec", storage::ROOT);
        let _ = std::fs::create_dir_all(&folder);
        let number = (1..)
            .find(|number| !std::path::Path::new(&format!("{folder}/rec-{number}.wav")).exists())
            .unwrap();
        let path = format!("{folder}/rec-{number}.wav");
        let data = &recording[..self.recorded_bytes];
        let written = std::fs::File::create(&path).and_then(|mut file| {
            file.write_all(&wav_header(data.len() as u32))?;
            file.write_all(data)
        });
        match written {
            Ok(()) => log::info!(
                "REC: {} samples ({:.2} s{}) -> {} in {} ms",
                data.len() / 2,
                data.len() as f32 / 2.0 / RECORD_RATE_HZ as f32,
                if data.len() == RECORD_BYTES {
                    ", buffer FULL"
                } else {
                    ""
                },
                path,
                started.elapsed().as_millis()
            ),
            Err(error) => log::warn!("REC: writing {} failed: {}", path, error),
        }
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

        if self.running.is_some() {
            if let Some(recording) = &mut self.recording {
                for sample in pcm.iter() {
                    if self.recorded_bytes + 2 > recording.len() {
                        break;
                    }
                    recording[self.recorded_bytes..self.recorded_bytes + 2]
                        .copy_from_slice(&sample.to_le_bytes());
                    self.recorded_bytes += 2;
                }
            }
        }
    }
}

/// A canonical 44-byte WAV header: PCM, mono, 16-bit, RECORD_RATE_HZ
fn wav_header(data_bytes: u32) -> [u8; 44] {
    let mut header = [0u8; 44];
    let byte_rate = RECORD_RATE_HZ * 2;
    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&(36 + data_bytes).to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(b"fmt ");
    header[16..20].copy_from_slice(&16u32.to_le_bytes());
    header[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    header[22..24].copy_from_slice(&1u16.to_le_bytes()); // mono
    header[24..28].copy_from_slice(&RECORD_RATE_HZ.to_le_bytes());
    header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    header[32..34].copy_from_slice(&2u16.to_le_bytes()); // bytes per sample
    header[34..36].copy_from_slice(&16u16.to_le_bytes()); // bits per sample
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&data_bytes.to_le_bytes());
    header
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
