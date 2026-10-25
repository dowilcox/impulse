//! The bytes a key sends to a terminal (the Swift `KeyEncoder`), for keys
//! described the way GPUI describes them: a key name ("enter", "up", "f5",
//! "a") and the text it types. Legacy xterm sequences: arrows honor
//! application cursor mode, modifiers go in CSI parameters, Ctrl makes
//! control characters, and Alt sends Esc + the key when it's Meta.

/// Which Option keys are Meta (`terminal_option_as_meta`): `held` says
/// which sides are down (left, right) where the platform can tell (the
/// Mac's device flags), else either side counts. On Linux Alt is always
/// Meta, as in other Linux terminals.
pub fn alt_is_meta_held(setting: &str, mac: bool, held: Option<(bool, bool)>) -> bool {
    if !mac {
        return true;
    }
    match setting.to_lowercase().as_str() {
        "off" | "" => false,
        "left" => held.is_none_or(|(left, _)| left),
        "right" => held.is_none_or(|(_, right)| right),
        _ => true,
    }
}

/// `alt_is_meta_held` without knowing the side.
pub fn alt_is_meta(setting: &str, mac: bool) -> bool {
    alt_is_meta_held(setting, mac, None)
}

/// A key press.
#[derive(Clone, Debug, Default)]
pub struct KeyPress<'a> {
    /// GPUI's key name: "enter", "backspace", "tab", "escape", "up", "down",
    /// "left", "right", "home", "end", "pageup", "pagedown", "delete",
    /// "insert", "f1"…"f20", "space", or the key's character ("a", "/").
    pub key: &'a str,
    /// The text the key types with its modifiers, if any ("A", "å").
    pub text: Option<&'a str>,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Cmd on the Mac, Super on Linux: never the terminal's.
    pub platform: bool,
}

fn modifier_param(press: &KeyPress) -> u8 {
    1 + press.shift as u8 + if press.alt { 2 } else { 0 } + if press.ctrl { 4 } else { 0 }
}

fn csi(tail: &str) -> Vec<u8> {
    let mut out = vec![0x1B, b'['];
    out.extend_from_slice(tail.as_bytes());
    out
}

/// The control character Ctrl makes of a key: Ctrl+A to Ctrl+Z, Ctrl+[
/// (Esc), Ctrl+\ and Ctrl+], plus Ctrl+Space and Ctrl+@ (NUL), Ctrl+^ and
/// Ctrl+_ and Ctrl+/ (as other terminals send them).
pub fn control_byte(key: char) -> Option<u8> {
    let c = key.to_ascii_lowercase();
    match c {
        'a'..='z' => Some(c as u8 - 0x60),
        '[' => Some(0x1B),
        '\\' => Some(0x1C),
        ']' => Some(0x1D),
        ' ' | '@' | '2' => Some(0x00),
        '^' | '6' => Some(0x1E),
        '_' | '-' | '/' | '7' => Some(0x1F),
        _ => None,
    }
}

/// The bytes for `press`, or `None` when the terminal doesn't get the key
/// (Cmd/Super shortcuts) or it only types text (leave that to text input,
/// so input methods and dead keys compose it).
pub fn encode(press: &KeyPress, app_cursor: bool, alt_is_meta: bool) -> Option<Vec<u8>> {
    if press.platform {
        return None;
    }
    let param = modifier_param(press);
    let any = param > 1;
    let meta = |bytes: Vec<u8>| -> Vec<u8> {
        if press.alt && alt_is_meta {
            let mut out = vec![0x1B];
            out.extend(bytes);
            out
        } else {
            bytes
        }
    };
    let arrow = |letter: char| -> Vec<u8> {
        if any {
            csi(&format!("1;{param}{letter}"))
        } else {
            vec![0x1B, if app_cursor { b'O' } else { b'[' }, letter as u8]
        }
    };
    let nav = |letter: char| -> Vec<u8> {
        if any {
            csi(&format!("1;{param}{letter}"))
        } else {
            csi(&letter.to_string())
        }
    };
    let tilde = |number: u8| -> Vec<u8> {
        if any {
            csi(&format!("{number};{param}~"))
        } else {
            csi(&format!("{number}~"))
        }
    };
    let ss3 = |letter: char| -> Vec<u8> {
        if any {
            csi(&format!("1;{param}{letter}"))
        } else {
            vec![0x1B, b'O', letter as u8]
        }
    };
    let bytes = match press.key {
        // Shift+Return is a plain Return (shells read CSI 13;2u as text).
        "enter" => meta(vec![0x0D]),
        "backspace" => {
            if press.ctrl {
                meta(vec![0x08])
            } else {
                meta(vec![0x7F])
            }
        }
        "tab" => {
            if press.shift {
                csi("Z")
            } else {
                meta(vec![0x09])
            }
        }
        "escape" => meta(vec![0x1B]),
        "up" => arrow('A'),
        "down" => arrow('B'),
        "right" => arrow('C'),
        "left" => arrow('D'),
        "home" => nav('H'),
        "end" => nav('F'),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "delete" => tilde(3),
        "insert" => tilde(2),
        "f1" => ss3('P'),
        "f2" => ss3('Q'),
        "f3" => ss3('R'),
        "f4" => ss3('S'),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        key => {
            let key = if key == "space" { " " } else { key };
            let mut chars = key.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return None;
            };
            if press.ctrl {
                return control_byte(c).map(|b| meta(vec![b]));
            }
            if press.alt && alt_is_meta {
                // Esc + the key's own character (Shift kept): Meta-f, Meta-F.
                let c = if press.shift {
                    c.to_ascii_uppercase()
                } else {
                    c
                };
                let mut out = vec![0x1B];
                out.extend(c.to_string().as_bytes());
                return Some(out);
            }
            return None;
        }
    };
    Some(bytes)
}

/// Text with no control characters but tab and line ends (the Swift
/// `withoutControlCharacters`): the line editor would act on them as keys
/// (^U clears what's typed, ESC starts a sequence).
pub fn without_control_characters(text: &str) -> String {
    text.chars()
        .filter(|c| matches!(c, '\t' | '\n' | '\r') || !c.is_control())
        .collect()
}

/// Text for the terminal from text input (typing, an input method, a
/// paste): control characters but tab and line ends are dropped, `\n`
/// becomes `\r` as a typed Return would send, and a paste is wrapped in
/// bracketed-paste marks when the program asked for them. With no ESC left
/// inside, nothing in a poisoned clipboard can end the paste early and run
/// what follows (removing only the end mark isn't enough: "ESC[20" +
/// "ESC[201~" + "1~" rejoins into one).
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let text = without_control_characters(text)
        .replace("\r\n", "\r")
        .replace('\n', "\r");
    if bracketed {
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pastes_carry_no_control_characters() {
        let poisoned = "ls\x1b[20\x1b[201~1~; echo pwned\x15\x03\n";
        let bracketed = String::from_utf8(paste_bytes(poisoned, true)).unwrap();
        assert_eq!(bracketed, "\x1b[200~ls[20[201~1~; echo pwned\r\x1b[201~");
        assert_eq!(bracketed.matches('\x1b').count(), 2);
        let plain = String::from_utf8(paste_bytes("a\tb\x1bc\u{9b}d\r\n", false)).unwrap();
        assert_eq!(plain, "a\tbcd\r");
    }

    fn key(key: &str) -> KeyPress<'_> {
        KeyPress {
            key,
            ..Default::default()
        }
    }

    fn enc(press: KeyPress) -> Option<Vec<u8>> {
        encode(&press, false, true)
    }

    #[test]
    fn specials() {
        assert_eq!(enc(key("enter")), Some(vec![0x0D]));
        assert_eq!(enc(key("backspace")), Some(vec![0x7F]));
        assert_eq!(enc(key("escape")), Some(vec![0x1B]));
        assert_eq!(enc(key("tab")), Some(vec![0x09]));
        assert_eq!(
            enc(KeyPress {
                shift: true,
                ..key("tab")
            }),
            Some(b"\x1b[Z".to_vec())
        );
        assert_eq!(
            enc(KeyPress {
                shift: true,
                ..key("enter")
            }),
            Some(vec![0x0D])
        );
        assert_eq!(enc(key("delete")), Some(b"\x1b[3~".to_vec()));
        assert_eq!(enc(key("f1")), Some(b"\x1bOP".to_vec()));
        assert_eq!(enc(key("f5")), Some(b"\x1b[15~".to_vec()));
        assert_eq!(enc(key("pageup")), Some(b"\x1b[5~".to_vec()));
    }

    #[test]
    fn arrows_and_modes() {
        assert_eq!(enc(key("up")), Some(b"\x1b[A".to_vec()));
        assert_eq!(encode(&key("up"), true, true), Some(b"\x1bOA".to_vec()));
        assert_eq!(
            enc(KeyPress {
                ctrl: true,
                ..key("right")
            }),
            Some(b"\x1b[1;5C".to_vec())
        );
        assert_eq!(
            enc(KeyPress {
                shift: true,
                alt: true,
                ..key("left")
            }),
            Some(b"\x1b[1;4D".to_vec())
        );
        assert_eq!(
            enc(KeyPress {
                shift: true,
                ..key("home")
            }),
            Some(b"\x1b[1;2H".to_vec())
        );
    }

    #[test]
    fn control_and_meta() {
        assert_eq!(
            enc(KeyPress {
                ctrl: true,
                ..key("c")
            }),
            Some(vec![0x03])
        );
        assert_eq!(
            enc(KeyPress {
                ctrl: true,
                ..key("[")
            }),
            Some(vec![0x1B])
        );
        assert_eq!(
            enc(KeyPress {
                ctrl: true,
                ..key("space")
            }),
            Some(vec![0x00])
        );
        assert_eq!(
            enc(KeyPress {
                alt: true,
                ..key("f")
            }),
            Some(b"\x1bf".to_vec())
        );
        assert_eq!(
            enc(KeyPress {
                alt: true,
                shift: true,
                ..key("f")
            }),
            Some(b"\x1bF".to_vec())
        );
        assert_eq!(
            enc(KeyPress {
                alt: true,
                ctrl: true,
                ..key("f")
            }),
            Some(vec![0x1B, 0x06])
        );
        assert_eq!(
            enc(KeyPress {
                alt: true,
                ..key("backspace")
            }),
            Some(vec![0x1B, 0x7F])
        );
        // Option that isn't Meta types its character: text input does it.
        assert_eq!(
            encode(
                &KeyPress {
                    alt: true,
                    ..key("f")
                },
                false,
                false
            ),
            None
        );
    }

    #[test]
    fn text_and_shortcuts_are_left_alone() {
        assert_eq!(enc(key("a")), None);
        assert_eq!(
            enc(KeyPress {
                platform: true,
                ..key("c")
            }),
            None
        );
        assert_eq!(
            enc(KeyPress {
                platform: true,
                ..key("enter")
            }),
            None
        );
    }

    #[test]
    fn meta_setting() {
        assert!(alt_is_meta("off", false), "Alt is Meta on Linux");
        assert!(!alt_is_meta("off", true));
        assert!(alt_is_meta("left", true));
        // The side held decides on the Mac.
        assert!(alt_is_meta_held("left", true, Some((true, false))));
        assert!(!alt_is_meta_held("left", true, Some((false, true))));
        assert!(alt_is_meta_held("right", true, Some((false, true))));
        assert!(!alt_is_meta_held("right", true, Some((true, false))));
        assert!(alt_is_meta_held("both", true, Some((false, true))));
    }

    #[test]
    fn pastes() {
        assert_eq!(paste_bytes("a\nb", false), b"a\rb".to_vec());
        assert_eq!(
            paste_bytes("x\x1b[201~y", true),
            b"\x1b[200~x[201~y\x1b[201~".to_vec()
        );
    }
}
