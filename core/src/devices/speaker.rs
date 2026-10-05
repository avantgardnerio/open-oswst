//! The speaker, as one queue of audio to play (SPK_AUDIO). Also the volume,
//! which the driver applies with `scale`.

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;

/// Audio for the speaker, played in order, back to back: stereo interleaved
/// (L, R, L, R…), any whole number of 40 ms frames (640 samples each). The
/// app sends whole decoded packets (160 ms), the bringup bins single frames.
/// When it runs dry the speaker just stops (the DMA plays silence) and starts
/// again with the next audio: the I2S clock sets the pace, nobody schedules
/// it. Small, so a backlog can't eat the heap: 4 packets is ~20 KB
pub static SPK_AUDIO: Channel<CriticalSectionRawMutex, Arc<[i16]>, 4> = Channel::new();

/// Samples in one 40 ms stereo frame: what the speaker hands the DMA at a time
pub const FRAME_SAMPLES: usize = 640;

/// Volume levels 0 (mute) ..= MAX_VOLUME (full scale), 3dB apart.
pub const MAX_VOLUME: u8 = 10;

/// Q15 gain per level: 10^(-3dB * (MAX_VOLUME - level) / 20)
const GAIN_Q15: [i32; MAX_VOLUME as usize + 1] = [
    0, 1464, 2067, 2920, 4125, 5827, 8231, 11627, 16423, 23198, 32767,
];

static VOLUME: AtomicU8 = AtomicU8::new(7);

/// Set the output level, clamped to 0..=MAX_VOLUME. Applies from the next frame.
pub fn set_volume(level: u8) {
    VOLUME.store(level.min(MAX_VOLUME), Ordering::Relaxed);
}

pub fn volume() -> u8 {
    VOLUME.load(Ordering::Relaxed)
}

/// Underruns since the last `take_underruns`: how many, and ms spent waiting
static UNDERRUNS: AtomicU32 = AtomicU32::new(0);
static UNDERRUN_MS: AtomicU32 = AtomicU32::new(0);

/// The driver ran dry and waited `ms` for the next frame
pub fn note_underrun(ms: u32) {
    UNDERRUNS.fetch_add(1, Ordering::Relaxed);
    UNDERRUN_MS.fetch_add(ms, Ordering::Relaxed);
}

/// Underruns (count, total ms) since the last call
pub fn take_underruns() -> (u32, u32) {
    (
        UNDERRUNS.swap(0, Ordering::Relaxed),
        UNDERRUN_MS.swap(0, Ordering::Relaxed),
    )
}

/// Copy `frame` into `out` at the current volume. Frames are shared (Arc),
/// so the driver scales into its own scratch buffer.
pub fn scale(frame: &[i16], out: &mut Vec<i16>) {
    let gain = GAIN_Q15[volume() as usize];
    out.clear();
    out.extend(frame.iter().map(|&s| ((s as i32 * gain) >> 15) as i16));
}
