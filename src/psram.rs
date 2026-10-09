//! Memory from the PSRAM (sdkconfig.defaults CONFIG_SPIRAM_USE_CAPS_ALLOC:
//! nothing lands there unless asked for here). Slower than internal RAM and
//! unreachable while the flash is written, so only for big buffers that
//! nothing time-critical touches.

use core::ops::{Deref, DerefMut};
use esp_idf_svc::sys::{heap_caps_calloc, heap_caps_free, MALLOC_CAP_8BIT, MALLOC_CAP_SPIRAM};

/// Zeroed bytes in PSRAM, freed when dropped
pub struct PsramBuffer {
    memory: *mut u8,
    len: usize,
}

// Plain memory, owned by whoever holds the buffer
unsafe impl Send for PsramBuffer {}

impl PsramBuffer {
    /// None if the PSRAM can't spare `len` bytes (or there is none)
    pub fn zeroed(len: usize) -> Option<PsramBuffer> {
        let memory = unsafe { heap_caps_calloc(1, len, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT) };
        (!memory.is_null()).then_some(PsramBuffer {
            memory: memory.cast(),
            len,
        })
    }
}

impl Drop for PsramBuffer {
    fn drop(&mut self) {
        unsafe { heap_caps_free(self.memory.cast()) };
    }
}

impl Deref for PsramBuffer {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        // SAFETY: `len` bytes allocated for us alone, until drop
        unsafe { core::slice::from_raw_parts(self.memory, self.len) }
    }
}

impl DerefMut for PsramBuffer {
    fn deref_mut(&mut self) -> &mut [u8] {
        // SAFETY: as deref, and &mut self makes it the only reference
        unsafe { core::slice::from_raw_parts_mut(self.memory, self.len) }
    }
}
