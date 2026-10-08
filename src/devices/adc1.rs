//! ADC1 is shared: the mic samples it continuously while PTT is held
//! (mic.rs), and the battery takes a one-shot reading the rest of the time
//! (battery.rs). The chip lets one driver own it at a time, so each takes
//! this lease first and steals the peripheral for its driver while it holds
//! it. Both run on the app's task, one after the other, so a lease is never
//! refused in practice; if it ever is, the battery skips its reading.

use core::sync::atomic::{AtomicBool, Ordering};

static IN_USE: AtomicBool = AtomicBool::new(false);

/// ADC1 is ours while this lives
pub struct Lease(());

/// None if ADC1 is in use
pub fn lease() -> Option<Lease> {
    IN_USE
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .ok()
        .map(|_| Lease(()))
}

impl Drop for Lease {
    fn drop(&mut self) {
        IN_USE.store(false, Ordering::Release);
    }
}
