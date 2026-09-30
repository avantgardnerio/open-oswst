//! The microphone: 8kHz mono PCM, a frame at a time.

#[allow(async_fn_in_trait)] // used by one task at a time; Send never needed
pub trait Mic {
    /// Discard any samples already buffered, so the next read is fresh audio.
    fn drain(&mut self);

    /// Fill `pcm` with the next samples. A short read is zero-padded.
    async fn read(&mut self, pcm: &mut [i16]);
}
