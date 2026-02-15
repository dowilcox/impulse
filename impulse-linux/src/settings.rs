use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

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

    // ── Keybinding overrides ─────────────────────────────────────────────
    #[serde(default)]
    pub keybinding_overrides: HashMap<String, String>,

    // ── Per-file-type overrides ───────────────────────────────────────────
    pub file_type_overrides: Vec<FileTypeOverride>,
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
            color_scheme: String::from("nord"),

            // Custom commands
            commands_on_save: Vec::new(),
            custom_keybindings: Vec::new(),

            // Keybinding overrides
            keybinding_overrides: HashMap::new(),

            // Per-file-type overrides
            file_type_overrides: Vec::new(),
        }
    }
}

pub fn matches_file_pattern(path: &str, pattern: &str) -> bool {
    impulse_core::util::matches_file_pattern(path, pattern)
}

fn settings_path() -> Option<PathBuf> {
    let config_dir = dirs::config_dir()?;
    let impulse_dir = config_dir.join("impulse");
    let _ = std::fs::create_dir_all(&impulse_dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&impulse_dir, std::fs::Permissions::from_mode(0o700)) {
            log::warn!("Failed to set permissions on {:?}: {}", impulse_dir, e);
        }
    }
    Some(impulse_dir.join("settings.json"))
}

pub fn load() -> Settings {
    let path = match settings_path() {
        Some(p) => p,
        None => {
            log::warn!("Cannot determine config directory; using default settings");
            return Settings::default();
        }
    };
    let mut settings = match std::fs::read_to_string(&path) {
        Ok(contents) => match serde_json::from_str(&contents) {
            Ok(s) => s,
            Err(e) => {
                log::error!(
                    "Failed to parse settings from {}: {}; using defaults",
                    path.display(),
                    e
                );
                Settings::default()
            }
        },
        Err(_) => Settings::default(),
    };
    migrate_format_on_save(&mut settings);
    settings
}

/// Migrates `format_on_save` entries from `FileTypeOverride` into
/// `CommandOnSave` entries with `reload_file: true`.
fn migrate_format_on_save(settings: &mut Settings) {
    let mut migrated = false;
    for ovr in &mut settings.file_type_overrides {
        if let Some(fmt) = ovr.format_on_save.take() {
            settings.commands_on_save.push(CommandOnSave {
                name: format!("Format ({})", ovr.pattern),
                command: fmt.command,
                args: fmt.args,
                file_pattern: ovr.pattern.clone(),
                reload_file: true,
            });
            migrated = true;
        }
    }
    if migrated {
        save(settings);
    }
}

pub fn save(settings: &Settings) {
    let path = match settings_path() {
        Some(p) => p,
        None => {
            log::error!("Cannot determine config directory; settings will not be saved");
            return;
        }
    };
    let json = match serde_json::to_string_pretty(settings) {
        Ok(j) => j,
        Err(e) => {
            log::error!("Failed to serialize settings: {}", e);
            return;
        }
    };
    // Atomic write: write to temp file, then rename
    let tmp_path = path.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp_path, &json) {
        log::error!("Failed to write settings to {}: {}", tmp_path.display(), e);
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600)) {
            log::warn!("Failed to set permissions on {:?}: {}", tmp_path, e);
        }
    }
    if let Err(e) = std::fs::rename(&tmp_path, &path) {
        log::error!("Failed to rename settings file: {}", e);
    }
}
