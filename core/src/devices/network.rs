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
    /// One screen line, `width` characters at most. Joined: the network's
    /// name on the left (cut short if need be: users know their own
    /// network) and the end of the address on the right, for reaching the
    /// radio: the last two bytes if they fit beside the whole name
    /// (".0.240"), else the last one (".240")
    pub fn screen_line(&self, width: usize) -> heapless::String<32> {
        let mut line = heapless::String::new();
        match self {
            Network::Off => {
                let _ = line.push_str("WiFi: off");
            }
            Network::Searching => {
                let _ = line.push_str("WiFi: searching");
            }
            Network::Joined { ssid, ip } => {
                let mut end = heapless::String::<8>::new();
                let _ = write!(end, ".{}.{}", ip[2], ip[3]);
                if ssid.len() + 1 + end.len() > width {
                    end.clear();
                    let _ = write!(end, ".{}", ip[3]);
                }
                let room = width.saturating_sub(end.len() + 1);
                let name: heapless::String<32> = ssid.chars().take(room).collect();
                let gap = width - end.len() - name.chars().count();
                let _ = line.push_str(&name);
                for _ in 0..gap {
                    let _ = line.push(' ');
                }
                let _ = line.push_str(&end);
            }
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joined(ssid: &str, ip: [u8; 4]) -> Network {
        Network::Joined {
            ssid: ssid.try_into().unwrap(),
            ip,
        }
    }

    #[test]
    fn name_left_two_address_bytes_right_when_they_fit() {
        let line = joined("koldendeco", [192, 168, 0, 240]).screen_line(21);
        assert_eq!(line, "koldendeco     .0.240");
        assert_eq!(line.len(), 21);
    }

    #[test]
    fn a_long_name_gets_the_last_byte_and_is_cut_short() {
        let line = joined("a-very-long-network-name", [10, 1, 20, 7]).screen_line(21);
        assert_eq!(line, "a-very-long-networ .7");
        assert_eq!(line.len(), 21);
        let line = joined("eighteen-char-name", [10, 1, 20, 132]).screen_line(21);
        assert_eq!(line, "eighteen-char-na .132");
    }

    #[test]
    fn off_and_searching_say_so() {
        assert_eq!(Network::Off.screen_line(21), "WiFi: off");
        assert_eq!(Network::Searching.screen_line(21), "WiFi: searching");
    }
}
