use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A formatter command that runs on save before the editor reloads the file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormatOnSave {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// Per-file-type overrides for editor settings (tab width, spaces, formatter).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileTypeOverride {
    pub pattern: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_spaces: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_on_save: Option<FormatOnSave>,
}

/// A command that runs automatically when a file matching the pattern is saved.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandOnSave {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub file_pattern: String,
    #[serde(default)]
    pub reload_file: bool,
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

/// Application settings shared across all frontends.
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
    pub editor_cursor_surrounding_lines: u32,
    pub editor_selection_highlight: bool,
    pub editor_occurrences_highlight: bool,
    pub editor_word_based_suggestions: String,

    // ── Sidebar ────────────────────────────────────────────────────────
    pub sidebar_show_hidden: bool,

    // ── Appearance ───────────────────────────────────────────────────────
    pub color_scheme: String,

    // ── Custom commands ──────────────────────────────────────────────────
    pub commands_on_save: Vec<CommandOnSave>,
    pub custom_keybindings: Vec<CustomKeybinding>,

    // ── Keybinding overrides ─────────────────────────────────────────────
    #[serde(default)]
    pub keybinding_overrides: HashMap<String, String>,

    // ── Per-file-type overrides ───────────────────────────────────────────
    pub file_type_overrides: Vec<FileTypeOverride>,

    // ── Updates ──────────────────────────────────────────────────────────
    pub check_for_updates: bool,
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
            font_family: String::from("JetBrains Mono"),
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
            terminal_font_family: String::from("JetBrains Mono"),
            terminal_font_size: 14,
            terminal_copy_on_select: true,
            terminal_scroll_on_output: false,
            terminal_allow_hyperlink: true,
            terminal_bold_is_bright: false,

            // Editor (additional)
            editor_line_height: 0,
            editor_auto_closing_brackets: String::from("languageDefined"),
            editor_cursor_surrounding_lines: 3,
            editor_selection_highlight: true,
            editor_occurrences_highlight: true,
            editor_word_based_suggestions: String::from("matchingDocuments"),

            // Sidebar
            sidebar_show_hidden: false,

            // Appearance
            color_scheme: String::from("nord"),

            // Custom commands
            commands_on_save: Vec::new(),
            custom_keybindings: Vec::new(),

            // Keybinding overrides
            keybinding_overrides: HashMap::new(),

            // Per-file-type overrides
            file_type_overrides: Vec::new(),

            // Updates
            check_for_updates: true,
        }
    }
}

impl Settings {
    /// Deserialize settings from JSON, applying migrations and validation.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let mut settings: Settings =
            serde_json::from_str(json).map_err(|e| format!("Failed to parse settings: {}", e))?;
        settings.migrate();
        settings.validate();
        Ok(settings)
    }

    /// Serialize settings to pretty-printed JSON.
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize settings: {}", e))
    }

    /// Serialize default settings to JSON.
    pub fn default_json() -> String {
        serde_json::to_string_pretty(&Settings::default()).expect("default settings must serialize")
    }

    /// Clamp settings values to valid ranges.
    pub fn validate(&mut self) {
        self.font_size = self.font_size.clamp(6, 72);
        self.terminal_font_size = self.terminal_font_size.clamp(6, 72);
        if self.terminal_scrollback > 1_000_000 {
            self.terminal_scrollback = 1_000_000;
        }
        self.tab_width = self.tab_width.clamp(1, 16);
        self.right_margin_position = self.right_margin_position.clamp(1, 500);
        self.sidebar_width = self.sidebar_width.clamp(100, 1000);
        self.editor_line_height = self.editor_line_height.min(100);
        self.window_width = self.window_width.clamp(400, 10000);
        self.window_height = self.window_height.clamp(300, 10000);
    }

    /// Run all pending migrations.
    pub fn migrate(&mut self) {
        self.migrate_format_on_save();
        self.migrate_default_font();
    }

    /// Migrates the default font from old platform defaults to "JetBrains Mono".
    fn migrate_default_font(&mut self) {
        let old_defaults = ["monospace", "SF Mono", ""];
        if old_defaults.iter().any(|d| self.font_family == *d) {
            self.font_family = String::from("JetBrains Mono");
        }
        if old_defaults.iter().any(|d| self.terminal_font_family == *d) {
            self.terminal_font_family = String::from("JetBrains Mono");
        }
    }

    /// Migrates `format_on_save` entries from `FileTypeOverride` into
    /// `CommandOnSave` entries with `reload_file: true`.
    fn migrate_format_on_save(&mut self) {
        for ovr in &mut self.file_type_overrides {
            if let Some(fmt) = ovr.format_on_save.take() {
                self.commands_on_save.push(CommandOnSave {
                    name: format!("Format ({})", ovr.pattern),
                    command: fmt.command,
                    args: fmt.args,
                    file_pattern: ovr.pattern.clone(),
                    reload_file: true,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_roundtrip() {
        let settings = Settings::default();
        let json = settings.to_json().unwrap();
        let parsed = Settings::from_json(&json).unwrap();
        assert_eq!(parsed.font_size, 14);
        assert_eq!(parsed.color_scheme, "nord");
    }

    #[test]
    fn missing_fields_get_defaults() {
        let json = r#"{"font_size": 20}"#;
        let settings = Settings::from_json(json).unwrap();
        assert_eq!(settings.font_size, 20);
        assert_eq!(settings.color_scheme, "nord");
        assert_eq!(settings.tab_width, 4);
    }

    #[test]
    fn validation_clamps_values() {
        let json = r#"{"font_size": 200, "tab_width": 0, "terminal_scrollback": 9999999}"#;
        let settings = Settings::from_json(json).unwrap();
        assert_eq!(settings.font_size, 72);
        assert_eq!(settings.tab_width, 1);
        assert_eq!(settings.terminal_scrollback, 1_000_000);
    }

    #[test]
    fn empty_json_returns_defaults() {
        let settings = Settings::from_json("{}").unwrap();
        assert_eq!(settings.font_size, 14);
        assert_eq!(settings.font_family, "JetBrains Mono");
    }

    #[test]
    fn format_on_save_migration() {
        let json = r#"{
            "file_type_overrides": [{
                "pattern": "*.rs",
                "format_on_save": {"command": "rustfmt", "args": ["--edition", "2021"]}
            }]
        }"#;
        let settings = Settings::from_json(json).unwrap();
        assert!(settings.file_type_overrides[0].format_on_save.is_none());
        assert_eq!(settings.commands_on_save.len(), 1);
        assert_eq!(settings.commands_on_save[0].command, "rustfmt");
        assert!(settings.commands_on_save[0].reload_file);
    }

    #[test]
    fn font_migration() {
        let json = r#"{"font_family": "monospace", "terminal_font_family": "SF Mono"}"#;
        let settings = Settings::from_json(json).unwrap();
        assert_eq!(settings.font_family, "JetBrains Mono");
        assert_eq!(settings.terminal_font_family, "JetBrains Mono");
    }
}
