//! Key encoding for the kitty keyboard protocol's progressive enhancement
//! (the Swift `KittyKeyboard`;
//! <https://sw.kovidgoyal.net/kitty/keyboard-protocol/>):
//!
//! ```text
//! CSI key[:shifted] ; modifiers[:event] ; text u
//! CSI 1 ; modifiers[:event] {A B C D H F P Q S}
//! CSI number ; modifiers[:event] ~
//! ```
//!
//! A terminal uses it once a program pushes flags (CSI > flags u, mode bits
//! 11 to 15). `encode` returns `None` when the legacy encoding applies
//! (plain typing with only "disambiguate" on, unmodified arrows, ...), so
//! the caller keeps its usual path for those, IME and dead keys included.

use std::ops::BitOr;

/// The enhancements a program asked for, in protocol order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags(pub u8);

impl Flags {
    pub const DISAMBIGUATE: Flags = Flags(1);
    pub const REPORT_EVENT_TYPES: Flags = Flags(2);
    pub const REPORT_ALTERNATE_KEYS: Flags = Flags(4);
    pub const REPORT_ALL_KEYS: Flags = Flags(8);
    pub const REPORT_TEXT: Flags = Flags(16);

    pub fn contains(self, other: Flags) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for Flags {
    type Output = Flags;
    fn bitor(self, other: Flags) -> Flags {
        Flags(self.0 | other.0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers(pub u8);

impl Modifiers {
    pub const NONE: Modifiers = Modifiers(0);
    pub const SHIFT: Modifiers = Modifiers(1);
    pub const ALT: Modifiers = Modifiers(2);
    pub const CONTROL: Modifiers = Modifiers(4);
    pub const SUPER: Modifiers = Modifiers(8);

    pub fn contains(self, other: Modifiers) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for Modifiers {
    type Output = Modifiers;
    fn bitor(self, other: Modifiers) -> Modifiers {
        Modifiers(self.0 | other.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Press = 1,
    Repeat = 2,
    Release = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A key that types `base` without modifiers (lowercase); `shifted` is
    /// what it types with Shift.
    Text {
        base: char,
        shifted: Option<char>,
    },
    Escape,
    Enter,
    Tab,
    Backspace,
    Insert,
    Delete,
    PageUp,
    PageDown,
    Up,
    Down,
    Right,
    Left,
    Home,
    End,
    /// F1 to F12.
    Function(u8),
}

/// The bytes for `key`, `None` when the legacy encoding applies, or empty
/// when nothing is sent (a release the program didn't ask for).
pub fn encode(
    key: Key,
    modifiers: Modifiers,
    event: Event,
    text: Option<&str>,
    flags: Flags,
) -> Option<Vec<u8>> {
    let all_keys = flags.contains(Flags::REPORT_ALL_KEYS);
    let report_events = flags.contains(Flags::REPORT_EVENT_TYPES);
    let event = if report_events || event == Event::Release {
        event
    } else {
        Event::Press
    };
    if event == Event::Release && !report_events {
        return Some(Vec::new());
    }
    let modifier_value = 1 + modifiers.0 as u32;
    let event_suffix = if event == Event::Press {
        String::new()
    } else {
        format!(":{}", event as u8)
    };
    let plain = modifier_value == 1 && event_suffix.is_empty();
    let csi = |body: String| -> Vec<u8> {
        let mut bytes = b"\x1b[".to_vec();
        bytes.extend_from_slice(body.as_bytes());
        bytes
    };
    let with_modifiers = |prefix: &str, last: &str| -> Vec<u8> {
        if plain {
            csi(format!("{prefix}{last}"))
        } else {
            csi(format!("{prefix};{modifier_value}{event_suffix}{last}"))
        }
    };

    match key {
        Key::Text { base, shifted } => {
            let command_modifiers =
                modifiers.0 & (Modifiers::ALT.0 | Modifiers::CONTROL.0 | Modifiers::SUPER.0) != 0;
            if !all_keys && !command_modifiers {
                // Typing stays text; its releases aren't reported.
                return if event == Event::Release {
                    Some(Vec::new())
                } else {
                    None
                };
            }
            let mut code = (base as u32).to_string();
            if flags.contains(Flags::REPORT_ALTERNATE_KEYS) && modifiers.contains(Modifiers::SHIFT)
            {
                if let Some(shifted) = shifted.filter(|&s| s != base) {
                    code.push_str(&format!(":{}", shifted as u32));
                }
            }
            let text_field = text
                .filter(|t| {
                    all_keys
                        && flags.contains(Flags::REPORT_TEXT)
                        && event != Event::Release
                        && !t.is_empty()
                        && !t.chars().any(|c| (c as u32) < 0x20 || c as u32 == 0x7F)
                })
                .map(|t| {
                    t.chars()
                        .map(|c| (c as u32).to_string())
                        .collect::<Vec<_>>()
                        .join(":")
                });
            match text_field {
                Some(field) => Some(csi(format!(
                    "{code};{modifier_value}{event_suffix};{field}u"
                ))),
                None => Some(with_modifiers(&code, "u")),
            }
        }

        Key::Escape | Key::Enter | Key::Tab | Key::Backspace => {
            let code = match key {
                Key::Escape => 27,
                Key::Enter => 13,
                Key::Tab => 9,
                _ => 127,
            };
            // Enter, Tab and Backspace stay legacy unless modified (so
            // `reset` can still be typed after a program dies in this mode).
            if key != Key::Escape && !all_keys {
                if plain {
                    return None;
                }
                if event == Event::Release {
                    return Some(Vec::new());
                }
            }
            Some(with_modifiers(&code.to_string(), "u"))
        }

        Key::Up | Key::Down | Key::Right | Key::Left | Key::Home | Key::End => {
            if plain {
                return None;
            }
            let letter = match key {
                Key::Up => "A",
                Key::Down => "B",
                Key::Right => "C",
                Key::Left => "D",
                Key::Home => "H",
                _ => "F",
            };
            Some(csi(format!("1;{modifier_value}{event_suffix}{letter}")))
        }

        Key::Insert | Key::Delete | Key::PageUp | Key::PageDown => {
            if plain {
                return None;
            }
            let number = match key {
                Key::Insert => 2,
                Key::Delete => 3,
                Key::PageUp => 5,
                _ => 6,
            };
            Some(csi(format!("{number};{modifier_value}{event_suffix}~")))
        }

        Key::Function(n) => {
            if plain {
                return None;
            }
            let letter = match n {
                1 => Some("P"),
                2 => Some("Q"),
                4 => Some("S"),
                _ => None,
            };
            if let Some(letter) = letter {
                return Some(csi(format!("1;{modifier_value}{event_suffix}{letter}")));
            }
            // F3 avoids CSI R, which reads as a cursor position report.
            let number = match n {
                3 => 13,
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                12 => 24,
                _ => return None,
            };
            Some(csi(format!("{number};{modifier_value}{event_suffix}~")))
        }
    }
}

/// A key as GPUI names it ("escape", "f5", "a", "space") with the text it
/// types (`key_char`), as the protocol's key and the text to report (the
/// Swift `KeyEncoder.kittyKey`). `None` for keys the protocol doesn't
/// describe and for Alt that isn't Meta and types a character on its own
/// (å, or @ on a German layout): those keep the legacy path, which sends
/// the text.
pub fn key_for<'a>(
    name: &str,
    text: Option<&'a str>,
    modifiers: Modifiers,
    alt_is_meta: bool,
) -> Option<(Key, Option<&'a str>)> {
    let functional = match name {
        "escape" => Some(Key::Escape),
        "enter" => Some(Key::Enter),
        "tab" => Some(Key::Tab),
        "backspace" => Some(Key::Backspace),
        "insert" => Some(Key::Insert),
        "delete" => Some(Key::Delete),
        "pageup" => Some(Key::PageUp),
        "pagedown" => Some(Key::PageDown),
        "up" => Some(Key::Up),
        "down" => Some(Key::Down),
        "right" => Some(Key::Right),
        "left" => Some(Key::Left),
        "home" => Some(Key::Home),
        "end" => Some(Key::End),
        f if f.len() > 1 && f.starts_with('f') => f[1..]
            .parse::<u8>()
            .ok()
            .filter(|n| (1..=12).contains(n))
            .map(Key::Function),
        _ => None,
    };
    if let Some(key) = functional {
        return Some((key, None));
    }
    let alt = modifiers.contains(Modifiers::ALT);
    let command = modifiers.0 & (Modifiers::CONTROL.0 | Modifiers::SUPER.0) != 0;
    if alt && !alt_is_meta && !command {
        return None;
    }
    let name = if name == "space" { " " } else { name };
    let mut chars = name.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    if (c as u32) < 0x20 || c as u32 == 0x7F {
        return None;
    }
    let base = c.to_lowercase().next().unwrap_or(c);
    let shifted = if modifiers.contains(Modifiers::SHIFT) {
        text.and_then(|t| {
            let mut chars = t.chars();
            match (chars.next(), chars.next()) {
                (Some(s), None) => Some(s),
                _ => None,
            }
        })
    } else {
        None
    };
    // What Alt makes of the key isn't text it types.
    Some((Key::Text { base, shifted }, if alt { None } else { text }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpui_keys_map_to_protocol_keys() {
        let none = Modifiers::NONE;
        assert_eq!(
            key_for("escape", None, none, true),
            Some((Key::Escape, None))
        );
        assert_eq!(
            key_for("f12", None, none, true),
            Some((Key::Function(12), None))
        );
        assert_eq!(key_for("f13", None, none, true), None);
        assert_eq!(
            key_for("space", Some(" "), none, true),
            Some((
                Key::Text {
                    base: ' ',
                    shifted: None
                },
                Some(" ")
            ))
        );
        assert_eq!(
            key_for("a", Some("A"), Modifiers::SHIFT, true),
            Some((
                Key::Text {
                    base: 'a',
                    shifted: Some('A')
                },
                Some("A")
            ))
        );
        // Alt that isn't Meta types its character the legacy way, unless
        // Control or Super is held too.
        assert_eq!(key_for("e", Some("é"), Modifiers::ALT, false), None);
        assert_eq!(
            key_for("f", None, Modifiers::ALT | Modifiers::CONTROL, false),
            Some((
                Key::Text {
                    base: 'f',
                    shifted: None
                },
                None
            ))
        );
        assert_eq!(
            key_for("f", Some("ƒ"), Modifiers::ALT, true),
            Some((
                Key::Text {
                    base: 'f',
                    shifted: None
                },
                None
            ))
        );
    }

    const A: Key = Key::Text {
        base: 'a',
        shifted: Some('A'),
    };
    const NONE: Modifiers = Modifiers::NONE;
    const SHIFT: Modifiers = Modifiers::SHIFT;
    const CONTROL: Modifiers = Modifiers::CONTROL;
    const ALT: Modifiers = Modifiers::ALT;

    fn enc(
        key: Key,
        modifiers: Modifiers,
        event: Event,
        text: Option<&str>,
        flags: Flags,
    ) -> Option<String> {
        encode(key, modifiers, event, text, flags)
            .map(|bytes| String::from_utf8(bytes).unwrap().replace('\x1b', "^["))
    }

    /// With only "disambiguate", as a press.
    fn press(key: Key, modifiers: Modifiers) -> Option<String> {
        enc(key, modifiers, Event::Press, None, Flags::DISAMBIGUATE)
    }

    #[test]
    fn disambiguate_keeps_typing_as_text() {
        assert_eq!(press(A, NONE), None);
        assert_eq!(press(A, SHIFT), None, "shift alone still types");
        assert_eq!(press(Key::Enter, NONE), None);
        assert_eq!(press(Key::Tab, NONE), None);
        assert_eq!(press(Key::Backspace, NONE), None);
        assert_eq!(press(Key::Up, NONE), None);
        assert_eq!(press(Key::Function(5), NONE), None);
    }

    #[test]
    fn disambiguated_keys() {
        let some = |s: &str| Some(s.to_string());
        assert_eq!(press(Key::Escape, NONE), some("^[[27u"));
        assert_eq!(press(A, CONTROL), some("^[[97;5u"));
        assert_eq!(press(A, CONTROL | SHIFT), some("^[[97;6u"));
        let i = Key::Text {
            base: 'i',
            shifted: Some('I'),
        };
        assert_eq!(press(i, CONTROL), some("^[[105;5u"), "not the same as Tab");
        assert_eq!(press(A, ALT), some("^[[97;3u"));
        assert_eq!(press(Key::Enter, CONTROL), some("^[[13;5u"));
        assert_eq!(press(Key::Tab, SHIFT), some("^[[9;2u"));
        assert_eq!(press(Key::Backspace, ALT), some("^[[127;3u"));
    }

    #[test]
    fn functional_keys_with_modifiers() {
        let some = |s: &str| Some(s.to_string());
        assert_eq!(press(Key::Up, CONTROL), some("^[[1;5A"));
        assert_eq!(press(Key::End, SHIFT), some("^[[1;2F"));
        assert_eq!(press(Key::Delete, ALT), some("^[[3;3~"));
        assert_eq!(press(Key::PageDown, CONTROL), some("^[[6;5~"));
        assert_eq!(press(Key::Function(1), SHIFT), some("^[[1;2P"));
        assert_eq!(
            press(Key::Function(3), SHIFT),
            some("^[[13;2~"),
            "F3 avoids CSI R"
        );
        assert_eq!(press(Key::Function(12), CONTROL), some("^[[24;5~"));
    }

    #[test]
    fn event_types() {
        let some = |s: &str| Some(s.to_string());
        let flags = Flags::DISAMBIGUATE | Flags::REPORT_EVENT_TYPES;
        assert_eq!(
            enc(A, CONTROL, Event::Repeat, None, flags),
            some("^[[97;5:2u")
        );
        assert_eq!(
            enc(A, CONTROL, Event::Release, None, flags),
            some("^[[97;5:3u")
        );
        assert_eq!(
            enc(Key::Up, NONE, Event::Release, None, flags),
            some("^[[1;1:3A")
        );
        assert_eq!(
            enc(A, NONE, Event::Release, None, flags),
            some(""),
            "typing isn't reported on release"
        );
        assert_eq!(
            enc(Key::Enter, NONE, Event::Release, None, flags),
            some(""),
            "nor are Enter, Tab and Backspace"
        );
        assert_eq!(
            enc(A, CONTROL, Event::Release, None, Flags::DISAMBIGUATE),
            some(""),
            "no releases without the flag"
        );
        assert_eq!(
            enc(A, CONTROL, Event::Repeat, None, Flags::DISAMBIGUATE),
            some("^[[97;5u"),
            "repeats are presses without the flag"
        );
    }

    #[test]
    fn all_keys_alternates_and_text() {
        let some = |s: &str| Some(s.to_string());
        let all = Flags::DISAMBIGUATE | Flags::REPORT_ALL_KEYS;
        assert_eq!(enc(A, NONE, Event::Press, None, all), some("^[[97u"));
        assert_eq!(
            enc(Key::Enter, NONE, Event::Press, None, all),
            some("^[[13u")
        );
        assert_eq!(
            enc(
                A,
                SHIFT,
                Event::Press,
                None,
                all | Flags::REPORT_ALTERNATE_KEYS
            ),
            some("^[[97:65;2u")
        );
        assert_eq!(
            enc(A, SHIFT, Event::Press, None, all),
            some("^[[97;2u"),
            "alternates only when asked"
        );
        assert_eq!(
            enc(A, SHIFT, Event::Press, Some("A"), all | Flags::REPORT_TEXT),
            some("^[[97;2;65u")
        );
        assert_eq!(
            enc(
                A,
                NONE,
                Event::Press,
                Some("a"),
                Flags::REPORT_ALL_KEYS | Flags::REPORT_TEXT
            ),
            some("^[[97;1;97u")
        );
    }
}
