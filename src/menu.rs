//! Nested menu: a static tree of lists plus a stack of where you are in it.
//! Every list gets an implicit "Back" at the top, and the cursor starts there,
//! so stray clicks only ever back out — they never change a setting.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    Lock,
    Repeater,
}

pub enum Item {
    Submenu(&'static str, &'static [Item]),
    Choice(&'static str, Setting, bool),
}

static ROOT: &[Item] = &[
    Item::Submenu(
        "Lock",
        &[
            Item::Choice("true", Setting::Lock, true),
            Item::Choice("false", Setting::Lock, false),
        ],
    ),
    Item::Submenu(
        "Repeater",
        &[
            Item::Choice("true", Setting::Repeater, true),
            Item::Choice("false", Setting::Repeater, false),
        ],
    ),
];

pub const BACK: &str = "Back";

/// What a click did.
pub enum Outcome {
    Stay,
    Exit,
    Set(Setting, bool),
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
    pub fn rows(&self, current: impl Fn(Setting, bool) -> bool) -> Vec<(&'static str, bool)> {
        let mut rows = vec![(BACK, false)];
        for item in self.level().items {
            rows.push(match item {
                Item::Submenu(label, _) => (*label, false),
                Item::Choice(label, setting, value) => (*label, current(*setting, *value)),
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
