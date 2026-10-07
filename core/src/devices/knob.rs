//! A rotary encoder with push switch: turns, presses and releases, nothing
//! more.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Cw,
    Ccw,
    Press,
    Release,
}

#[allow(async_fn_in_trait)] // used by one task at a time; Send never needed
pub trait Knob {
    /// Wait for the next turn, press or release. Must be cancel-safe: it sits
    /// in a `select` that drops it whenever something else happens first.
    async fn next(&mut self) -> Event;
}
