use std::collections::HashMap;

/// A built-in keyboard shortcut with its default accelerator.
pub struct BuiltinKeybinding {
    pub id: &'static str,
    pub description: &'static str,
    pub category: &'static str,
    pub default_accel: &'static str,
}

pub const BUILTIN_KEYBINDINGS: &[BuiltinKeybinding] = &[
    // Tabs
    BuiltinKeybinding {
        id: "new_tab",
        description: "New Terminal Tab",
        category: "Tabs",
        default_accel: "<Ctrl>t",
    },
    BuiltinKeybinding {
        id: "close_tab",
        description: "Close Tab",
        category: "Tabs",
        default_accel: "<Ctrl>w",
    },
    BuiltinKeybinding {
        id: "next_tab",
        description: "Next Tab",
        category: "Tabs",
        default_accel: "<Ctrl>Tab",
    },
    BuiltinKeybinding {
        id: "prev_tab",
        description: "Previous Tab",
        category: "Tabs",
        default_accel: "<Ctrl><Shift>Tab",
    },
    // Terminal
    BuiltinKeybinding {
        id: "paste",
        description: "Paste",
        category: "Terminal",
        default_accel: "<Ctrl><Shift>v",
    },
    BuiltinKeybinding {
        id: "copy",
        description: "Copy",
        category: "Terminal",
        default_accel: "<Ctrl><Shift>c",
    },
    BuiltinKeybinding {
        id: "split_horizontal",
        description: "Split Horizontally",
        category: "Terminal",
        default_accel: "<Ctrl><Shift>e",
    },
    BuiltinKeybinding {
        id: "split_vertical",
        description: "Split Vertically",
        category: "Terminal",
        default_accel: "<Ctrl><Shift>o",
    },
    BuiltinKeybinding {
        id: "focus_prev_split",
        description: "Focus Previous Split",
        category: "Terminal",
        default_accel: "<Alt>Left",
    },
    BuiltinKeybinding {
        id: "focus_next_split",
        description: "Focus Next Split",
        category: "Terminal",
        default_accel: "<Alt>Right",
    },
    // Editor
    BuiltinKeybinding {
        id: "save",
        description: "Save File",
        category: "Editor",
        default_accel: "<Ctrl>s",
    },
    BuiltinKeybinding {
        id: "find",
        description: "Find",
        category: "Editor",
        default_accel: "<Ctrl>f",
    },
    BuiltinKeybinding {
        id: "go_to_line",
        description: "Go to Line",
        category: "Editor",
        default_accel: "<Ctrl>g",
    },
    // Navigation
    BuiltinKeybinding {
        id: "toggle_sidebar",
        description: "Toggle Sidebar",
        category: "Navigation",
        default_accel: "<Ctrl><Shift>b",
    },
    BuiltinKeybinding {
        id: "project_search",
        description: "Find in Project",
        category: "Navigation",
        default_accel: "<Ctrl><Shift>f",
    },
    BuiltinKeybinding {
        id: "command_palette",
        description: "Command Palette",
        category: "Navigation",
        default_accel: "<Ctrl><Shift>p",
    },
    BuiltinKeybinding {
        id: "open_settings",
        description: "Open Settings",
        category: "Navigation",
        default_accel: "<Ctrl>comma",
    },
    // Font
    BuiltinKeybinding {
        id: "font_increase",
        description: "Increase Font Size",
        category: "Font",
        default_accel: "<Ctrl>equal",
    },
    BuiltinKeybinding {
        id: "font_decrease",
        description: "Decrease Font Size",
        category: "Font",
        default_accel: "<Ctrl>minus",
    },
    BuiltinKeybinding {
        id: "font_reset",
        description: "Reset Font Size",
        category: "Font",
        default_accel: "<Ctrl>0",
    },
    // App
    BuiltinKeybinding {
        id: "new_window",
        description: "New Window",
        category: "App",
        default_accel: "<Ctrl><Shift>n",
    },
    BuiltinKeybinding {
        id: "fullscreen",
        description: "Toggle Fullscreen",
        category: "App",
        default_accel: "F11",
    },
];

/// Returns the GTK accel string for a given keybinding ID, using the override
/// if present, otherwise the built-in default.
pub fn get_accel(id: &str, overrides: &HashMap<String, String>) -> String {
    if let Some(display_str) = overrides.get(id) {
        let accel = parse_keybinding_to_accel(display_str);
        if !accel.is_empty() {
            return accel;
        }
    }
    BUILTIN_KEYBINDINGS
        .iter()
        .find(|kb| kb.id == id)
        .map(|kb| kb.default_accel.to_string())
        .unwrap_or_default()
}

/// Converts a GTK accel string like `"<Ctrl><Shift>b"` to a human-readable
/// display string like `"Ctrl+Shift+B"`.
pub fn accel_to_display(accel: &str) -> String {
    let mut parts = Vec::new();
    let mut remaining = accel;

    while remaining.starts_with('<') {
        if let Some(end) = remaining.find('>') {
            let modifier = &remaining[1..end];
            match modifier.to_lowercase().as_str() {
                "ctrl" | "control" => parts.push("Ctrl"),
                "shift" => parts.push("Shift"),
                "alt" => parts.push("Alt"),
                "super" => parts.push("Super"),
                _ => parts.push(modifier),
            }
            remaining = &remaining[end + 1..];
        } else {
            break;
        }
    }

    if !remaining.is_empty() {
        let key_display = match remaining {
            "comma" => ",",
            "period" => ".",
            "equal" => "=",
            "minus" => "-",
            "Tab" => "Tab",
            "Left" => "Left",
            "Right" => "Right",
            "Up" => "Up",
            "Down" => "Down",
            other => other,
        };
        parts.push(key_display);
    }

    parts.join("+")
}

/// Converts a human-readable keybinding string like `"Ctrl+Shift+B"` into a
/// GTK accelerator string like `"<Ctrl><Shift>b"`.
pub fn parse_keybinding_to_accel(key: &str) -> String {
    let parts: Vec<&str> = key.split('+').collect();
    if parts.is_empty() {
        return String::new();
    }
    let mut accel = String::new();
    for part in &parts[..parts.len() - 1] {
        match part.trim().to_lowercase().as_str() {
            "ctrl" | "control" => accel.push_str("<Ctrl>"),
            "shift" => accel.push_str("<Shift>"),
            "alt" => accel.push_str("<Alt>"),
            "super" => accel.push_str("<Super>"),
            _ => return String::new(),
        }
    }
    accel.push_str(parts.last().unwrap().trim());
    accel
}

/// Returns the ordered list of keybinding categories for display purposes.
pub fn categories() -> &'static [&'static str] {
    &["Tabs", "Terminal", "Editor", "Navigation", "Font", "App"]
}

/// Parsed representation of a keybinding for efficient matching in event handlers.
pub struct ParsedAccel {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub super_: bool,
    /// The lowercase key name (e.g. "g", "tab", "f11")
    pub key_lower: String,
}

/// Parse a GTK accel string like `"<Ctrl><Shift>g"` into a `ParsedAccel`
/// for matching against `EventControllerKey` events.
pub fn parse_accel(accel: &str) -> Option<ParsedAccel> {
    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    let mut super_ = false;
    let mut remaining = accel;

    while remaining.starts_with('<') {
        if let Some(end) = remaining.find('>') {
            let modifier = &remaining[1..end];
            match modifier.to_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "shift" => shift = true,
                "alt" => alt = true,
                "super" => super_ = true,
                _ => {}
            }
            remaining = &remaining[end + 1..];
        } else {
            break;
        }
    }

    if remaining.is_empty() {
        return None;
    }

    Some(ParsedAccel {
        ctrl,
        shift,
        alt,
        super_,
        key_lower: remaining.to_lowercase(),
    })
}

/// Check if a pressed key + modifiers matches a `ParsedAccel`.
pub fn matches_key(
    parsed: &ParsedAccel,
    key: gtk4::gdk::Key,
    modifiers: gtk4::gdk::ModifierType,
) -> bool {
    let ctrl = modifiers.contains(gtk4::gdk::ModifierType::CONTROL_MASK);
    let shift = modifiers.contains(gtk4::gdk::ModifierType::SHIFT_MASK);
    let alt = modifiers.contains(gtk4::gdk::ModifierType::ALT_MASK);
    let super_ = modifiers.contains(gtk4::gdk::ModifierType::SUPER_MASK);

    if ctrl != parsed.ctrl || shift != parsed.shift || alt != parsed.alt || super_ != parsed.super_
    {
        return false;
    }

    // Get the key name and compare case-insensitively
    let key_name = key
        .name()
        .map(|n| n.to_string().to_lowercase())
        .unwrap_or_default();
    key_name == parsed.key_lower
}
