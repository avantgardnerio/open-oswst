//! Typing a WiFi password with the knob alone. The knob turns a wheel of
//! every printable character, both ways round; a click types the one it's
//! on, and the wheel stays there (a repeated or nearby letter is quick).
//! Two stops on the wheel aren't characters: OK (save) and DEL (backspace;
//! on nothing typed, give up). They sit just before `a`, where the wheel
//! starts, so they're a turn or two back from anywhere in the lowercase.

/// The characters, in wheel order after OK and DEL: lowercase, uppercase,
/// digits, then space and the symbols in ASCII order. 95, every printable
/// ASCII character
const CHARACTERS: &str = concat!(
    "abcdefghijklmnopqrstuvwxyz",
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
    "0123456789",
    " !\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~",
);

/// WPA's longest passphrase
pub const MAX_LEN: usize = 63;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    Ok,
    Del,
    Character(char),
}

/// Stops on the wheel: OK, DEL, then the characters
fn stops() -> usize {
    2 + CHARACTERS.len()
}

fn stop(at: usize) -> Stop {
    match at {
        0 => Stop::Ok,
        1 => Stop::Del,
        _ => Stop::Character(CHARACTERS.as_bytes()[at - 2] as char),
    }
}

fn label(at: usize) -> String {
    match stop(at) {
        Stop::Ok => "OK".into(),
        Stop::Del => "DEL".into(),
        Stop::Character(character) => character.into(),
    }
}

/// What a click did
#[derive(Debug, PartialEq, Eq)]
pub enum Click {
    Typing,
    Done(String),
    Cancel,
}

pub struct PasswordEntry {
    typed: String,
    at: usize, // the wheel's stop
}

impl Default for PasswordEntry {
    fn default() -> Self {
        Self::new()
    }
}

impl PasswordEntry {
    pub fn new() -> Self {
        PasswordEntry {
            typed: String::new(),
            at: 2, // `a`
        }
    }

    /// Turn the wheel; it wraps round both ways
    pub fn rotate(&mut self, delta: isize) {
        self.at = (self.at as isize + delta).rem_euclid(stops() as isize) as usize;
    }

    pub fn click(&mut self) -> Click {
        match stop(self.at) {
            Stop::Ok => Click::Done(self.typed.clone()),
            Stop::Del if self.typed.is_empty() => Click::Cancel,
            Stop::Del => {
                self.typed.pop();
                Click::Typing
            }
            Stop::Character(character) => {
                if self.typed.len() < MAX_LEN {
                    self.typed.push(character);
                }
                Click::Typing
            }
        }
    }

    /// What's typed, then the cursor (`_`), `width` characters at most: the
    /// end of a long password, where the typing is
    pub fn typed_line(&self, width: usize) -> String {
        let shown = self.typed.len().min(width - 1);
        format!("{}_", &self.typed[self.typed.len() - shown..])
    }

    /// The wheel, `width` characters wide: the stop it's on in the middle
    /// and as many of its neighbours either side as fit. Returns the line,
    /// and where in it the stop it's on starts and how long it is (to draw
    /// it inverted). Single characters sit side by side; OK and DEL get a
    /// space either side
    pub fn wheel_line(&self, width: usize) -> (String, usize, usize) {
        let gap = |left: usize, right: usize| {
            usize::from(label(left).len() > 1 || label(right).len() > 1)
        };
        let n = stops();
        let current = label(self.at);
        let start = (width - current.len()) / 2;

        let mut left = String::new();
        let (mut room, mut right_of) = (start, self.at);
        loop {
            let at = (right_of + n - 1) % n;
            let needs = gap(at, right_of) + label(at).len();
            if needs > room {
                break;
            }
            room -= needs;
            left.insert_str(0, &" ".repeat(gap(at, right_of)));
            left.insert_str(0, &label(at));
            right_of = at;
        }

        let mut right = String::new();
        let (mut room, mut left_of) = (width - start - current.len(), self.at);
        loop {
            let at = (left_of + 1) % n;
            let needs = gap(left_of, at) + label(at).len();
            if needs > room {
                break;
            }
            room -= needs;
            right.push_str(&" ".repeat(gap(left_of, at)));
            right.push_str(&label(at));
            left_of = at;
        }

        let line = format!(
            "{}{}{}{}",
            " ".repeat(start - left.len()),
            left,
            current,
            right
        );
        (line, start, current.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_fix_and_save() {
        let mut entry = PasswordEntry::new();
        entry.rotate(1); // b
        entry.click();
        entry.click(); // bb
        entry.rotate(-1); // a
        entry.click(); // bba
        entry.rotate(-1); // DEL
        entry.click(); // bb
        assert_eq!(entry.typed_line(21), "bb_");
        entry.rotate(-1); // OK
        assert_eq!(entry.click(), Click::Done("bb".into()));
    }

    #[test]
    fn wheel_wraps_both_ways() {
        let mut entry = PasswordEntry::new();
        entry.rotate(-3); // a, DEL, OK, then round to the last symbol
        entry.click();
        assert_eq!(entry.typed_line(21), "~_");
        entry.rotate(stops() as isize); // all the way round: still `~`
        entry.click();
        assert_eq!(entry.typed_line(21), "~~_");
    }

    #[test]
    fn del_on_nothing_typed_cancels() {
        let mut entry = PasswordEntry::new();
        entry.rotate(-1);
        assert_eq!(entry.click(), Click::Cancel);
    }

    #[test]
    fn a_long_password_shows_its_end_and_stops_at_the_limit() {
        let mut entry = PasswordEntry::new();
        for _ in 0..MAX_LEN + 5 {
            entry.click();
        }
        assert_eq!(entry.typed_line(21), format!("{}_", "a".repeat(20)));
        entry.rotate(-2);
        assert_eq!(entry.click(), Click::Done("a".repeat(MAX_LEN)));
    }

    #[test]
    fn wheel_line_centres_the_stop() {
        let mut entry = PasswordEntry::new();
        let (line, start, len) = entry.wheel_line(21);
        assert_eq!(line, "}~ OK DEL abcdefghijk");
        assert_eq!((start, len), (10, 1));
        assert_eq!(&line[start..start + len], "a");
        entry.rotate(-1);
        let (line, start, len) = entry.wheel_line(21);
        assert_eq!(&line[start..start + len], "DEL");
        assert_eq!(line.len(), 21);
    }
}
