//! Nested menu: a static tree of lists plus a stack of where you are in it.
//! Every list gets an implicit "Back" at the top, and the cursor starts there,
//! so stray clicks only ever back out — they never change a setting.

use crate::mode::Mode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    Lock,         // 0 = off, 1 = on
    Mode,         // a mode::Mode as u8
    Wifi,         // config::WIFI_ON, 0 or 1
    HttpApi,      // config::HTTP_API_ON, 0 or 1
    Gps,          // config::GPS_ON, 0 or 1
    SendPosition, // config::SEND_POSITION, 0 or 1
    LogToFlash,   // config::LOG_TO_FLASH, 0 or 1
}

/// Something done once, rather than a setting to choose a value for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Screenshot, // save the screen as it was when the menu opened
}

/// A screen of its own, opened from the menu; the menu is where it was
/// when the page is left
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Connected,  // the network we're on, and our address
    AddWifi,    // networks in range, then the password
    ForgetWifi, // the saved networks
    EraseLogs,  // asks first, then deletes every log file
    Update,     // ask the management server
}

pub enum Item {
    Submenu(&'static str, &'static [Item]),
    Choice(&'static str, Setting, u8),
    Action(&'static str, Action), // closes the whole menu, then it's done
    Page(&'static str, Page),
}

static ROOT: &[Item] = &[
    Item::Action("Screenshot", Action::Screenshot),
    Item::Submenu(
        "Lock",
        &[
            Item::Choice("true", Setting::Lock, 1),
            Item::Choice("false", Setting::Lock, 0),
        ],
    ),
    Item::Submenu(
        "Mode",
        &[
            Item::Choice("Normal", Setting::Mode, Mode::Normal as u8),
            Item::Choice("Repeater", Setting::Mode, Mode::Repeater as u8),
            Item::Choice("Echo", Setting::Mode, Mode::Echo as u8),
        ],
    ),
    // What the radio gives away: RF that can be direction-found, where we
    // are, and what a captured radio would hold. Each choice is saved
    Item::Submenu(
        "Privacy",
        &[
            Item::Submenu(
                "WiFi",
                &[
                    Item::Choice("on", Setting::Wifi, 1),
                    Item::Choice("off", Setting::Wifi, 0),
                ],
            ),
            Item::Submenu(
                "HTTP API",
                &[
                    Item::Choice("on", Setting::HttpApi, 1),
                    Item::Choice("off", Setting::HttpApi, 0),
                ],
            ),
            Item::Submenu(
                "GPS",
                &[
                    Item::Choice("on", Setting::Gps, 1),
                    Item::Choice("off", Setting::Gps, 0),
                ],
            ),
            Item::Submenu(
                "Send name+position",
                &[
                    Item::Choice("on", Setting::SendPosition, 1),
                    Item::Choice("off", Setting::SendPosition, 0),
                ],
            ),
            Item::Submenu(
                "Log to flash",
                &[
                    Item::Choice("on", Setting::LogToFlash, 1),
                    Item::Choice("off", Setting::LogToFlash, 0),
                ],
            ),
            Item::Page("Erase logs", Page::EraseLogs),
        ],
    ),
    Item::Submenu(
        "WiFi networks",
        &[
            Item::Page("Connected", Page::Connected),
            Item::Page("Add", Page::AddWifi),
            Item::Page("Forget", Page::ForgetWifi),
        ],
    ),
    Item::Page("Update", Page::Update),
];

pub const BACK: &str = "Back";

/// What a click did.
pub enum Outcome {
    Stay,
    Exit,
    Set(Setting, u8),
    ExitAndDo(Action),
    Open(Page),
}

#[derive(Clone, Copy)]
struct Level {
    title: &'static str,
    items: &'static [Item],
    cursor: usize, // 0 = Back, 1.. = items
}

pub struct Menu {
    stack: Vec<Level>,
}

impl Default for Menu {
    fn default() -> Self {
        Self::new()
    }
}

impl Menu {
    pub fn new() -> Self {
        Menu {
            stack: vec![Level {
                title: "Menu",
                items: ROOT,
                cursor: 0,
            }],
        }
    }

    /// Move the cursor, stopping at either end.
    pub fn rotate(&mut self, delta: isize) {
        let level = self.level_mut();
        let last = level.items.len();
        level.cursor = level.cursor.saturating_add_signed(delta).min(last);
    }

    pub fn click(&mut self) -> Outcome {
        let Level { items, cursor, .. } = *self.level();
        if cursor == 0 {
            self.stack.pop();
            return if self.stack.is_empty() {
                Outcome::Exit
            } else {
                Outcome::Stay
            };
        }
        match &items[cursor - 1] {
            Item::Submenu(title, items) => {
                self.stack.push(Level {
                    title,
                    items,
                    cursor: 0,
                });
                Outcome::Stay
            }
            Item::Choice(_, setting, value) => {
                // Choosing returns to the parent list (a top-level choice stays put)
                if self.stack.len() > 1 {
                    self.stack.pop();
                }
                Outcome::Set(*setting, *value)
            }
            Item::Action(_, action) => {
                self.stack.clear();
                Outcome::ExitAndDo(*action)
            }
            Item::Page(_, page) => Outcome::Open(*page),
        }
    }

    pub fn title(&self) -> &'static str {
        self.level().title
    }

    pub fn cursor(&self) -> usize {
        self.level().cursor
    }

    /// Rows of the current list, "Back" first. `current` says whether a
    /// choice is the setting's present value (drawn with a *).
    pub fn rows(&self, current: impl Fn(Setting, u8) -> bool) -> Vec<(&'static str, bool)> {
        let mut rows = vec![(BACK, false)];
        for item in self.level().items {
            rows.push(match item {
                Item::Submenu(label, _) => (*label, false),
                Item::Choice(label, setting, value) => (*label, current(*setting, *value)),
                Item::Action(label, _) | Item::Page(label, _) => (*label, false),
            });
        }
        rows
    }

    fn level(&self) -> &Level {
        self.stack.last().unwrap()
    }

    fn level_mut(&mut self) -> &mut Level {
        self.stack.last_mut().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_closes_the_menu_and_asks_for_one() {
        let mut menu = Menu::new();
        assert_eq!(menu.rows(|_, _| false)[1].0, "Screenshot");
        menu.rotate(1);
        assert!(matches!(
            menu.click(),
            Outcome::ExitAndDo(Action::Screenshot)
        ));
    }

    #[test]
    fn privacy_switches_wifi_off() {
        let mut menu = Menu::new();
        menu.rotate(4); // Back, Screenshot, Lock, Mode, Privacy
        assert!(matches!(menu.click(), Outcome::Stay));
        assert_eq!(menu.title(), "Privacy");
        menu.rotate(1); // WiFi
        assert!(matches!(menu.click(), Outcome::Stay));
        menu.rotate(2); // on, off
        assert!(matches!(menu.click(), Outcome::Set(Setting::Wifi, 0)));
        assert_eq!(menu.title(), "Privacy"); // back to the list it came from
    }

    #[test]
    fn a_page_opens_and_the_menu_stays_put() {
        let mut menu = Menu::new();
        menu.rotate(5); // Back, Screenshot, Lock, Mode, Privacy, WiFi networks
        assert!(matches!(menu.click(), Outcome::Stay));
        menu.rotate(2); // Connected, Add
        assert!(matches!(menu.click(), Outcome::Open(Page::AddWifi)));
        assert_eq!(menu.title(), "WiFi networks");
        assert_eq!(menu.cursor(), 2);
    }
}
