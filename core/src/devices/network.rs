//! The board's WiFi, as the screen shows it. The firmware's network code
//! reports it (Platform::network); the app only displays it.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Network {
    /// No networks configured: WiFi never starts
    Off,
    /// Configured, but not on a network (yet, or any more)
    Searching,
    /// On this network
    Joined(heapless::String<32>),
}
