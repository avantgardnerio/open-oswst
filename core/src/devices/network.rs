//! The board's WiFi, as the screen shows it. The firmware's network code
//! reports it (Platform::network); the app only displays it.

use core::fmt::Write as _;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Network {
    /// Not running: switched off, or no networks saved
    Off,
    /// Configured, but not on a network (yet, or any more)
    Searching,
    /// On this network, at this address
    Joined {
        ssid: heapless::String<32>,
        ip: [u8; 4],
    },
}

impl Network {
    /// For the screen (screen_text row 5): the address, which also says
    /// we're on a network, or why there's none. The network's name is in
    /// the menu (WiFi networks > Connected). Widest: "192.168.100.240", 15
    pub fn screen_text(&self) -> heapless::String<16> {
        let mut text = heapless::String::new();
        let _ = match self {
            Network::Off => write!(text, "WiFi off"),
            Network::Searching => write!(text, "WiFi searching"),
            Network::Joined { ip, .. } => write!(text, "{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]),
        };
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_screen_shows_the_address_or_why_none() {
        let joined = Network::Joined {
            ssid: "koldendeco".try_into().unwrap(),
            ip: [192, 168, 100, 240],
        };
        assert_eq!(joined.screen_text(), "192.168.100.240");
        assert_eq!(Network::Off.screen_text(), "WiFi off");
        assert_eq!(Network::Searching.screen_text(), "WiFi searching");
    }
}
