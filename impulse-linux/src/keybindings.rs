use std::collections::HashMap;

/// A keybinding definition with an action name and default key sequence.
#[derive(Debug, Clone)]
pub struct Keybinding {
    pub id: &'static str,
    pub label: &'static str,
    pub default_keys: &'static str,
    pub category: &'static str,
}

/// All built-in keybindings.
pub fn builtin_keybindings() -> Vec<Keybinding> {
    vec![
        // Tabs
        Keybinding {
            id: "new_terminal",
            label: "New Terminal Tab",
            default_keys: "Ctrl+T",
            category: "Tabs",
        },
        Keybinding {
            id: "close_tab",
            label: "Close Tab",
            default_keys: "Ctrl+W",
            category: "Tabs",
        },
        Keybinding {
            id: "next_tab",
            label: "Next Tab",
            default_keys: "Ctrl+Tab",
            category: "Tabs",
        },
        Keybinding {
            id: "prev_tab",
            label: "Previous Tab",
            default_keys: "Ctrl+Shift+Tab",
            category: "Tabs",
        },
        Keybinding {
            id: "tab_1",
            label: "Switch to Tab 1",
            default_keys: "Alt+1",
            category: "Tabs",
        },
        Keybinding {
            id: "tab_2",
            label: "Switch to Tab 2",
            default_keys: "Alt+2",
            category: "Tabs",
        },
        Keybinding {
            id: "tab_3",
            label: "Switch to Tab 3",
            default_keys: "Alt+3",
            category: "Tabs",
        },
        Keybinding {
            id: "tab_4",
            label: "Switch to Tab 4",
            default_keys: "Alt+4",
            category: "Tabs",
        },
        Keybinding {
            id: "tab_5",
            label: "Switch to Tab 5",
            default_keys: "Alt+5",
            category: "Tabs",
        },
        // Terminal
        Keybinding {
            id: "split_horizontal",
            label: "Split Horizontal",
            default_keys: "Ctrl+Shift+D",
            category: "Terminal",
        },
        Keybinding {
            id: "split_vertical",
            label: "Split Vertical",
            default_keys: "Ctrl+D",
            category: "Terminal",
        },
        Keybinding {
            id: "close_split",
            label: "Close Split",
            default_keys: "Ctrl+Shift+W",
            category: "Terminal",
        },
        Keybinding {
            id: "focus_next_terminal",
            label: "Focus Next Terminal",
            default_keys: "Ctrl+Shift+]",
            category: "Terminal",
        },
        Keybinding {
            id: "focus_prev_terminal",
            label: "Focus Previous Terminal",
            default_keys: "Ctrl+Shift+[",
            category: "Terminal",
        },
        Keybinding {
            id: "terminal_find",
            label: "Find in Terminal",
            default_keys: "Ctrl+Shift+F",
            category: "Terminal",
        },
        // Editor
        Keybinding {
            id: "save",
            label: "Save File",
            default_keys: "Ctrl+S",
            category: "Editor",
        },
        Keybinding {
            id: "go_to_line",
            label: "Go to Line",
            default_keys: "Ctrl+G",
            category: "Editor",
        },
        // Navigation
        Keybinding {
            id: "toggle_sidebar",
            label: "Toggle Sidebar",
            default_keys: "Ctrl+B",
            category: "Navigation",
        },
        Keybinding {
            id: "quick_open",
            label: "Quick Open",
            default_keys: "Ctrl+P",
            category: "Navigation",
        },
        Keybinding {
            id: "command_palette",
            label: "Command Palette",
            default_keys: "Ctrl+Shift+P",
            category: "Navigation",
        },
        Keybinding {
            id: "project_search",
            label: "Project Search",
            default_keys: "Ctrl+Shift+S",
            category: "Navigation",
        },
        // Font
        Keybinding {
            id: "increase_font",
            label: "Increase Font Size",
            default_keys: "Ctrl+plus",
            category: "Font",
        },
        Keybinding {
            id: "decrease_font",
            label: "Decrease Font Size",
            default_keys: "Ctrl+minus",
            category: "Font",
        },
        // App
        Keybinding {
            id: "settings",
            label: "Open Settings",
            default_keys: "Ctrl+comma",
            category: "App",
        },
    ]
}

/// Resolve effective keybinding given user overrides.
pub fn resolve_keybindings(overrides: &HashMap<String, String>) -> Vec<(String, String)> {
    builtin_keybindings()
        .into_iter()
        .map(|kb| {
            let keys = overrides
                .get(kb.id)
                .cloned()
                .unwrap_or_else(|| kb.default_keys.to_string());
            (kb.id.to_string(), keys)
        })
        .collect()
}
