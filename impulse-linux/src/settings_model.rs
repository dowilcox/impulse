// SPDX-License-Identifier: GPL-3.0-only
//
// Settings bridge QObject for QML. Exposes every field from
// impulse_core::settings::Settings as QObject properties with load/save
// operations to ~/.config/impulse/settings.json.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        // Window
        #[qproperty(i32, window_width)]
        #[qproperty(i32, window_height)]
        #[qproperty(bool, sidebar_visible)]
        #[qproperty(i32, sidebar_width)]
        #[qproperty(QString, last_directory)]
        #[qproperty(QString, open_files_json)]
        // Editor
        #[qproperty(bool, auto_save)]
        #[qproperty(i32, font_size)]
        #[qproperty(QString, font_family)]
        #[qproperty(i32, tab_width)]
        #[qproperty(bool, use_spaces)]
        #[qproperty(bool, show_line_numbers)]
        #[qproperty(bool, show_right_margin)]
        #[qproperty(i32, right_margin_position)]
        #[qproperty(bool, word_wrap)]
        #[qproperty(bool, highlight_current_line)]
        #[qproperty(bool, minimap_enabled)]
        #[qproperty(QString, render_whitespace)]
        #[qproperty(bool, sticky_scroll)]
        #[qproperty(bool, bracket_pair_colorization)]
        #[qproperty(bool, indent_guides)]
        #[qproperty(bool, font_ligatures)]
        #[qproperty(bool, folding)]
        #[qproperty(bool, scroll_beyond_last_line)]
        #[qproperty(bool, smooth_scrolling)]
        #[qproperty(QString, editor_cursor_style)]
        #[qproperty(QString, editor_cursor_blinking)]
        // Terminal
        #[qproperty(i32, terminal_scrollback)]
        #[qproperty(QString, terminal_cursor_shape)]
        #[qproperty(bool, terminal_cursor_blink)]
        #[qproperty(bool, terminal_bell)]
        #[qproperty(QString, terminal_font_family)]
        #[qproperty(i32, terminal_font_size)]
        #[qproperty(bool, terminal_copy_on_select)]
        #[qproperty(bool, terminal_scroll_on_output)]
        #[qproperty(bool, terminal_allow_hyperlink)]
        #[qproperty(bool, terminal_bold_is_bright)]
        // Editor (additional)
        #[qproperty(i32, editor_line_height)]
        #[qproperty(QString, editor_auto_closing_brackets)]
        #[qproperty(i32, editor_cursor_surrounding_lines)]
        #[qproperty(bool, editor_selection_highlight)]
        #[qproperty(bool, editor_occurrences_highlight)]
        #[qproperty(QString, editor_word_based_suggestions)]
        // Sidebar
        #[qproperty(bool, sidebar_show_hidden)]
        // Appearance
        #[qproperty(QString, color_scheme)]
        // Updates
        #[qproperty(bool, check_for_updates)]
        // Complex fields as JSON
        #[qproperty(QString, keybinding_overrides_json)]
        #[qproperty(QString, file_type_overrides_json)]
        #[qproperty(QString, commands_on_save_json)]
        #[qproperty(QString, custom_keybindings_json)]
        type SettingsModel = super::SettingsModelRust;

        #[qinvokable]
        fn load(self: Pin<&mut SettingsModel>);

        #[qinvokable]
        fn save(self: Pin<&mut SettingsModel>);

        #[qinvokable]
        fn reset_to_defaults(self: Pin<&mut SettingsModel>);

        #[qinvokable]
        fn set_setting(self: Pin<&mut SettingsModel>, key: &QString, value: &QString);

        #[qinvokable]
        fn add_file_type_override(self: Pin<&mut SettingsModel>, json: &QString);

        #[qinvokable]
        fn remove_file_type_override(self: Pin<&mut SettingsModel>, index: i32);

        #[qinvokable]
        fn add_command_on_save(self: Pin<&mut SettingsModel>, json: &QString);

        #[qinvokable]
        fn remove_command_on_save(self: Pin<&mut SettingsModel>, index: i32);

        #[qsignal]
        fn settings_changed(self: Pin<&mut SettingsModel>);
    }
}

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use std::pin::Pin;

/// Return the platform settings file path.
fn settings_path() -> std::path::PathBuf {
    let config_dir = dirs::config_dir().unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
            .join(".config")
    });
    config_dir.join("impulse").join("settings.json")
}

pub struct SettingsModelRust {
    // Window
    window_width: i32,
    window_height: i32,
    sidebar_visible: bool,
    sidebar_width: i32,
    last_directory: QString,
    open_files_json: QString,
    // Editor
    auto_save: bool,
    font_size: i32,
    font_family: QString,
    tab_width: i32,
    use_spaces: bool,
    show_line_numbers: bool,
    show_right_margin: bool,
    right_margin_position: i32,
    word_wrap: bool,
    highlight_current_line: bool,
    minimap_enabled: bool,
    render_whitespace: QString,
    sticky_scroll: bool,
    bracket_pair_colorization: bool,
    indent_guides: bool,
    font_ligatures: bool,
    folding: bool,
    scroll_beyond_last_line: bool,
    smooth_scrolling: bool,
    editor_cursor_style: QString,
    editor_cursor_blinking: QString,
    // Terminal
    terminal_scrollback: i32,
    terminal_cursor_shape: QString,
    terminal_cursor_blink: bool,
    terminal_bell: bool,
    terminal_font_family: QString,
    terminal_font_size: i32,
    terminal_copy_on_select: bool,
    terminal_scroll_on_output: bool,
    terminal_allow_hyperlink: bool,
    terminal_bold_is_bright: bool,
    // Editor (additional)
    editor_line_height: i32,
    editor_auto_closing_brackets: QString,
    editor_cursor_surrounding_lines: i32,
    editor_selection_highlight: bool,
    editor_occurrences_highlight: bool,
    editor_word_based_suggestions: QString,
    // Sidebar
    sidebar_show_hidden: bool,
    // Appearance
    color_scheme: QString,
    // Updates
    check_for_updates: bool,
    // Complex fields as JSON
    keybinding_overrides_json: QString,
    file_type_overrides_json: QString,
    commands_on_save_json: QString,
    custom_keybindings_json: QString,
    // Internal: the actual Settings struct
    inner: impulse_core::settings::Settings,
}

impl Default for SettingsModelRust {
    fn default() -> Self {
        let s = impulse_core::settings::Settings::default();
        Self::from_settings(s)
    }
}

impl SettingsModelRust {
    fn from_settings(s: impulse_core::settings::Settings) -> Self {
        let open_files_json =
            serde_json::to_string(&s.open_files).unwrap_or_else(|_| "[]".to_string());
        let keybinding_overrides_json =
            serde_json::to_string(&s.keybinding_overrides).unwrap_or_else(|_| "{}".to_string());
        let file_type_overrides_json =
            serde_json::to_string(&s.file_type_overrides).unwrap_or_else(|_| "[]".to_string());
        let commands_on_save_json =
            serde_json::to_string(&s.commands_on_save).unwrap_or_else(|_| "[]".to_string());
        let custom_keybindings_json =
            serde_json::to_string(&s.custom_keybindings).unwrap_or_else(|_| "[]".to_string());

        Self {
            window_width: s.window_width,
            window_height: s.window_height,
            sidebar_visible: s.sidebar_visible,
            sidebar_width: s.sidebar_width,
            last_directory: QString::from(s.last_directory.as_str()),
            open_files_json: QString::from(open_files_json.as_str()),
            auto_save: s.auto_save,
            font_size: s.font_size,
            font_family: QString::from(s.font_family.as_str()),
            tab_width: s.tab_width as i32,
            use_spaces: s.use_spaces,
            show_line_numbers: s.show_line_numbers,
            show_right_margin: s.show_right_margin,
            right_margin_position: s.right_margin_position as i32,
            word_wrap: s.word_wrap,
            highlight_current_line: s.highlight_current_line,
            minimap_enabled: s.minimap_enabled,
            render_whitespace: QString::from(s.render_whitespace.as_str()),
            sticky_scroll: s.sticky_scroll,
            bracket_pair_colorization: s.bracket_pair_colorization,
            indent_guides: s.indent_guides,
            font_ligatures: s.font_ligatures,
            folding: s.folding,
            scroll_beyond_last_line: s.scroll_beyond_last_line,
            smooth_scrolling: s.smooth_scrolling,
            editor_cursor_style: QString::from(s.editor_cursor_style.as_str()),
            editor_cursor_blinking: QString::from(s.editor_cursor_blinking.as_str()),
            terminal_scrollback: s.terminal_scrollback as i32,
            terminal_cursor_shape: QString::from(s.terminal_cursor_shape.as_str()),
            terminal_cursor_blink: s.terminal_cursor_blink,
            terminal_bell: s.terminal_bell,
            terminal_font_family: QString::from(s.terminal_font_family.as_str()),
            terminal_font_size: s.terminal_font_size,
            terminal_copy_on_select: s.terminal_copy_on_select,
            terminal_scroll_on_output: s.terminal_scroll_on_output,
            terminal_allow_hyperlink: s.terminal_allow_hyperlink,
            terminal_bold_is_bright: s.terminal_bold_is_bright,
            editor_line_height: s.editor_line_height as i32,
            editor_auto_closing_brackets: QString::from(s.editor_auto_closing_brackets.as_str()),
            editor_cursor_surrounding_lines: s.editor_cursor_surrounding_lines as i32,
            editor_selection_highlight: s.editor_selection_highlight,
            editor_occurrences_highlight: s.editor_occurrences_highlight,
            editor_word_based_suggestions: QString::from(s.editor_word_based_suggestions.as_str()),
            sidebar_show_hidden: s.sidebar_show_hidden,
            color_scheme: QString::from(s.color_scheme.as_str()),
            check_for_updates: s.check_for_updates,
            keybinding_overrides_json: QString::from(keybinding_overrides_json.as_str()),
            file_type_overrides_json: QString::from(file_type_overrides_json.as_str()),
            commands_on_save_json: QString::from(commands_on_save_json.as_str()),
            custom_keybindings_json: QString::from(custom_keybindings_json.as_str()),
            inner: s,
        }
    }

    /// Sync all QObject properties back to the inner Settings struct.
    fn sync_to_inner(&mut self) {
        self.inner.window_width = self.window_width;
        self.inner.window_height = self.window_height;
        self.inner.sidebar_visible = self.sidebar_visible;
        self.inner.sidebar_width = self.sidebar_width;
        self.inner.last_directory = self.last_directory.to_string();
        self.inner.auto_save = self.auto_save;
        self.inner.font_size = self.font_size;
        self.inner.font_family = self.font_family.to_string();
        self.inner.tab_width = self.tab_width as u32;
        self.inner.use_spaces = self.use_spaces;
        self.inner.show_line_numbers = self.show_line_numbers;
        self.inner.show_right_margin = self.show_right_margin;
        self.inner.right_margin_position = self.right_margin_position as u32;
        self.inner.word_wrap = self.word_wrap;
        self.inner.highlight_current_line = self.highlight_current_line;
        self.inner.minimap_enabled = self.minimap_enabled;
        self.inner.render_whitespace = self.render_whitespace.to_string();
        self.inner.sticky_scroll = self.sticky_scroll;
        self.inner.bracket_pair_colorization = self.bracket_pair_colorization;
        self.inner.indent_guides = self.indent_guides;
        self.inner.font_ligatures = self.font_ligatures;
        self.inner.folding = self.folding;
        self.inner.scroll_beyond_last_line = self.scroll_beyond_last_line;
        self.inner.smooth_scrolling = self.smooth_scrolling;
        self.inner.editor_cursor_style = self.editor_cursor_style.to_string();
        self.inner.editor_cursor_blinking = self.editor_cursor_blinking.to_string();
        self.inner.terminal_scrollback = self.terminal_scrollback as i64;
        self.inner.terminal_cursor_shape = self.terminal_cursor_shape.to_string();
        self.inner.terminal_cursor_blink = self.terminal_cursor_blink;
        self.inner.terminal_bell = self.terminal_bell;
        self.inner.terminal_font_family = self.terminal_font_family.to_string();
        self.inner.terminal_font_size = self.terminal_font_size;
        self.inner.terminal_copy_on_select = self.terminal_copy_on_select;
        self.inner.terminal_scroll_on_output = self.terminal_scroll_on_output;
        self.inner.terminal_allow_hyperlink = self.terminal_allow_hyperlink;
        self.inner.terminal_bold_is_bright = self.terminal_bold_is_bright;
        self.inner.editor_line_height = self.editor_line_height as u32;
        self.inner.editor_auto_closing_brackets = self.editor_auto_closing_brackets.to_string();
        self.inner.editor_cursor_surrounding_lines = self.editor_cursor_surrounding_lines as u32;
        self.inner.editor_selection_highlight = self.editor_selection_highlight;
        self.inner.editor_occurrences_highlight = self.editor_occurrences_highlight;
        self.inner.editor_word_based_suggestions = self.editor_word_based_suggestions.to_string();
        self.inner.sidebar_show_hidden = self.sidebar_show_hidden;
        self.inner.color_scheme = self.color_scheme.to_string();
        self.inner.check_for_updates = self.check_for_updates;

        // Deserialize complex JSON fields back into the inner struct
        let open_files_str = self.open_files_json.to_string();
        if let Ok(files) = serde_json::from_str::<Vec<String>>(&open_files_str) {
            self.inner.open_files = files;
        }

        let overrides_str = self.keybinding_overrides_json.to_string();
        if let Ok(overrides) =
            serde_json::from_str::<std::collections::HashMap<String, String>>(&overrides_str)
        {
            self.inner.keybinding_overrides = overrides;
        }

        let ft_overrides_str = self.file_type_overrides_json.to_string();
        if let Ok(overrides) =
            serde_json::from_str::<Vec<impulse_core::settings::FileTypeOverride>>(&ft_overrides_str)
        {
            self.inner.file_type_overrides = overrides;
        }

        let commands_str = self.commands_on_save_json.to_string();
        if let Ok(commands) =
            serde_json::from_str::<Vec<impulse_core::settings::CommandOnSave>>(&commands_str)
        {
            self.inner.commands_on_save = commands;
        }

        let keybindings_str = self.custom_keybindings_json.to_string();
        if let Ok(keybindings) =
            serde_json::from_str::<Vec<impulse_core::settings::CustomKeybinding>>(&keybindings_str)
        {
            self.inner.custom_keybindings = keybindings;
        }
    }
}

impl qobject::SettingsModel {
    pub fn load(mut self: Pin<&mut Self>) {
        let path = settings_path();

        let settings = if path.is_file() {
            match std::fs::read_to_string(&path) {
                Ok(json) => match impulse_core::settings::Settings::from_json(&json) {
                    Ok(s) => s,
                    Err(e) => {
                        log::warn!("Failed to parse settings from {:?}: {}", path, e);
                        impulse_core::settings::Settings::default()
                    }
                },
                Err(e) => {
                    log::warn!("Failed to read settings from {:?}: {}", path, e);
                    impulse_core::settings::Settings::default()
                }
            }
        } else {
            impulse_core::settings::Settings::default()
        };

        self.as_mut().apply_settings(settings);
        self.as_mut().settings_changed();
    }

    pub fn save(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().sync_to_inner();

        let path = settings_path();
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log::warn!("Failed to create settings directory: {}", e);
                return;
            }
        }

        match self.as_ref().rust().inner.to_json() {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, &json) {
                    log::warn!("Failed to write settings to {:?}: {}", path, e);
                }
            }
            Err(e) => {
                log::warn!("Failed to serialize settings: {}", e);
            }
        }

        self.as_mut().settings_changed();
    }

    pub fn reset_to_defaults(mut self: Pin<&mut Self>) {
        let defaults = impulse_core::settings::Settings::default();
        self.as_mut().apply_settings(defaults);
        self.as_mut().settings_changed();
    }

    pub fn set_setting(mut self: Pin<&mut Self>, key: &QString, value: &QString) {
        let key_str = key.to_string();
        let val_str = value.to_string();

        match key_str.as_str() {
            // Window
            "window_width" => {
                if let Ok(v) = val_str.parse::<i32>() {
                    self.as_mut().set_window_width(v);
                }
            }
            "window_height" => {
                if let Ok(v) = val_str.parse::<i32>() {
                    self.as_mut().set_window_height(v);
                }
            }
            "sidebar_visible" => {
                self.as_mut().set_sidebar_visible(val_str == "true");
            }
            "sidebar_width" => {
                if let Ok(v) = val_str.parse::<i32>() {
                    self.as_mut().set_sidebar_width(v);
                }
            }
            "last_directory" => {
                self.as_mut()
                    .set_last_directory(QString::from(val_str.as_str()));
            }
            // Editor
            "auto_save" => {
                self.as_mut().set_auto_save(val_str == "true");
            }
            "font_size" => {
                if let Ok(v) = val_str.parse::<i32>() {
                    self.as_mut().set_font_size(v);
                }
            }
            "font_family" => {
                self.as_mut()
                    .set_font_family(QString::from(val_str.as_str()));
            }
            "tab_width" => {
                if let Ok(v) = val_str.parse::<i32>() {
                    self.as_mut().set_tab_width(v);
                }
            }
            "use_spaces" => {
                self.as_mut().set_use_spaces(val_str == "true");
            }
            "show_line_numbers" => {
                self.as_mut().set_show_line_numbers(val_str == "true");
            }
            "show_right_margin" => {
                self.as_mut().set_show_right_margin(val_str == "true");
            }
            "right_margin_position" => {
                if let Ok(v) = val_str.parse::<i32>() {
                    self.as_mut().set_right_margin_position(v);
                }
            }
            "word_wrap" => {
                self.as_mut().set_word_wrap(val_str == "true");
            }
            "highlight_current_line" => {
                self.as_mut().set_highlight_current_line(val_str == "true");
            }
            "minimap_enabled" => {
                self.as_mut().set_minimap_enabled(val_str == "true");
            }
            "render_whitespace" => {
                self.as_mut()
                    .set_render_whitespace(QString::from(val_str.as_str()));
            }
            "sticky_scroll" => {
                self.as_mut().set_sticky_scroll(val_str == "true");
            }
            "bracket_pair_colorization" => {
                self.as_mut()
                    .set_bracket_pair_colorization(val_str == "true");
            }
            "indent_guides" => {
                self.as_mut().set_indent_guides(val_str == "true");
            }
            "font_ligatures" => {
                self.as_mut().set_font_ligatures(val_str == "true");
            }
            "folding" => {
                self.as_mut().set_folding(val_str == "true");
            }
            "scroll_beyond_last_line" => {
                self.as_mut()
                    .set_scroll_beyond_last_line(val_str == "true");
            }
            "smooth_scrolling" => {
                self.as_mut().set_smooth_scrolling(val_str == "true");
            }
            "editor_cursor_style" => {
                self.as_mut()
                    .set_editor_cursor_style(QString::from(val_str.as_str()));
            }
            "editor_cursor_blinking" => {
                self.as_mut()
                    .set_editor_cursor_blinking(QString::from(val_str.as_str()));
            }
            // Terminal
            "terminal_scrollback" => {
                if let Ok(v) = val_str.parse::<i32>() {
                    self.as_mut().set_terminal_scrollback(v);
                }
            }
            "terminal_cursor_shape" => {
                self.as_mut()
                    .set_terminal_cursor_shape(QString::from(val_str.as_str()));
            }
            "terminal_cursor_blink" => {
                self.as_mut().set_terminal_cursor_blink(val_str == "true");
            }
            "terminal_bell" => {
                self.as_mut().set_terminal_bell(val_str == "true");
            }
            "terminal_font_family" => {
                self.as_mut()
                    .set_terminal_font_family(QString::from(val_str.as_str()));
            }
            "terminal_font_size" => {
                if let Ok(v) = val_str.parse::<i32>() {
                    self.as_mut().set_terminal_font_size(v);
                }
            }
            "terminal_copy_on_select" => {
                self.as_mut()
                    .set_terminal_copy_on_select(val_str == "true");
            }
            "terminal_scroll_on_output" => {
                self.as_mut()
                    .set_terminal_scroll_on_output(val_str == "true");
            }
            "terminal_allow_hyperlink" => {
                self.as_mut()
                    .set_terminal_allow_hyperlink(val_str == "true");
            }
            "terminal_bold_is_bright" => {
                self.as_mut()
                    .set_terminal_bold_is_bright(val_str == "true");
            }
            // Editor (additional)
            "editor_line_height" => {
                if let Ok(v) = val_str.parse::<i32>() {
                    self.as_mut().set_editor_line_height(v);
                }
            }
            "editor_auto_closing_brackets" => {
                self.as_mut()
                    .set_editor_auto_closing_brackets(QString::from(val_str.as_str()));
            }
            "editor_cursor_surrounding_lines" => {
                if let Ok(v) = val_str.parse::<i32>() {
                    self.as_mut().set_editor_cursor_surrounding_lines(v);
                }
            }
            "editor_selection_highlight" => {
                self.as_mut()
                    .set_editor_selection_highlight(val_str == "true");
            }
            "editor_occurrences_highlight" => {
                self.as_mut()
                    .set_editor_occurrences_highlight(val_str == "true");
            }
            "editor_word_based_suggestions" => {
                self.as_mut()
                    .set_editor_word_based_suggestions(QString::from(val_str.as_str()));
            }
            // Sidebar
            "sidebar_show_hidden" => {
                self.as_mut().set_sidebar_show_hidden(val_str == "true");
            }
            // Appearance
            "color_scheme" => {
                self.as_mut()
                    .set_color_scheme(QString::from(val_str.as_str()));
            }
            // Updates
            "check_for_updates" => {
                self.as_mut().set_check_for_updates(val_str == "true");
            }
            _ => {
                log::warn!("Unknown setting key: '{}'", key_str);
            }
        }

        self.as_mut().settings_changed();
    }

    pub fn add_file_type_override(mut self: Pin<&mut Self>, json: &QString) {
        let json_str = json.to_string();
        match serde_json::from_str::<impulse_core::settings::FileTypeOverride>(&json_str) {
            Ok(override_entry) => {
                self.as_mut().rust_mut().inner.file_type_overrides.push(override_entry);
                let new_json = serde_json::to_string(&self.as_ref().rust().inner.file_type_overrides)
                    .unwrap_or_else(|_| "[]".to_string());
                self.as_mut()
                    .set_file_type_overrides_json(QString::from(new_json.as_str()));
                self.as_mut().settings_changed();
            }
            Err(e) => {
                log::warn!("Failed to parse file type override: {}", e);
            }
        }
    }

    pub fn remove_file_type_override(mut self: Pin<&mut Self>, index: i32) {
        let idx = index as usize;
        if idx < self.as_ref().rust().inner.file_type_overrides.len() {
            self.as_mut().rust_mut().inner.file_type_overrides.remove(idx);
            let new_json = serde_json::to_string(&self.as_ref().rust().inner.file_type_overrides)
                .unwrap_or_else(|_| "[]".to_string());
            self.as_mut()
                .set_file_type_overrides_json(QString::from(new_json.as_str()));
            self.as_mut().settings_changed();
        }
    }

    pub fn add_command_on_save(mut self: Pin<&mut Self>, json: &QString) {
        let json_str = json.to_string();
        match serde_json::from_str::<impulse_core::settings::CommandOnSave>(&json_str) {
            Ok(command) => {
                self.as_mut().rust_mut().inner.commands_on_save.push(command);
                let new_json = serde_json::to_string(&self.as_ref().rust().inner.commands_on_save)
                    .unwrap_or_else(|_| "[]".to_string());
                self.as_mut()
                    .set_commands_on_save_json(QString::from(new_json.as_str()));
                self.as_mut().settings_changed();
            }
            Err(e) => {
                log::warn!("Failed to parse command on save: {}", e);
            }
        }
    }

    pub fn remove_command_on_save(mut self: Pin<&mut Self>, index: i32) {
        let idx = index as usize;
        if idx < self.as_ref().rust().inner.commands_on_save.len() {
            self.as_mut().rust_mut().inner.commands_on_save.remove(idx);
            let new_json = serde_json::to_string(&self.as_ref().rust().inner.commands_on_save)
                .unwrap_or_else(|_| "[]".to_string());
            self.as_mut()
                .set_commands_on_save_json(QString::from(new_json.as_str()));
            self.as_mut().settings_changed();
        }
    }

    /// Apply a full Settings struct to all QObject properties.
    fn apply_settings(mut self: Pin<&mut Self>, s: impulse_core::settings::Settings) {
        let new_state = SettingsModelRust::from_settings(s);

        // Set all properties via the Pin setters to trigger QML change notifications
        self.as_mut().set_window_width(new_state.window_width);
        self.as_mut().set_window_height(new_state.window_height);
        self.as_mut().set_sidebar_visible(new_state.sidebar_visible);
        self.as_mut().set_sidebar_width(new_state.sidebar_width);
        self.as_mut()
            .set_last_directory(new_state.last_directory.clone());
        self.as_mut()
            .set_open_files_json(new_state.open_files_json.clone());
        self.as_mut().set_auto_save(new_state.auto_save);
        self.as_mut().set_font_size(new_state.font_size);
        self.as_mut()
            .set_font_family(new_state.font_family.clone());
        self.as_mut().set_tab_width(new_state.tab_width);
        self.as_mut().set_use_spaces(new_state.use_spaces);
        self.as_mut()
            .set_show_line_numbers(new_state.show_line_numbers);
        self.as_mut()
            .set_show_right_margin(new_state.show_right_margin);
        self.as_mut()
            .set_right_margin_position(new_state.right_margin_position);
        self.as_mut().set_word_wrap(new_state.word_wrap);
        self.as_mut()
            .set_highlight_current_line(new_state.highlight_current_line);
        self.as_mut()
            .set_minimap_enabled(new_state.minimap_enabled);
        self.as_mut()
            .set_render_whitespace(new_state.render_whitespace.clone());
        self.as_mut().set_sticky_scroll(new_state.sticky_scroll);
        self.as_mut()
            .set_bracket_pair_colorization(new_state.bracket_pair_colorization);
        self.as_mut().set_indent_guides(new_state.indent_guides);
        self.as_mut().set_font_ligatures(new_state.font_ligatures);
        self.as_mut().set_folding(new_state.folding);
        self.as_mut()
            .set_scroll_beyond_last_line(new_state.scroll_beyond_last_line);
        self.as_mut()
            .set_smooth_scrolling(new_state.smooth_scrolling);
        self.as_mut()
            .set_editor_cursor_style(new_state.editor_cursor_style.clone());
        self.as_mut()
            .set_editor_cursor_blinking(new_state.editor_cursor_blinking.clone());
        self.as_mut()
            .set_terminal_scrollback(new_state.terminal_scrollback);
        self.as_mut()
            .set_terminal_cursor_shape(new_state.terminal_cursor_shape.clone());
        self.as_mut()
            .set_terminal_cursor_blink(new_state.terminal_cursor_blink);
        self.as_mut().set_terminal_bell(new_state.terminal_bell);
        self.as_mut()
            .set_terminal_font_family(new_state.terminal_font_family.clone());
        self.as_mut()
            .set_terminal_font_size(new_state.terminal_font_size);
        self.as_mut()
            .set_terminal_copy_on_select(new_state.terminal_copy_on_select);
        self.as_mut()
            .set_terminal_scroll_on_output(new_state.terminal_scroll_on_output);
        self.as_mut()
            .set_terminal_allow_hyperlink(new_state.terminal_allow_hyperlink);
        self.as_mut()
            .set_terminal_bold_is_bright(new_state.terminal_bold_is_bright);
        self.as_mut()
            .set_editor_line_height(new_state.editor_line_height);
        self.as_mut()
            .set_editor_auto_closing_brackets(new_state.editor_auto_closing_brackets.clone());
        self.as_mut()
            .set_editor_cursor_surrounding_lines(new_state.editor_cursor_surrounding_lines);
        self.as_mut()
            .set_editor_selection_highlight(new_state.editor_selection_highlight);
        self.as_mut()
            .set_editor_occurrences_highlight(new_state.editor_occurrences_highlight);
        self.as_mut()
            .set_editor_word_based_suggestions(new_state.editor_word_based_suggestions.clone());
        self.as_mut()
            .set_sidebar_show_hidden(new_state.sidebar_show_hidden);
        self.as_mut()
            .set_color_scheme(new_state.color_scheme.clone());
        self.as_mut()
            .set_check_for_updates(new_state.check_for_updates);
        self.as_mut()
            .set_keybinding_overrides_json(new_state.keybinding_overrides_json.clone());
        self.as_mut()
            .set_file_type_overrides_json(new_state.file_type_overrides_json.clone());
        self.as_mut()
            .set_commands_on_save_json(new_state.commands_on_save_json.clone());
        self.as_mut()
            .set_custom_keybindings_json(new_state.custom_keybindings_json.clone());

        // Update the inner settings struct
        self.as_mut().rust_mut().inner = new_state.inner;
    }
}
