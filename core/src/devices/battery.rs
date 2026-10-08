//! The battery: its voltage, read now and then. Knows nothing else.

#[allow(async_fn_in_trait)] // used by one task at a time; Send never needed
pub trait Battery {
    /// The battery's voltage in mV; None if it can't be read right now
    async fn millivolts(&mut self) -> Option<u32>;
}
