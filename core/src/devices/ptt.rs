//! The push-to-talk button.

#[allow(async_fn_in_trait)] // used by one task at a time; Send never needed
pub trait Ptt {
    /// Wait until the button is down (returns at once if it already is).
    async fn pressed(&mut self);

    /// Wait until the button is up (returns at once if it already is).
    async fn released(&mut self);

    fn is_pressed(&self) -> bool;
}
