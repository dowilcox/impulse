//! `settings.json`, as the Swift `Settings` reads and writes it: the same
//! snake_case keys and defaults, so one file works for both apps. Decoding
//! is per key: a missing or mistyped value takes its default and the rest
//! still load. A file that doesn't parse at all is copied aside at launch,
//! the defaults are used, and saves pause until it's fixed, so Impulse never
//! overwrites what the user was editing. A save that would overwrite a
//! change made on disk since Impulse last read or wrote the file doesn't
//! write; the app merges (`Settings::merged`) and saves again.
//!
//! `catalog()` describes every user-facing setting once (the Swift
//! `SettingsCatalog`): its title, category, section and control for the
//! Settings tab, and its JSON Schema entry.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// A formatter command that runs on save before the editor reloads the file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FormatOnSave {
    pub command: String,
    pub args: Vec<String>,
}

/// Per-file-type overrides for editor settings (tab width, spaces, formatter).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FileTypeOverride {
    pub pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_width: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_spaces: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_on_save: Option<FormatOnSave>,
}

/// A command that runs when a file matching the pattern is saved.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CommandOnSave {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub file_pattern: String,
    pub reload_file: bool,
}

/// A user-defined shortcut that runs a shell command line.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CustomKeybinding {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub key: String,
    /// The command line, as typed at a shell prompt.
    pub command: String,
    /// Extra arguments added after `command`, each quoted.
    #[serde(default)]
    pub args: Vec<String>,
}

/// A value of `T` from settings.json, or `None` when it isn't one. A whole
/// number written with a fraction (`14.0`) is still a whole number, as
/// Foundation's decoder takes it.
fn decode<T: serde::de::DeserializeOwned>(value: &Value) -> Option<T> {
    serde_json::from_value(value.clone()).ok().or_else(|| {
        let whole = value
            .as_f64()
            .filter(|f| f.fract() == 0.0 && f.abs() < 9e15)?;
        serde_json::from_value(Value::from(whole as i64)).ok()
    })
}

macro_rules! settings {
    ($( $(#[$meta:meta])* $field:ident : $ty:ty = $key:literal => $default:expr, )*) => {
        /// Impulse's settings. See the module docs.
        #[derive(Clone, Debug, PartialEq)]
        pub struct Settings {
            $( $(#[$meta])* pub $field: $ty, )*
        }

        impl Default for Settings {
            fn default() -> Self {
                Settings { $( $field: $default, )* }
            }
        }

        impl Settings {
            /// Every settings.json key, in declaration order.
            pub const KEYS: &'static [&'static str] = &[$($key),*];

            /// Settings from a parsed settings.json, each key on its own:
            /// missing or mistyped values take their defaults.
            pub fn from_json(value: &Value) -> Settings {
                let mut settings = Settings::default();
                if let Some(object) = value.as_object() {
                    $(
                        if let Some(v) = object.get($key).and_then(decode::<$ty>) {
                            settings.$field = v;
                        }
                    )*
                }
                settings
            }

            /// The settings as settings.json holds them (keys sorted).
            pub fn to_json(&self) -> Value {
                let mut object = Map::new();
                $( object.insert($key.to_string(), json!(self.$field)); )*
                Value::Object(object)
            }

            /// The value of one key, as JSON.
            pub fn get(&self, key: &str) -> Option<Value> {
                match key {
                    $( $key => Some(json!(self.$field)), )*
                    _ => None,
                }
            }

            /// Set one key from JSON; false when the key is unknown or the
            /// value isn't its type.
            pub fn set(&mut self, key: &str, value: &Value) -> bool {
                match key {
                    $( $key => match decode::<$ty>(value) {
                        Some(v) => { self.$field = v; true }
                        None => false,
                    }, )*
                    _ => false,
                }
            }
        }
    };
}

/// The default for `agent_task_limit` (the Swift `AgentTaskPolicy.defaultLimit`).
pub const DEFAULT_AGENT_TASK_LIMIT: i64 = 4;
/// The sidebar's default width (the Swift `Metrics.leftDockDefaultWidth`).
pub const DEFAULT_SIDEBAR_WIDTH: i64 = 260;
const DEFAULT_FONT: &str = "JetBrains Mono";

settings! {
    // -- Window --
    window_width: i64 = "window_width" => 1200,
    window_height: i64 = "window_height" => 800,
    sidebar_visible: bool = "sidebar_visible" => true,
    sidebar_width: i64 = "sidebar_width" => DEFAULT_SIDEBAR_WIDTH,
    confirm_close_warnings: bool = "confirm_close_warnings" => true,
    restore_session: bool = "restore_session" => true,
    /// Bring back each terminal's recent output when restoring a session.
    restore_scrollback: bool = "restore_scrollback" => true,
    /// Keep command history across terminals and launches.
    terminal_persistent_history: bool = "terminal_persistent_history" => true,
    /// New terminals set $EDITOR / $VISUAL to `impulse edit`.
    terminal_editor_integration: bool = "terminal_editor_integration" => false,
    /// Open the composer when the focused terminal's agent asks for input.
    agent_composer_auto_show: bool = "agent_composer_auto_show" => false,
    /// Notify when two of a repository's workspaces start changing the same files.
    task_overlap_notify: bool = "task_overlap_notify" => true,
    agent_hook_task_summary: bool = "agent_hook_task_summary" => true,
    agent_hook_shared_files: bool = "agent_hook_shared_files" => true,
    agent_hook_merge_guard: bool = "agent_hook_merge_guard" => true,
    agent_hook_dependencies: bool = "agent_hook_dependencies" => true,
    /// ask, trusted_projects or off.
    agent_task_creation: String = "agent_task_creation" => "ask".into(),
    /// Open tasks agents asked for, per repository (0: no limit).
    agent_task_limit: i64 = "agent_task_limit" => DEFAULT_AGENT_TASK_LIMIT,
    /// workspace, window or off.
    agent_tool_terminal_output: String = "agent_tool_terminal_output" => "workspace".into(),
    quick_terminal_enabled: bool = "quick_terminal_enabled" => false,
    quick_terminal_shortcut: String = "quick_terminal_shortcut" => "Ctrl+`".into(),
    /// on, off, offUnlessPressed, onUnlessPressed.
    editor_inlay_hints: String = "editor_inlay_hints" => "on".into(),
    terminal_shell_completions: bool = "terminal_shell_completions" => false,
    /// off, left, right or both.
    terminal_option_as_meta: String = "terminal_option_as_meta" => "off".into(),
    editor_preview_tabs: bool = "editor_preview_tabs" => true,
    editor_vim_mode: bool = "editor_vim_mode" => false,
    /// Where the Scratch workspace's terminals start ("" = home; "~" works).
    scratch_directory: String = "scratch_directory" => String::new(),
    /// Ask before running code from a folder until it's trusted.
    ask_to_trust_folders: bool = "workspace_trust" => true,
    git_commit_and_push: bool = "git_commit_and_push" => false,
    git_push_tags_on_create: bool = "git_push_tags_on_create" => false,
    git_push_follow_tags: bool = "git_push_follow_tags" => false,
    /// ff-only, rebase or merge.
    git_pull_mode: String = "git_pull_mode" => "ff-only".into(),
    /// Fetch open repositories every N minutes (0 = off).
    git_auto_fetch_minutes: i64 = "git_auto_fetch_minutes" => 0,
    review_context_lines: i64 = "review_context_lines" => 3,
    last_directory: String = "last_directory" => String::new(),
    open_files: Vec<String> = "open_files" => Vec::new(),
    // -- Editor --
    auto_save: bool = "auto_save" => false,
    font_size: i64 = "font_size" => 14,
    font_family: String = "font_family" => DEFAULT_FONT.into(),
    tab_width: i64 = "tab_width" => 4,
    use_spaces: bool = "use_spaces" => true,
    show_line_numbers: bool = "show_line_numbers" => true,
    show_right_margin: bool = "show_right_margin" => true,
    right_margin_position: i64 = "right_margin_position" => 120,
    word_wrap: bool = "word_wrap" => false,
    highlight_current_line: bool = "highlight_current_line" => true,
    minimap_enabled: bool = "minimap_enabled" => false,
    render_whitespace: String = "render_whitespace" => "selection".into(),
    sticky_scroll: bool = "sticky_scroll" => false,
    bracket_pair_colorization: bool = "bracket_pair_colorization" => true,
    indent_guides: bool = "indent_guides" => true,
    font_ligatures: bool = "font_ligatures" => true,
    folding: bool = "folding" => true,
    scroll_beyond_last_line: bool = "scroll_beyond_last_line" => false,
    smooth_scrolling: bool = "smooth_scrolling" => false,
    editor_cursor_style: String = "editor_cursor_style" => "line".into(),
    editor_cursor_blinking: String = "editor_cursor_blinking" => "smooth".into(),
    // -- Terminal --
    terminal_scrollback: i64 = "terminal_scrollback" => 10000,
    terminal_cursor_shape: String = "terminal_cursor_shape" => "block".into(),
    terminal_cursor_blink: bool = "terminal_cursor_blink" => true,
    terminal_bell: bool = "terminal_bell" => true,
    terminal_attention_on_bell: bool = "terminal_attention_on_bell" => true,
    terminal_font_family: String = "terminal_font_family" => DEFAULT_FONT.into(),
    terminal_font_size: i64 = "terminal_font_size" => 14,
    terminal_copy_on_select: bool = "terminal_copy_on_select" => true,
    terminal_scroll_on_output: bool = "terminal_scroll_on_output" => true,
    terminal_allow_hyperlink: bool = "terminal_allow_hyperlink" => true,
    terminal_allow_notifications: bool = "terminal_allow_notifications" => true,
    /// Play Impulse's chime with desktop notifications (Rust app; the Swift
    /// app plays the system's default sound and doesn't read this key).
    notification_sound: bool = "notification_sound" => true,
    terminal_attention_on_long_command: bool = "terminal_attention_on_long_command" => true,
    terminal_long_command_seconds: i64 = "terminal_long_command_seconds" => 30,
    terminal_bold_is_bright: bool = "terminal_bold_is_bright" => true,
    /// Minimum WCAG contrast ratio (1 to 21); 1 turns the adjustment off.
    terminal_minimum_contrast: f64 = "terminal_minimum_contrast" => 3.0,
    terminal_allow_osc52_write: bool = "terminal_allow_osc52_write" => true,
    terminal_allow_osc52_read: bool = "terminal_allow_osc52_read" => false,
    terminal_blocks: bool = "terminal_blocks" => true,
    terminal_context_bar: bool = "terminal_context_bar" => true,
    // -- Tabs --
    sidebar_tabs: bool = "sidebar_tabs" => false,
    // -- Editor (additional) --
    editor_line_height: i64 = "editor_line_height" => 0,
    editor_auto_closing_brackets: String = "editor_auto_closing_brackets" => "languageDefined".into(),
    editor_cursor_surrounding_lines: i64 = "editor_cursor_surrounding_lines" => 3,
    editor_selection_highlight: bool = "editor_selection_highlight" => true,
    editor_occurrences_highlight: bool = "editor_occurrences_highlight" => true,
    editor_word_based_suggestions: String = "editor_word_based_suggestions" => "matchingDocuments".into(),
    // -- Sidebar --
    sidebar_show_hidden: bool = "sidebar_show_hidden" => false,
    // -- Appearance --
    color_scheme: String = "color_scheme" => "nord".into(),
    // -- Custom commands --
    commands_on_save: Vec<CommandOnSave> = "commands_on_save" => Vec::new(),
    custom_keybindings: Vec<CustomKeybinding> = "custom_keybindings" => Vec::new(),
    keybinding_overrides: BTreeMap<String, String> = "keybinding_overrides" => BTreeMap::new(),
    file_type_overrides: Vec<FileTypeOverride> = "file_type_overrides" => Vec::new(),
    // -- Updates --
    check_for_updates: bool = "check_for_updates" => true,
}

impl Settings {
    /// Keep values in the ranges the Settings tab offers, so a hand-edited
    /// file can't hold values the UI can't, and normalize the choices whose
    /// spelling changed.
    pub fn validate(&mut self) {
        for item in catalog() {
            match item.control {
                Control::Integer { min, max, .. } => {
                    if let Some(v) = self.get(item.key).and_then(|v| v.as_i64()) {
                        self.set(item.key, &json!(v.clamp(min, max)));
                    }
                }
                Control::Decimal { min, max, .. } => {
                    if let Some(v) = self.get(item.key).and_then(|v| v.as_f64()) {
                        self.set(item.key, &json!(v.clamp(min, max)));
                    }
                }
                _ => {}
            }
        }
        self.sidebar_width = self.sidebar_width.clamp(100, 1000);
        // The settings UI once offered "bar" while the terminal only knows "beam".
        self.terminal_cursor_shape = match self.terminal_cursor_shape.to_lowercase().as_str() {
            "bar" | "beam" | "ibeam" => "beam",
            "underline" => "underline",
            _ => "block",
        }
        .into();
        let meta = self.terminal_option_as_meta.to_lowercase();
        self.terminal_option_as_meta = match meta.as_str() {
            "left" | "right" | "both" => meta,
            _ => "off".into(),
        };
        if !["ask", "trusted_projects", "off"].contains(&self.agent_task_creation.as_str()) {
            self.agent_task_creation = "ask".into();
        }
        if !["workspace", "window", "off"].contains(&self.agent_tool_terminal_output.as_str()) {
            self.agent_tool_terminal_output = "workspace".into();
        }
        self.window_width = self.window_width.clamp(400, 10000);
        self.window_height = self.window_height.clamp(300, 10000);
    }

    /// Move fonts still on an old platform default ("monospace", "SF Mono")
    /// to the bundled JetBrains Mono. True when something changed.
    pub fn migrate_default_font(&mut self) -> bool {
        let old = |font: &str| ["monospace", "SF Mono", ""].contains(&font);
        let mut changed = false;
        for font in [&mut self.font_family, &mut self.terminal_font_family] {
            if old(font) {
                *font = DEFAULT_FONT.into();
                changed = true;
            }
        }
        changed
    }

    /// `theirs` (the file as it is now) with the changes made in Impulse
    /// since `base` (the file as Impulse last read or wrote it) on top, key
    /// by key. Where both changed a key, Impulse's value wins.
    pub fn merged(ours: &Settings, theirs: &Settings, base: &Settings) -> Settings {
        if ours == base {
            return theirs.clone();
        }
        let mut result = theirs.clone();
        for key in Settings::KEYS {
            let mine = ours.get(key);
            if mine != base.get(key) {
                if let Some(value) = mine {
                    result.set(key, &value);
                }
            }
        }
        result.validate();
        result
    }

    /// Tab width and spaces-or-tabs for a file: each from the first
    /// file-type override that matches and sets it, else the editor's own.
    pub fn indentation(&self, path: &str) -> (i64, bool) {
        let matching: Vec<&FileTypeOverride> = self
            .file_type_overrides
            .iter()
            .filter(|o| crate::glob::matches_file_pattern(path, &o.pattern))
            .collect();
        (
            matching
                .iter()
                .filter_map(|o| o.tab_width)
                .find(|w| *w > 0)
                .unwrap_or(self.tab_width),
            matching
                .iter()
                .find_map(|o| o.use_spaces)
                .unwrap_or(self.use_spaces),
        )
    }

    /// The formatter for a file, from the first matching override that has one.
    pub fn format_on_save(&self, path: &str) -> Option<&FormatOnSave> {
        self.file_type_overrides
            .iter()
            .filter(|o| crate::glob::matches_file_pattern(path, &o.pattern))
            .find_map(|o| o.format_on_save.as_ref())
    }

    /// The commands to run after saving `path`.
    pub fn commands_on_save_for<'a>(
        &'a self,
        path: &'a str,
    ) -> impl Iterator<Item = &'a CommandOnSave> {
        self.commands_on_save
            .iter()
            .filter(move |c| crate::glob::matches_file_pattern(path, &c.file_pattern))
    }

    /// The settings as settings.json holds them: pretty, keys sorted.
    pub fn encode(&self) -> String {
        let mut text = serde_json::to_string_pretty(&self.to_json()).unwrap_or_default();
        text.push('\n');
        text
    }
}

// MARK: - The file

/// Why the settings in use aren't the file's.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadWarning {
    pub settings_path: PathBuf,
    /// Where a copy of the broken file went, at launch.
    pub backup_path: Option<PathBuf>,
    pub message: String,
    /// True when the defaults are in use (the file was broken at launch);
    /// false when the settings Impulse had are kept.
    pub using_defaults: bool,
}

/// What became of a save.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    /// Written, or the file already held exactly these settings.
    Saved,
    /// settings.json changed since Impulse last read or wrote it, so
    /// nothing was written.
    ChangedOnDisk,
    /// Not written: the file doesn't parse (saves pause until it's fixed),
    /// or writing failed.
    Failed,
}

/// settings.json and what Impulse knows about it: the content it last read
/// or wrote, and whether saves are paused.
#[derive(Debug)]
pub struct SettingsFile {
    pub path: PathBuf,
    /// False for headless runs that mustn't write files.
    pub persistence: bool,
    snapshot: Option<String>,
    /// The file's size, change time and inode when its content last
    /// matched `snapshot`: an unchanged stat skips reading and hashing it
    /// on each check.
    checked: std::cell::Cell<Option<FileStamp>>,
    blocked: bool,
    pub warning: Option<LoadWarning>,
}

type FileStamp = (u64, std::time::SystemTime, u64);

fn file_stamp(path: &Path) -> Option<FileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    #[cfg(unix)]
    let inode = std::os::unix::fs::MetadataExt::ino(&meta);
    #[cfg(not(unix))]
    let inode = 0;
    Some((meta.len(), meta.modified().ok()?, inode))
}

/// The settings.json path in the data directory.
pub fn settings_path(dev: bool) -> PathBuf {
    crate::paths::data_dir(dev).join("settings.json")
}

fn content_hash(data: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in data {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{}:{hash:016x}", data.len())
}

impl SettingsFile {
    pub fn new(path: PathBuf) -> Self {
        SettingsFile {
            path,
            persistence: true,
            snapshot: None,
            checked: std::cell::Cell::new(None),
            blocked: false,
            warning: None,
        }
    }

    /// Read the settings. `reloading`: the file changed while Impulse runs,
    /// and the caller keeps its settings when this returns `None`. At launch
    /// a broken file gives the defaults (and a copy of it beside it).
    pub fn load(&mut self, reloading: bool) -> Option<Settings> {
        let using_defaults = !reloading || self.warning.as_ref().is_some_and(|w| w.using_defaults);
        let earlier_backup = if using_defaults {
            self.warning.as_ref().and_then(|w| w.backup_path.clone())
        } else {
            None
        };
        let data = match std::fs::read(&self.path) {
            Ok(data) => data,
            Err(error) => {
                self.snapshot = None;
                if self.path.exists() {
                    log::error!(
                        "Failed to read settings from {}: {error}",
                        self.path.display()
                    );
                    self.warning = Some(LoadWarning {
                        settings_path: self.path.clone(),
                        backup_path: if reloading { earlier_backup } else { None },
                        message: error.to_string(),
                        using_defaults,
                    });
                    self.blocked = true;
                    return using_defaults.then(Settings::default);
                }
                // A missing file is expected on first launch.
                self.warning = None;
                self.blocked = false;
                return Some(Settings::default());
            }
        };
        match serde_json::from_slice::<Value>(&data) {
            Ok(value) if value.is_object() => {
                self.warning = None;
                self.blocked = false;
                self.snapshot = Some(content_hash(&data));
                let mut settings = Settings::from_json(&value);
                if settings.migrate_default_font() {
                    self.save(&settings);
                }
                settings.validate();
                Some(settings)
            }
            parsed => {
                let message = match parsed {
                    Err(error) => error.to_string(),
                    Ok(_) => "The settings file isn't a JSON object.".into(),
                };
                let backup = if reloading || !self.persistence {
                    None
                } else {
                    backup_invalid_file(&self.path, &data)
                };
                log::error!(
                    "Failed to decode settings from {}: {message}",
                    self.path.display()
                );
                self.warning = Some(LoadWarning {
                    settings_path: self.path.clone(),
                    backup_path: if reloading { earlier_backup } else { backup },
                    message,
                    using_defaults,
                });
                self.blocked = true;
                self.snapshot = None;
                using_defaults.then(Settings::default)
            }
        }
    }

    /// settings.json changed since Impulse last read or wrote it.
    pub fn changed_on_disk(&self) -> bool {
        let stamp = file_stamp(&self.path);
        if stamp.is_some() && stamp == self.checked.get() {
            return false;
        }
        let changed = self.change_since_load().is_some();
        self.checked.set(if changed { None } else { stamp });
        changed
    }

    fn change_since_load(&self) -> Option<String> {
        match std::fs::read(&self.path) {
            Ok(data) => match &self.snapshot {
                None => Some("it was created after startup".into()),
                Some(hash) if *hash != content_hash(&data) => {
                    Some("it changed on disk since it was loaded".into())
                }
                Some(_) => None,
            },
            Err(error) if self.path.exists() => Some(format!("it couldn't be checked: {error}")),
            Err(_) => self
                .snapshot
                .as_ref()
                .map(|_| "it was removed after it was loaded".into()),
        }
    }

    /// Write `settings` unless saves are paused, the file already holds
    /// them, or it changed since Impulse last read or wrote it. 0600,
    /// through a symlink rather than over it.
    pub fn save(&mut self, settings: &Settings) -> SaveOutcome {
        if !self.persistence {
            return SaveOutcome::Saved;
        }
        if self.blocked {
            log::error!(
                "Skipping settings save to keep the invalid file {}",
                self.path.display()
            );
            return SaveOutcome::Failed;
        }
        let data = settings.encode();
        let hash = content_hash(data.as_bytes());
        if self.snapshot.as_deref() == Some(hash.as_str()) {
            return SaveOutcome::Saved;
        }
        if let Some(why) = self.change_since_load() {
            log::info!("Not saving settings yet because {why}");
            return SaveOutcome::ChangedOnDisk;
        }
        let target = crate::text_file::target(&self.path);
        match write_private_file(&target, data.as_bytes()) {
            Ok(()) => {
                self.snapshot = Some(hash);
                SaveOutcome::Saved
            }
            Err(error) => {
                log::error!(
                    "Failed to write settings to {}: {error}",
                    self.path.display()
                );
                SaveOutcome::Failed
            }
        }
    }
}

/// Write `data` atomically, readable by the owner only.
pub fn write_private_file(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // A name of this write's own (two threads saving the same file mustn't
    // share one), created new and private from the start, never readable by
    // others even for a moment.
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temp = path.with_file_name(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let written = options
        .open(&temp)
        .and_then(|mut file| file.write_all(data));
    written
        .and_then(|()| std::fs::rename(&temp, path))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&temp);
        })
}

/// Copy a settings file that doesn't parse to
/// `settings.invalid-<time>[-n].json` beside it.
fn backup_invalid_file(path: &Path, data: &[u8]) -> Option<PathBuf> {
    let directory = path.parent()?;
    let stem = path.file_stem()?.to_string_lossy();
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_else(|| "json".into());
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    for attempt in 0..100 {
        let suffix = if attempt == 0 {
            String::new()
        } else {
            format!("-{attempt}")
        };
        let backup = directory.join(format!("{stem}.invalid-{timestamp}{suffix}.{ext}"));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&backup) {
            Ok(mut file) => {
                use std::io::Write;
                return file.write_all(data).ok().map(|_| backup);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                log::error!(
                    "Failed to back up invalid settings to {}: {error}",
                    backup.display()
                );
                return None;
            }
        }
    }
    None
}

// MARK: - Catalog

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    General,
    Editor,
    Terminal,
    Appearance,
    Git,
    Automation,
    LanguageServers,
    Advanced,
}

impl Category {
    pub const ALL: [Category; 8] = [
        Category::General,
        Category::Editor,
        Category::Terminal,
        Category::Appearance,
        Category::Git,
        Category::Automation,
        Category::LanguageServers,
        Category::Advanced,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Category::General => "General",
            Category::Editor => "Editor",
            Category::Terminal => "Terminal",
            Category::Appearance => "Appearance",
            Category::Git => "Git",
            Category::Automation => "Automation",
            Category::LanguageServers => "Language Servers",
            Category::Advanced => "Advanced",
        }
    }

    /// The lucide icon name.
    pub fn icon(self) -> &'static str {
        match self {
            Category::General => "settings",
            Category::Editor => "file-code",
            Category::Terminal => "square-terminal",
            Category::Appearance => "eye",
            Category::Git => "git-branch",
            Category::Automation => "zap",
            Category::LanguageServers => "plug",
            Category::Advanced => "sliders-horizontal",
        }
    }
}

/// How a setting is edited in the Settings tab.
#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    Toggle,
    Integer {
        min: i64,
        max: i64,
        step: i64,
        zero_label: Option<&'static str>,
    },
    Decimal {
        min: f64,
        max: f64,
        step: f64,
    },
    Text {
        placeholder: &'static str,
    },
    /// A folder path, typed or chosen; empty means the placeholder's default.
    Folder {
        placeholder: &'static str,
    },
    Font,
    Choice(Vec<(&'static str, &'static str)>),
    Theme,
}

/// One user-facing setting.
#[derive(Clone, Debug)]
pub struct SettingItem {
    pub key: &'static str,
    pub title: &'static str,
    pub detail: Option<&'static str>,
    pub category: Category,
    pub section: &'static str,
    pub control: Control,
}

impl SettingItem {
    pub fn matches(&self, query: &str) -> bool {
        let q = query.to_lowercase();
        self.title.to_lowercase().contains(&q)
            || self.key.contains(&q)
            || self.section.to_lowercase().contains(&q)
            || self.detail.is_some_and(|d| d.to_lowercase().contains(&q))
    }

    pub fn is_modified(&self, settings: &Settings) -> bool {
        settings.get(self.key) != Settings::default().get(self.key)
    }

    pub fn reset(&self, settings: &mut Settings) {
        if let Some(value) = Settings::default().get(self.key) {
            settings.set(self.key, &value);
        }
    }

    /// The JSON Schema entry for the value.
    pub fn schema(&self) -> Value {
        let default = Settings::default().get(self.key).unwrap_or(Value::Null);
        let mut entry = match &self.control {
            Control::Toggle => json!({"type": "boolean"}),
            Control::Integer { min, max, .. } => {
                json!({"type": "integer", "minimum": min, "maximum": max})
            }
            Control::Decimal { min, max, .. } => {
                json!({"type": "number", "minimum": min, "maximum": max})
            }
            Control::Choice(options) => json!({
                "type": "string",
                "enum": options.iter().map(|(v, _)| *v).collect::<Vec<_>>(),
            }),
            Control::Theme => json!({"type": "string", "enum": crate::theme::available_themes()}),
            Control::Text { .. } | Control::Folder { .. } | Control::Font => {
                json!({"type": "string"})
            }
        };
        entry["default"] = default;
        entry
    }
}

/// A key's label on this platform: ⌘↩ on the Mac, Ctrl+Enter elsewhere.
const fn mac_or(mac: &'static str, other: &'static str) -> &'static str {
    if cfg!(target_os = "macos") {
        mac
    } else {
        other
    }
}

fn toggle(
    key: &'static str,
    title: &'static str,
    category: Category,
    section: &'static str,
    detail: Option<&'static str>,
) -> SettingItem {
    SettingItem {
        key,
        title,
        detail,
        category,
        section,
        control: Control::Toggle,
    }
}

#[allow(clippy::too_many_arguments)]
fn integer(
    key: &'static str,
    title: &'static str,
    category: Category,
    section: &'static str,
    range: std::ops::RangeInclusive<i64>,
    step: i64,
    zero_label: Option<&'static str>,
    detail: Option<&'static str>,
) -> SettingItem {
    SettingItem {
        key,
        title,
        detail,
        category,
        section,
        control: Control::Integer {
            min: *range.start(),
            max: *range.end(),
            step,
            zero_label,
        },
    }
}

fn choice(
    key: &'static str,
    title: &'static str,
    category: Category,
    section: &'static str,
    options: &[(&'static str, &'static str)],
    detail: Option<&'static str>,
) -> SettingItem {
    SettingItem {
        key,
        title,
        detail,
        category,
        section,
        control: Control::Choice(options.to_vec()),
    }
}

fn other(
    key: &'static str,
    title: &'static str,
    category: Category,
    section: &'static str,
    control: Control,
    detail: Option<&'static str>,
) -> SettingItem {
    SettingItem {
        key,
        title,
        detail,
        category,
        section,
        control,
    }
}

/// Every user-facing setting, in the Settings tab's order.
pub fn catalog() -> Vec<SettingItem> {
    use Category::*;
    let none = None;
    vec![
        // General
        toggle("quick_terminal_enabled", "Quick terminal", General, "Quick terminal",
            Some("A terminal that drops down from the top of the screen, over any app, on a global shortcut.")),
        other("quick_terminal_shortcut", "Quick terminal shortcut", General, "Quick terminal",
            Control::Text { placeholder: "Ctrl+`" }, Some("For example Ctrl+` or Alt+Space.")),
        toggle("restore_session", "Restore previous session", General, "Startup",
            Some("Reopen workspaces, tabs, splits and terminal folders on launch.")),
        toggle("restore_scrollback", "Restore terminal scrollback", General, "Startup",
            Some("Bring back each terminal's output when the session is restored.")),
        toggle("check_for_updates", "Check for updates on launch", General, "Startup", none),
        toggle("confirm_close_warnings", "Warn before closing running commands", General, "Window",
            Some("Ask before closing a tab or window, or quitting, while a terminal is still running a command. Unsaved files are always asked about.")),
        toggle("sidebar_tabs", "List tabs in the sidebar", General, "Sidebar",
            Some("Show the active workspace's tabs under it in the sidebar instead of in the titlebar.")),
        toggle("sidebar_show_hidden", "Show hidden files", General, "Sidebar", none),
        other("scratch_directory", "Scratch folder", General, "Workspaces",
            Control::Folder { placeholder: "~" },
            Some("Where terminals in the Scratch workspace start, and where a new window's file tree begins. Empty: your home folder.")),
        toggle("workspace_trust", "Ask before trusting folders", General, "Workspaces",
            Some("Language servers, formatters on save and background fetch can run a project's own code, so they only run in folders you trust. Off: every folder is trusted.")),
        // Editor
        other("font_family", "Font family", Editor, "Font", Control::Font, none),
        integer("font_size", "Font size", Editor, "Font", 6..=72, 1, None, none),
        toggle("font_ligatures", "Font ligatures", Editor, "Font", none),
        integer("editor_line_height", "Line height", Editor, "Font", 0..=100, 1, Some("Default"),
            Some("In points; 0 uses the font's own.")),
        integer("tab_width", "Tab width", Editor, "Indentation", 1..=16, 1, None, none),
        toggle("use_spaces", "Insert spaces instead of tabs", Editor, "Indentation", none),
        toggle("indent_guides", "Indent guides", Editor, "Indentation", none),
        toggle("show_line_numbers", "Line numbers", Editor, "Display", none),
        toggle("word_wrap", "Word wrap", Editor, "Display", none),
        toggle("minimap_enabled", "Minimap", Editor, "Display", none),
        toggle("highlight_current_line", "Highlight current line", Editor, "Display", none),
        toggle("bracket_pair_colorization", "Bracket pair colors", Editor, "Display", none),
        choice("render_whitespace", "Render whitespace", Editor, "Display",
            &[("none", "None"), ("boundary", "Boundary"), ("selection", "Selection"),
              ("trailing", "Trailing"), ("all", "All")], none),
        toggle("show_right_margin", "Right margin", Editor, "Display", none),
        integer("right_margin_position", "Right margin column", Editor, "Display", 1..=500, 1, None, none),
        toggle("sticky_scroll", "Sticky scroll", Editor, "Scrolling",
            Some("Keep the enclosing scopes pinned at the top while scrolling.")),
        toggle("scroll_beyond_last_line", "Scroll beyond last line", Editor, "Scrolling", none),
        toggle("smooth_scrolling", "Smooth scrolling", Editor, "Scrolling", none),
        integer("editor_cursor_surrounding_lines", "Lines kept around the cursor", Editor, "Scrolling",
            0..=20, 1, None, none),
        choice("editor_cursor_style", "Cursor style", Editor, "Cursor",
            &[("line", "Line"), ("block", "Block"), ("underline", "Underline"), ("line-thin", "Thin line"),
              ("block-outline", "Block outline"), ("underline-thin", "Thin underline")], none),
        choice("editor_cursor_blinking", "Cursor blinking", Editor, "Cursor",
            &[("blink", "Blink"), ("smooth", "Smooth"), ("phase", "Phase"), ("expand", "Expand"),
              ("solid", "Solid")], none),
        choice("editor_auto_closing_brackets", "Auto-close brackets", Editor, "Behavior",
            &[("always", "Always"), ("languageDefined", "Language defined"),
              ("beforeWhitespace", "Before whitespace"), ("never", "Never")], none),
        toggle("folding", "Code folding", Editor, "Behavior", none),
        toggle("auto_save", "Save when focus leaves the editor", Editor, "Behavior", none),
        toggle("editor_vim_mode", "Vim keybindings", Editor, "Behavior",
            Some("Normal, insert and visual modes in the editor; the mode shows at the bottom right.")),
        toggle("editor_preview_tabs", "Preview files from the file tree", Editor, "Behavior",
            Some("A click shows a file in an italic tab the next click reuses; double-click or edit it to keep it.")),
        toggle("editor_selection_highlight", "Highlight matches of the selection", Editor, "Behavior", none),
        toggle("editor_occurrences_highlight", "Highlight occurrences of the symbol", Editor, "Behavior", none),
        choice("editor_inlay_hints", "Inlay hints", Editor, "Behavior",
            &[("on", "On"), ("off", "Off"),
              ("offUnlessPressed", mac_or("While holding ⌃⌥", "While holding Ctrl+Alt")),
              ("onUnlessPressed", mac_or("Hidden while holding ⌃⌥", "Hidden while holding Ctrl+Alt"))],
            Some("Types and parameter names from the language server, shown inline.")),
        choice("editor_word_based_suggestions", "Word-based suggestions", Editor, "Behavior",
            &[("off", "Off"), ("currentDocument", "Current file"),
              ("matchingDocuments", "Files of the same language"), ("allDocuments", "All open files")], none),
        // Terminal
        other("terminal_font_family", "Font family", Terminal, "Font", Control::Font, none),
        integer("terminal_font_size", "Font size", Terminal, "Font", 6..=72, 1, None, none),
        toggle("terminal_bold_is_bright", "Bold text uses bright colors", Terminal, "Font", none),
        other("terminal_minimum_contrast", "Minimum contrast", Terminal, "Font",
            Control::Decimal { min: 1.0, max: 21.0, step: 0.5 },
            Some("Lift text colors to at least this contrast ratio (1 turns it off).")),
        choice("terminal_cursor_shape", "Cursor shape", Terminal, "Cursor",
            &[("block", "Block"), ("underline", "Underline"), ("beam", "Beam")], none),
        toggle("terminal_cursor_blink", "Blinking cursor", Terminal, "Cursor", none),
        toggle("terminal_blocks", "Command blocks", Terminal, "Blocks & input",
            Some("Separate each command and its output, with status, actions and navigation.")),
        toggle("terminal_context_bar", "Input bar", Terminal, "Blocks & input",
            Some("Type commands in Impulse's editor below the output instead of at the shell prompt.")),
        toggle("terminal_persistent_history", "Keep command history", Terminal, "Blocks & input",
            Some("Remember commands across sessions and terminals (stored locally).")),
        toggle("terminal_shell_completions", "Ask the shell for completions", Terminal, "Blocks & input",
            Some("Add fish's own completions (its completion scripts and descriptions) to the Tab menu.")),
        toggle("terminal_editor_integration", "Use Impulse as $EDITOR", Terminal, "Blocks & input",
            Some("git commit and friends open files in an Impulse tab.")),
        toggle("agent_composer_auto_show", "Open the composer when an agent needs input", Terminal, "Agents",
            Some("In the focused terminal, so you can answer without clicking into the agent.")),
        toggle("task_overlap_notify", "Notify when tasks change the same files", Terminal, "Agents",
            Some("Once per pair of workspaces, when the main checkout or a task starts changing files another one changes.")),
        toggle("agent_hook_task_summary", "Tell agents about other tasks when they start", Terminal, "Agents",
            Some("Claude Code and opencode (with Impulse's hooks) get a short summary of the repository's other workspaces.")),
        toggle("agent_hook_shared_files", "Tell agents when they edit a file another task changes", Terminal, "Agents",
            Some("Once per file, naming the other workspace and its agent.")),
        toggle("agent_hook_merge_guard", "Ask before an agent merges a branch still being worked on", Terminal, "Agents",
            Some("When an agent runs git merge, rebase or pull on a task's branch while that task's agent is working or it has uncommitted files. Claude Code and opencode ask you, with the reason, before running it.")),
        toggle("agent_hook_dependencies", "Tell agents when dependency files changed", Terminal, "Agents",
            Some("After an agent's git pull, merge or checkout brings in a changed lock file or Dockerfile.")),
        choice("agent_task_creation", "When an agent asks to start a task", Terminal, "Agents",
            &[("ask", "Ask me"), ("trusted_projects", "Start it in trusted projects"), ("off", "Don't allow it")],
            Some("From impulse tasks new or the MCP server's create_task. Asking shows New Task filled in. Trusted projects: a trusted repository whose project settings you trusted, otherwise ask; moving your changes, and a task asked for from inside an agent's task, always ask.")),
        integer("agent_task_limit", "Open tasks agents may start", Terminal, "Agents", 0..=50, 1,
            Some("No limit"), Some("Per repository. Archive or finish one to make room.")),
        choice("agent_tool_terminal_output", "Agents may read terminal output", Terminal, "Agents",
            &[("workspace", "In their own workspace"), ("window", "In the whole window"), ("off", "Never")],
            Some("The commands and output of other terminals, through the MCP server's list_command_blocks and get_block_output (and impulse blocks). Output can hold secrets.")),
        toggle("terminal_copy_on_select", "Copy on select", Terminal, "Behavior", none),
        toggle("terminal_scroll_on_output", "Scroll to bottom on output", Terminal, "Behavior", none),
        toggle("terminal_allow_hyperlink", "Clickable links", Terminal, "Behavior", none),
        choice("terminal_option_as_meta", mac_or("Option as Meta", "Alt as Meta"), Terminal, "Behavior",
            &[("off", "Off"), ("left", mac_or("Left Option", "Left Alt")),
              ("right", mac_or("Right Option", "Right Alt")), ("both", mac_or("Both Option keys", "Both Alt keys"))],
            Some(mac_or(
                "The Option key sends Esc + the key (Meta-f, Meta-b in the shell) instead of typing ƒ, or @ on some layouts.",
                "The Alt key sends Esc + the key (Meta-f, Meta-b in the shell) instead of typing what the layout gives it."))),
        toggle("terminal_allow_osc52_write", "Programs may set the clipboard", Terminal, "Clipboard",
            Some("OSC 52 writes, as used by tmux, vim and ssh sessions.")),
        toggle("terminal_allow_osc52_read", "Programs may read the clipboard", Terminal, "Clipboard", none),
        toggle("terminal_bell", "Audible bell", Terminal, "Bell & notifications", none),
        toggle("terminal_attention_on_bell", "Request attention on bell", Terminal, "Bell & notifications", none),
        toggle("terminal_allow_notifications", "Allow terminal notifications", Terminal, "Bell & notifications",
            Some("OSC 9 / 99 / 777 notifications from programs and agents.")),
        toggle("notification_sound", "Notification sound", Terminal, "Bell & notifications",
            Some("Play Impulse's chime with desktop notifications.")),
        toggle("terminal_attention_on_long_command", "Notify when long commands finish", Terminal,
            "Bell & notifications", none),
        integer("terminal_long_command_seconds", "Long command threshold", Terminal, "Bell & notifications",
            1..=86_400, 1, None, Some("Seconds.")),
        integer("terminal_scrollback", "Scrollback lines", Terminal, "Scrollback", 100..=1_000_000, 1000, None, none),
        // Appearance
        other("color_scheme", "Theme", Appearance, "Theme", Control::Theme,
            Some("Colors for the window, editor and terminal.")),
        // Git
        toggle("git_commit_and_push", "Commit and push", Git, "Commit",
            Some(mac_or("The commit button and ⌘↩ push right after committing; ⇧⌘↩ only commits.",
                "The commit button and Ctrl+Enter push right after committing; Ctrl+Shift+Enter only commits."))),
        choice("git_pull_mode", "When pulling", Git, "Remote",
            &[("ff-only", "Fast-forward only"), ("rebase", "Rebase local commits"), ("merge", "Merge")],
            Some("Fast-forward only never makes a merge commit; it stops when the branch has diverged.")),
        toggle("git_push_follow_tags", "Push annotated tags with commits", Git, "Remote",
            Some("Push uses --follow-tags: annotated tags on the commits being pushed go along.")),
        integer("git_auto_fetch_minutes", "Fetch in the background", Git, "Remote", 0..=120, 5, Some("Off"),
            Some("Minutes between fetches of the repositories open in windows, so ahead/behind stays current. Never asks for credentials.")),
        toggle("git_push_tags_on_create", "Push new tags to the remote", Git, "Tags",
            Some("Creating a tag pushes it right away. The Create Tag sheet's checkbox starts from this.")),
        integer("review_context_lines", "Context lines", Git, "Review", 0..=100, 1, None,
            Some("Unchanged lines shown around each change in Review and History. The review header's context menu shows more for one review.")),
    ]
}

/// A JSON Schema (draft-07) for settings.json: every catalog setting plus
/// the list- and map-valued ones edited in the file itself.
pub fn json_schema() -> Value {
    let mut properties = Map::new();
    for item in catalog() {
        let mut entry = item.schema();
        let description: Vec<&str> = [Some(item.title), item.detail]
            .into_iter()
            .flatten()
            .collect();
        entry["description"] = json!(description.join(". "));
        properties.insert(item.key.into(), entry);
    }
    let string_array = json!({"type": "array", "items": {"type": "string"}});
    properties.insert("keybinding_overrides".into(), json!({
        "type": "object",
        "description": "Shortcut per command id, e.g. \"split_right\": \"Cmd+D\" (\"none\" removes it).",
        "additionalProperties": {"type": "string"},
    }));
    properties.insert("custom_keybindings".into(), json!({
        "type": "array",
        "description": "Shortcuts that run a shell command line in the focused terminal (or a new terminal tab when none is at its prompt).",
        "items": {
            "type": "object", "required": ["name", "key", "command"],
            "properties": {
                "name": {"type": "string"}, "key": {"type": "string"},
                "command": {"type": "string", "description": "The command line, as you'd type it in the shell."},
                "args": {"type": "array", "items": {"type": "string"},
                         "description": "Optional arguments added after the command, each quoted."},
            },
        },
    }));
    properties.insert(
        "commands_on_save".into(),
        json!({
            "type": "array", "description": "Commands run after saving matching files.",
            "items": {
                "type": "object", "required": ["name", "command", "file_pattern"],
                "properties": {
                    "name": {"type": "string"}, "command": {"type": "string"}, "args": string_array,
                    "file_pattern": {"type": "string"}, "reload_file": {"type": "boolean"},
                },
            },
        }),
    );
    properties.insert("file_type_overrides".into(), json!({
        "type": "array", "description": "Indentation and formatting per file pattern.",
        "items": {
            "type": "object", "required": ["pattern"],
            "properties": {
                "pattern": {"type": "string"}, "tab_width": {"type": "integer", "minimum": 1},
                "use_spaces": {"type": "boolean"},
                "format_on_save": {"type": "object",
                                   "properties": {"command": {"type": "string"}, "args": string_array}},
            },
        },
    }));
    // Window state Impulse writes itself.
    for key in ["window_width", "window_height", "sidebar_width"] {
        properties.insert(key.into(), json!({"type": "integer"}));
    }
    properties.insert(
        "sidebar_visible".into(),
        json!({"type": "boolean", "default": Settings::default().sidebar_visible}),
    );
    properties.insert("last_directory".into(), json!({"type": "string"}));
    properties.insert("open_files".into(), string_array);
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "Impulse settings",
        "type": "object",
        "properties": properties,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip() {
        let defaults = Settings::default();
        assert_eq!(Settings::from_json(&defaults.to_json()), defaults);
        assert_eq!(
            Settings::KEYS.len(),
            defaults.to_json().as_object().unwrap().len()
        );
        assert_eq!(defaults.get("workspace_trust"), Some(json!(true)));
        assert_eq!(defaults.color_scheme, "nord");
    }

    #[test]
    fn the_notification_sound_is_on_unless_turned_off() {
        assert!(Settings::default().notification_sound);
        assert!(Settings::from_json(&json!({})).notification_sound);
        assert!(
            Settings::from_json(&json!({"notification_sound": "no"})).notification_sound,
            "a mistyped value takes its default"
        );
        let off = Settings::from_json(&json!({"notification_sound": false}));
        assert!(!off.notification_sound);
        assert_eq!(off.to_json()["notification_sound"], json!(false));
        let schema = json_schema();
        assert_eq!(
            schema["properties"]["notification_sound"]["type"],
            json!("boolean")
        );
        assert_eq!(
            schema["properties"]["notification_sound"]["default"],
            json!(true)
        );
    }

    #[test]
    fn decoding_is_per_key() {
        let settings = Settings::from_json(&json!({
            "font_size": "big",
            "tab_width": 2,
            "terminal_font_size": 16.0,
            "color_scheme": "dracula",
            "unknown_key": 1,
            "custom_keybindings": [{"key": "Cmd+K", "command": "ls"}],
            "commands_on_save": [{"name": "x", "command": "y", "file_pattern": "*.rs"}],
            "file_type_overrides": [{"pattern": "*.go", "use_spaces": false}],
        }));
        assert_eq!(settings.font_size, 14, "a mistyped value takes its default");
        assert_eq!(settings.tab_width, 2);
        assert_eq!(settings.terminal_font_size, 16);
        assert_eq!(settings.color_scheme, "dracula");
        assert_eq!(settings.custom_keybindings[0].args, Vec::<String>::new());
        assert!(
            settings.commands_on_save.is_empty(),
            "every key of a command on save is required, as in the Swift app"
        );
        assert_eq!(settings.file_type_overrides[0].use_spaces, Some(false));
        let encoded = settings.to_json();
        assert_eq!(
            encoded["file_type_overrides"][0],
            json!({"pattern": "*.go", "use_spaces": false}),
            "unset overrides are left out"
        );
    }

    #[test]
    fn validation_clamps_and_normalizes() {
        let mut settings = Settings::from_json(&json!({
            "font_size": 400, "terminal_scrollback": 5, "terminal_minimum_contrast": 50.0,
            "sidebar_width": 20, "terminal_cursor_shape": "Bar", "terminal_option_as_meta": "LEFT",
            "agent_task_creation": "sometimes", "agent_tool_terminal_output": "all",
            "window_width": 100, "window_height": 99999,
        }));
        settings.validate();
        assert_eq!(settings.font_size, 72);
        assert_eq!(settings.terminal_scrollback, 100);
        assert_eq!(settings.terminal_minimum_contrast, 21.0);
        assert_eq!(settings.sidebar_width, 100);
        assert_eq!(settings.terminal_cursor_shape, "beam");
        assert_eq!(settings.terminal_option_as_meta, "left");
        assert_eq!(settings.agent_task_creation, "ask");
        assert_eq!(settings.agent_tool_terminal_output, "workspace");
        assert_eq!(
            (settings.window_width, settings.window_height),
            (400, 10000)
        );
    }

    #[test]
    fn fonts_move_off_old_defaults() {
        let mut settings = Settings {
            font_family: "SF Mono".into(),
            terminal_font_family: "Menlo".into(),
            ..Settings::default()
        };
        assert!(settings.migrate_default_font());
        assert_eq!(settings.font_family, "JetBrains Mono");
        assert_eq!(settings.terminal_font_family, "Menlo");
        assert!(!settings.migrate_default_font());
    }

    #[test]
    fn merge_keeps_both_sides() {
        let base = Settings::default();
        let ours = Settings {
            font_size: 18,
            ..base.clone()
        };
        let theirs = Settings {
            tab_width: 2,
            font_size: 12,
            ..base.clone()
        };
        let merged = Settings::merged(&ours, &theirs, &base);
        assert_eq!((merged.font_size, merged.tab_width), (18, 2));
        assert_eq!(Settings::merged(&base, &theirs, &base), theirs);
    }

    #[test]
    fn indentation_per_file() {
        let settings = Settings::from_json(&json!({
            "file_type_overrides": [
                {"pattern": "*.go", "use_spaces": false},
                {"pattern": "*.go", "tab_width": 8},
            ],
        }));
        assert_eq!(settings.indentation("/x/main.go"), (8, false));
        assert_eq!(settings.indentation("/x/main.rs"), (4, true));
    }

    #[test]
    fn catalog_keys_exist_and_schema_has_defaults() {
        let defaults = Settings::default();
        for item in catalog() {
            assert!(
                defaults.get(item.key).is_some(),
                "{} isn't a setting",
                item.key
            );
            assert!(item.schema().get("default").is_some());
        }
        let schema = json_schema();
        assert_eq!(schema["properties"]["font_size"]["maximum"], json!(72));
        assert_eq!(schema["properties"]["font_size"]["default"], json!(14));
        assert!(schema["properties"]["color_scheme"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("nord")));
    }

    #[test]
    fn file_load_and_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut file = SettingsFile::new(path.clone());
        let settings = file.load(false).unwrap();
        assert_eq!(settings, Settings::default(), "no file: defaults");
        assert_eq!(file.save(&settings), SaveOutcome::Saved);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(!file.changed_on_disk());

        // Changed outside Impulse: saving doesn't overwrite it.
        std::fs::write(&path, r#"{"font_size": 20}"#).unwrap();
        assert!(file.changed_on_disk());
        let ours = Settings {
            tab_width: 3,
            ..settings.clone()
        };
        assert_eq!(file.save(&ours), SaveOutcome::ChangedOnDisk);
        let theirs = file.load(true).unwrap();
        assert_eq!(theirs.font_size, 20);
        let merged = Settings::merged(&ours, &theirs, &settings);
        assert_eq!(file.save(&merged), SaveOutcome::Saved);
        let reread = SettingsFile::new(path.clone()).load(false).unwrap();
        assert_eq!((reread.font_size, reread.tab_width), (20, 3));
    }

    #[test]
    fn a_broken_file_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();
        let mut file = SettingsFile::new(path.clone());
        assert_eq!(file.load(false), Some(Settings::default()));
        let warning = file.warning.clone().unwrap();
        assert!(warning.using_defaults);
        let backup = warning.backup_path.unwrap();
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "{ not json");
        assert_eq!(file.save(&Settings::default()), SaveOutcome::Failed);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");

        // Fixed while running: loads, and saves again.
        std::fs::write(&path, r#"{"tab_width": 2}"#).unwrap();
        assert_eq!(file.load(true).unwrap().tab_width, 2);
        assert!(file.warning.is_none());
        assert_eq!(file.save(&Settings::default()), SaveOutcome::Saved);

        // Broken while running: the caller keeps what it has.
        std::fs::write(&path, "[").unwrap();
        assert_eq!(file.load(true), None);
        assert!(!file.warning.as_ref().unwrap().using_defaults);
    }
}
