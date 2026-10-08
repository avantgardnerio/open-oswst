//! The microphone: 8kHz mono PCM, a frame at a time.

#[allow(async_fn_in_trait)] // used by one task at a time; Send never needed
pub trait Mic {
    /// Start sampling: the next read is fresh audio. Until then the mic holds
    /// no hardware (the board shares its ADC with the battery reading)
    fn start(&mut self);

    /// Stop sampling and let go of the hardware
    fn stop(&mut self);

    /// Fill `pcm` with the next samples. A short read, or a read while
    /// stopped, is zero-padded.
    async fn read(&mut self, pcm: &mut [i16]);
}
