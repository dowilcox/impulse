use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A command that runs automatically when a file matching the pattern is saved.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandOnSave {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub file_pattern: String,
}

/// A user-defined keybinding that runs a command.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CustomKeybinding {
    pub name: String,
    pub key: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// Application settings, persisted to `~/.config/impulse/settings.json`.
///
/// The `#[serde(default)]` on the struct ensures that any fields missing from
/// an existing settings file are filled in with their `Default` values, making
/// it safe to add new fields without breaking old config files.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // ── Window ───────────────────────────────────────────────────────────
    pub window_width: i32,
    pub window_height: i32,
    pub sidebar_visible: bool,
    pub sidebar_width: i32,
    pub last_directory: String,
    pub open_files: Vec<String>,

    // ── Editor ───────────────────────────────────────────────────────────
    pub auto_save: bool,
    pub font_size: i32,
    pub font_family: String,
    pub tab_width: u32,
    pub use_spaces: bool,
    pub show_line_numbers: bool,
    pub show_right_margin: bool,
    pub right_margin_position: u32,
    pub word_wrap: bool,
    pub highlight_current_line: bool,
    pub minimap_enabled: bool,
    pub render_whitespace: String,
    pub sticky_scroll: bool,
    pub bracket_pair_colorization: bool,
    pub indent_guides: bool,
    pub font_ligatures: bool,
    pub folding: bool,
    pub scroll_beyond_last_line: bool,
    pub smooth_scrolling: bool,
    pub editor_cursor_style: String,
    pub editor_cursor_blinking: String,

    // ── Terminal ─────────────────────────────────────────────────────────
    pub terminal_scrollback: i64,
    pub terminal_cursor_shape: String,
    pub terminal_cursor_blink: bool,
    pub terminal_bell: bool,
    pub terminal_font_family: String,
    pub terminal_font_size: i32,
    pub terminal_copy_on_select: bool,
    pub terminal_scroll_on_output: bool,
    pub terminal_allow_hyperlink: bool,
    pub terminal_bold_is_bright: bool,

    // ── Editor (additional) ──────────────────────────────────────────────
    pub editor_line_height: u32,
    pub editor_auto_closing_brackets: String,

    // ── Sidebar ────────────────────────────────────────────────────────
    pub sidebar_show_hidden: bool,

    // ── Appearance ───────────────────────────────────────────────────────
    pub color_scheme: String,

    // ── Custom commands ──────────────────────────────────────────────────
    pub commands_on_save: Vec<CommandOnSave>,
    pub custom_keybindings: Vec<CustomKeybinding>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            // Window
            window_width: 1200,
            window_height: 800,
            sidebar_visible: false,
            sidebar_width: 250,
            last_directory: String::new(),
            open_files: Vec::new(),

            // Editor
            auto_save: false,
            font_size: 14,
            font_family: String::from("monospace"),
            tab_width: 4,
            use_spaces: true,
            show_line_numbers: true,
            show_right_margin: true,
            right_margin_position: 120,
            word_wrap: false,
            highlight_current_line: true,
            minimap_enabled: false,
            render_whitespace: String::from("selection"),
            sticky_scroll: false,
            bracket_pair_colorization: true,
            indent_guides: true,
            font_ligatures: true,
            folding: true,
            scroll_beyond_last_line: false,
            smooth_scrolling: false,
            editor_cursor_style: String::from("line"),
            editor_cursor_blinking: String::from("smooth"),

            // Terminal
            terminal_scrollback: 10000,
            terminal_cursor_shape: String::from("block"),
            terminal_cursor_blink: true,
            terminal_bell: false,
            terminal_font_family: String::from("monospace"),
            terminal_font_size: 14,
            terminal_copy_on_select: true,
            terminal_scroll_on_output: false,
            terminal_allow_hyperlink: true,
            terminal_bold_is_bright: false,

            // Editor (additional)
            editor_line_height: 0,
            editor_auto_closing_brackets: String::from("languageDefined"),

            // Sidebar
            sidebar_show_hidden: false,

            // Appearance
            color_scheme: String::from("kanagawa"),

            // Custom commands
            commands_on_save: Vec::new(),
            custom_keybindings: Vec::new(),
        }
    }
}

fn settings_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let impulse_dir = PathBuf::from(home).join(".config").join("impulse");
    let _ = std::fs::create_dir_all(&impulse_dir);
    impulse_dir.join("settings.json")
}

pub fn load() -> Settings {
    let path = settings_path();
    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

pub fn save(settings: &Settings) {
    let path = settings_path();
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        let _ = std::fs::write(&path, json);
    }
}
