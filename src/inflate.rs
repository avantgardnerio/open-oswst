//! Inflating (gzip's deflate) with the copy of miniz in the ESP32-S3's ROM:
//! tinfl_decompress sits at a fixed address (esp_rom esp32s3.rom.ld), so it
//! costs no flash. core's gzip.rs does the rest (the header, the window).
//!
//! Its state and its 32KB window go in PSRAM: an update (bundle.rs) is the
//! only user, and internal RAM is short with WiFi on.

use core::ffi::c_void;
use open_oswst_core::gzip::{InflateStep, Status, Step, WINDOW};

use crate::psram::PsramBuffer;

/// tinfl_decompressor's size in the ROM's build of miniz (esp_rom
/// include/miniz.h, 32-bit bit buffer): 11 u32 + 3 table sizes + bit
/// buffer + a size_t = 64, 3 Huffman tables of 288 + 2x1024 + 2x576 = 3488
/// each, 4 + 457 header and code lengths: 10989. Rounded up
const DECOMPRESSOR_BYTES: usize = 11 * 1024;

// tinfl_decompress's flags and statuses (miniz.h)
const TINFL_FLAG_HAS_MORE_INPUT: u32 = 2;
const TINFL_STATUS_DONE: i32 = 0;
const TINFL_STATUS_NEEDS_MORE_INPUT: i32 = 1;
const TINFL_STATUS_HAS_MORE_OUTPUT: i32 = 2;

extern "C" {
    /// In ROM. Inflates from `in_next` (`*in_size` bytes; set to how many it
    /// used) into the window starting at `out_start`, at `out_next`
    /// (`*out_size` bytes of room; set to how many it wrote)
    fn tinfl_decompress(
        decompressor: *mut c_void,
        in_next: *const u8,
        in_size: *mut usize,
        out_start: *mut u8,
        out_next: *mut u8,
        out_size: *mut usize,
        flags: u32,
    ) -> i32;
}

pub struct RomInflate {
    /// tinfl_decompressor: zeroed is its starting state (m_state = 0)
    state: PsramBuffer,
}

impl RomInflate {
    /// The inflater, and the window it needs (gzip::GzipReader takes both)
    pub fn new() -> Option<(RomInflate, PsramBuffer)> {
        let state = PsramBuffer::zeroed(DECOMPRESSOR_BYTES)?;
        let window = PsramBuffer::zeroed(WINDOW)?;
        Some((RomInflate { state }, window))
    }
}

impl InflateStep for RomInflate {
    fn step(&mut self, input: &[u8], window: &mut [u8], position: usize, more_input: bool) -> Step {
        let mut read = input.len();
        let mut written = window.len() - position;
        let flags = if more_input {
            TINFL_FLAG_HAS_MORE_INPUT
        } else {
            0
        };
        // SAFETY: the state is the decompressor's own memory; the pointers
        // and sizes are the input's and the window's, which outlive the call
        let status = unsafe {
            tinfl_decompress(
                self.state.as_mut_ptr().cast(),
                input.as_ptr(),
                &mut read,
                window.as_mut_ptr(),
                window.as_mut_ptr().add(position),
                &mut written,
                flags,
            )
        };
        let status = match status {
            TINFL_STATUS_DONE => Status::Done,
            TINFL_STATUS_NEEDS_MORE_INPUT => Status::NeedsInput,
            TINFL_STATUS_HAS_MORE_OUTPUT => Status::HasMoreOutput,
            _ => Status::Failed,
        };
        Step {
            status,
            read,
            written,
        }
    }
}
