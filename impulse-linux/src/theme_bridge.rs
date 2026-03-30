// SPDX-License-Identifier: GPL-3.0-only
//
// Theme data bridge QObject for QML. Exposes resolved theme colors
// and Monaco theme JSON to the QML layer.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        // UI backgrounds
        #[qproperty(QString, bg)]
        #[qproperty(QString, fg)]
        #[qproperty(QString, accent)]
        #[qproperty(QString, border)]
        #[qproperty(QString, bg_dark)]
        #[qproperty(QString, bg_highlight)]
        #[qproperty(QString, bg_surface)]
        #[qproperty(QString, fg_muted)]
        // Palette hues
        #[qproperty(QString, red)]
        #[qproperty(QString, orange)]
        #[qproperty(QString, yellow)]
        #[qproperty(QString, green)]
        #[qproperty(QString, cyan)]
        #[qproperty(QString, blue)]
        #[qproperty(QString, magenta)]
        // Selection / cursor
        #[qproperty(QString, selection)]
        #[qproperty(QString, cursor_color)]
        // Terminal
        #[qproperty(QString, terminal_palette_json)]
        // Theme metadata
        #[qproperty(bool, is_light)]
        #[qproperty(QString, theme_id)]
        #[qproperty(QString, theme_name)]
        #[qproperty(QString, available_themes_json)]
        #[qproperty(QString, monaco_theme_json)]
        type ThemeBridge = super::ThemeBridgeRust;

        #[qinvokable]
        fn set_theme(self: Pin<&mut ThemeBridge>, id: &QString);

        #[qinvokable]
        fn get_markdown_theme_json(self: &ThemeBridge) -> QString;
    }
}

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use std::pin::Pin;

pub struct ThemeBridgeRust {
    bg: QString,
    fg: QString,
    accent: QString,
    border: QString,
    bg_dark: QString,
    bg_highlight: QString,
    bg_surface: QString,
    fg_muted: QString,
    red: QString,
    orange: QString,
    yellow: QString,
    green: QString,
    cyan: QString,
    blue: QString,
    magenta: QString,
    selection: QString,
    cursor_color: QString,
    terminal_palette_json: QString,
    is_light: bool,
    theme_id: QString,
    theme_name: QString,
    available_themes_json: QString,
    monaco_theme_json: QString,
    // Internal cached resolved theme for markdown preview, etc.
    resolved_theme: Option<impulse_core::theme::ResolvedTheme>,
}

impl Default for ThemeBridgeRust {
    fn default() -> Self {
        let mut s = Self {
            bg: QString::default(),
            fg: QString::default(),
            accent: QString::default(),
            border: QString::default(),
            bg_dark: QString::default(),
            bg_highlight: QString::default(),
            bg_surface: QString::default(),
            fg_muted: QString::default(),
            red: QString::default(),
            orange: QString::default(),
            yellow: QString::default(),
            green: QString::default(),
            cyan: QString::default(),
            blue: QString::default(),
            magenta: QString::default(),
            selection: QString::default(),
            cursor_color: QString::default(),
            terminal_palette_json: QString::from("[]"),
            is_light: false,
            theme_id: QString::from("nord"),
            theme_name: QString::from("Nord"),
            available_themes_json: QString::from("[]"),
            monaco_theme_json: QString::from("{}"),
            resolved_theme: None,
        };

        // Build available themes list
        s.rebuild_available_themes();

        // Load default theme
        s.apply_theme("nord");

        s
    }
}

impl ThemeBridgeRust {
    fn apply_theme(&mut self, id: &str) {
        let theme = impulse_core::theme::get_theme(id);

        self.bg = QString::from(theme.bg.as_str());
        self.fg = QString::from(theme.fg.as_str());
        self.accent = QString::from(theme.accent.as_str());
        self.border = QString::from(theme.border.as_str());
        self.bg_dark = QString::from(theme.bg_dark.as_str());
        self.bg_highlight = QString::from(theme.bg_highlight.as_str());
        self.bg_surface = QString::from(theme.bg_surface.as_str());
        self.fg_muted = QString::from(theme.fg_muted.as_str());
        self.red = QString::from(theme.red.as_str());
        self.orange = QString::from(theme.orange.as_str());
        self.yellow = QString::from(theme.yellow.as_str());
        self.green = QString::from(theme.green.as_str());
        self.cyan = QString::from(theme.cyan.as_str());
        self.blue = QString::from(theme.blue.as_str());
        self.magenta = QString::from(theme.magenta.as_str());
        self.selection = QString::from(theme.selection.as_str());
        self.cursor_color = QString::from(theme.cursor.as_str());
        self.is_light = theme.is_light;
        self.theme_id = QString::from(theme.id.as_str());
        self.theme_name = QString::from(theme.name.as_str());

        // Terminal palette as JSON array
        let palette_json =
            serde_json::to_string(&theme.terminal_palette).unwrap_or_else(|_| "[]".to_string());
        self.terminal_palette_json = QString::from(palette_json.as_str());

        // Monaco theme definition
        let monaco_def = impulse_editor::protocol::theme_to_monaco(&theme);
        let monaco_json =
            serde_json::to_string(&monaco_def).unwrap_or_else(|_| "{}".to_string());
        self.monaco_theme_json = QString::from(monaco_json.as_str());

        self.resolved_theme = Some(theme);
    }

    fn rebuild_available_themes(&mut self) {
        let themes = impulse_core::theme::available_themes();
        let theme_list: Vec<serde_json::Value> = themes
            .iter()
            .map(|id| {
                let display_name = impulse_core::theme::theme_display_name(id);
                serde_json::json!({
                    "id": id,
                    "name": display_name,
                })
            })
            .collect();

        let json = serde_json::to_string(&theme_list).unwrap_or_else(|_| "[]".to_string());
        self.available_themes_json = QString::from(json.as_str());
    }
}

impl qobject::ThemeBridge {
    pub fn set_theme(mut self: Pin<&mut Self>, id: &QString) {
        let id_str = id.to_string();
        eprintln!("[impulse] ThemeBridge::set_theme: {}", id_str);
        self.as_mut().rust_mut().apply_theme(&id_str);

        // Notify QML that all properties changed by re-setting the theme_id
        // (CXX-Qt auto-generates change signals for each qproperty).
        // Since apply_theme sets all fields on the Rust struct directly, we
        // need to use the Pin setters to trigger QML property change notifications.
        let rust = self.as_ref();
        let bg = rust.bg().clone();
        let fg = rust.fg().clone();
        let accent = rust.accent().clone();
        let border = rust.border().clone();
        let bg_dark = rust.bg_dark().clone();
        let bg_highlight = rust.bg_highlight().clone();
        let bg_surface = rust.bg_surface().clone();
        let fg_muted = rust.fg_muted().clone();
        let red = rust.red().clone();
        let orange = rust.orange().clone();
        let yellow = rust.yellow().clone();
        let green = rust.green().clone();
        let cyan = rust.cyan().clone();
        let blue = rust.blue().clone();
        let magenta = rust.magenta().clone();
        let selection = rust.selection().clone();
        let cursor_color = rust.cursor_color().clone();
        let terminal_palette_json = rust.terminal_palette_json().clone();
        let is_light = *rust.is_light();
        let theme_id = rust.theme_id().clone();
        let theme_name = rust.theme_name().clone();
        let monaco_theme_json = rust.monaco_theme_json().clone();

        self.as_mut().set_bg(bg);
        self.as_mut().set_fg(fg);
        self.as_mut().set_accent(accent);
        self.as_mut().set_border(border);
        self.as_mut().set_bg_dark(bg_dark);
        self.as_mut().set_bg_highlight(bg_highlight);
        self.as_mut().set_bg_surface(bg_surface);
        self.as_mut().set_fg_muted(fg_muted);
        self.as_mut().set_red(red);
        self.as_mut().set_orange(orange);
        self.as_mut().set_yellow(yellow);
        self.as_mut().set_green(green);
        self.as_mut().set_cyan(cyan);
        self.as_mut().set_blue(blue);
        self.as_mut().set_magenta(magenta);
        self.as_mut().set_selection(selection);
        self.as_mut().set_cursor_color(cursor_color);
        self.as_mut().set_terminal_palette_json(terminal_palette_json);
        self.as_mut().set_is_light(is_light);
        self.as_mut().set_theme_id(theme_id);
        self.as_mut().set_theme_name(theme_name);
        self.as_mut().set_monaco_theme_json(monaco_theme_json);
    }

    pub fn get_markdown_theme_json(&self) -> QString {
        match &self.resolved_theme {
            Some(theme) => {
                let md_colors = impulse_editor::markdown::theme_to_markdown_colors(theme);
                let json = serde_json::to_string(&md_colors).unwrap_or_else(|_| "{}".to_string());
                QString::from(json.as_str())
            }
            None => QString::from("{}"),
        }
    }
}
