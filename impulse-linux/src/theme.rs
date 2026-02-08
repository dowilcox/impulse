use gtk4::gdk;

/// A color theme definition for the entire application.
pub struct ThemeColors {
    pub bg: &'static str,
    pub bg_dark: &'static str,
    pub bg_highlight: &'static str,
    pub fg: &'static str,
    pub fg_dark: &'static str,
    pub cyan: &'static str,
    pub blue: &'static str,
    pub green: &'static str,
    pub magenta: &'static str,
    pub red: &'static str,
    pub yellow: &'static str,
    pub orange: &'static str,
    pub comment: &'static str,
    pub terminal_palette: [&'static str; 16],
}

impl ThemeColors {
    /// Parse the 16-color terminal palette into VTE-compatible RGBA values.
    pub fn terminal_palette_rgba(&self) -> Vec<gdk::RGBA> {
        self.terminal_palette
            .iter()
            .map(|hex| parse_color(hex))
            .collect()
    }

    pub fn fg_rgba(&self) -> gdk::RGBA {
        parse_color(self.fg)
    }

    pub fn bg_rgba(&self) -> gdk::RGBA {
        parse_color(self.bg)
    }
}

// ---------------------------------------------------------------------------
// Built-in themes
// ---------------------------------------------------------------------------

/// Cyberpunk — neon accents on deep dark purple (default).
pub static CYBERPUNK: ThemeColors = ThemeColors {
    bg: "#2b2444",
    bg_dark: "#221c3a",
    bg_highlight: "#3a3260",
    fg: "#d8d5e8",
    fg_dark: "#a8a2be",
    cyan: "#5ecfb8",
    blue: "#56b8d6",
    green: "#7ddb6a",
    magenta: "#d97aaa",
    red: "#e05577",
    yellow: "#d4b855",
    orange: "#d98a4a",
    comment: "#605888",
    terminal_palette: [
        "#1c1735", "#e05577", "#7ddb6a", "#d4b855", "#56b8d6", "#b85aac", "#5ecfb8", "#a8a2be",
        "#605888", "#e87a95", "#96e484", "#e0c96e", "#6ec8e0", "#cc76c0", "#7adbc8", "#d8d5e8",
    ],
};

/// Tokyo Night — cool blue-purple palette.
pub static TOKYO_NIGHT: ThemeColors = ThemeColors {
    bg: "#1a1b26",
    bg_dark: "#16161e",
    bg_highlight: "#292e42",
    fg: "#c0caf5",
    fg_dark: "#a9b1d6",
    cyan: "#7dcfff",
    blue: "#7aa2f7",
    green: "#9ece6a",
    magenta: "#bb9af7",
    red: "#f7768e",
    yellow: "#e0af68",
    orange: "#ff9e64",
    comment: "#565f89",
    terminal_palette: [
        "#15161e", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#a9b1d6",
        "#414868", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#c0caf5",
    ],
};

/// Catppuccin Mocha — warm pastel palette on dark base.
pub static CATPPUCCIN_MOCHA: ThemeColors = ThemeColors {
    bg: "#1e1e2e",
    bg_dark: "#181825",
    bg_highlight: "#313244",
    fg: "#cdd6f4",
    fg_dark: "#bac2de",
    cyan: "#94e2d5",
    blue: "#89b4fa",
    green: "#a6e3a1",
    magenta: "#cba6f7",
    red: "#f38ba8",
    yellow: "#f9e2af",
    orange: "#fab387",
    comment: "#6c7086",
    terminal_palette: [
        "#45475a", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#cba6f7", "#94e2d5", "#bac2de",
        "#585b70", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#cba6f7", "#94e2d5", "#cdd6f4",
    ],
};

/// Dracula — classic dark theme with vivid accents.
pub static DRACULA: ThemeColors = ThemeColors {
    bg: "#282a36",
    bg_dark: "#21222c",
    bg_highlight: "#44475a",
    fg: "#f8f8f2",
    fg_dark: "#bfbfbf",
    cyan: "#8be9fd",
    blue: "#6272a4",
    green: "#50fa7b",
    magenta: "#ff79c6",
    red: "#ff5555",
    yellow: "#f1fa8c",
    orange: "#ffb86c",
    comment: "#6272a4",
    terminal_palette: [
        "#21222c", "#ff5555", "#50fa7b", "#f1fa8c", "#bd93f9", "#ff79c6", "#8be9fd", "#f8f8f2",
        "#6272a4", "#ff6e6e", "#69ff94", "#ffffa5", "#d6acff", "#ff92df", "#a4ffff", "#ffffff",
    ],
};

// ---------------------------------------------------------------------------
// Theme lookup helpers
// ---------------------------------------------------------------------------

/// Return the theme matching `name` (case-insensitive). Falls back to `CYBERPUNK`.
pub fn get_theme(name: &str) -> &'static ThemeColors {
    match name.to_ascii_lowercase().as_str() {
        "cyberpunk" => &CYBERPUNK,
        "tokyo-night" | "tokyo_night" | "tokyonight" => &TOKYO_NIGHT,
        "catppuccin-mocha" | "catppuccin_mocha" | "catppuccinmocha" => &CATPPUCCIN_MOCHA,
        "dracula" => &DRACULA,
        _ => &CYBERPUNK,
    }
}

/// Return the list of built-in theme names.
pub fn get_available_themes() -> Vec<&'static str> {
    vec!["cyberpunk", "tokyo-night", "catppuccin-mocha", "dracula"]
}

// ---------------------------------------------------------------------------
// CSS loading
// ---------------------------------------------------------------------------

fn parse_color(hex: &str) -> gdk::RGBA {
    gdk::RGBA::parse(hex).unwrap_or(gdk::RGBA::WHITE)
}

/// Generate a GtkSourceView 5 style scheme XML that matches the given theme,
/// write it to the user styles directory, and return the scheme ID.
///
/// The scheme is installed at `~/.local/share/gtksourceview-5/styles/` so that
/// GtkSourceView's `StyleSchemeManager` can discover it.
pub fn install_sourceview_scheme(theme: &ThemeColors, theme_name: &str) -> String {
    let scheme_id = format!(
        "impulse-{}",
        theme_name.to_lowercase().replace(' ', "-")
    );
    let display_name = format!("Impulse {}", theme_name);

    let xml = format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<style-scheme id="{id}" name="{name}" version="1.0">
  <author>Impulse</author>
  <description>Auto-generated Impulse editor theme</description>

  <!-- Editor chrome -->
  <style name="text" foreground="{fg}" background="{bg}"/>
  <style name="line-numbers" foreground="{comment}" background="{bg_dark}"/>
  <style name="current-line" background="{bg_highlight}"/>
  <style name="current-line-number" foreground="{fg}" background="{bg_highlight}"/>
  <style name="cursor" foreground="{fg}"/>
  <style name="selection" background="{bg_highlight}"/>
  <style name="bracket-match" foreground="{cyan}" background="{bg_highlight}" bold="true"/>
  <style name="right-margin" foreground="{comment}" background="{bg_dark}"/>
  <style name="draw-spaces" foreground="{comment}"/>
  <style name="background-pattern" background="{bg_dark}"/>

  <!-- Syntax highlighting -->
  <style name="def:comment" foreground="{comment}" italic="true"/>
  <style name="def:shebang" foreground="{comment}" italic="true"/>
  <style name="def:doc-comment" foreground="{comment}" italic="true"/>
  <style name="def:doc-comment-element" foreground="{comment}" bold="true"/>
  <style name="def:string" foreground="{green}"/>
  <style name="def:special-char" foreground="{orange}"/>
  <style name="def:character" foreground="{orange}"/>
  <style name="def:keyword" foreground="{magenta}"/>
  <style name="def:builtin" foreground="{cyan}"/>
  <style name="def:type" foreground="{cyan}"/>
  <style name="def:function" foreground="{blue}"/>
  <style name="def:constant" foreground="{orange}"/>
  <style name="def:number" foreground="{yellow}"/>
  <style name="def:decimal" foreground="{yellow}"/>
  <style name="def:floating-point" foreground="{yellow}"/>
  <style name="def:base-n-integer" foreground="{yellow}"/>
  <style name="def:complex" foreground="{yellow}"/>
  <style name="def:boolean" foreground="{orange}"/>
  <style name="def:preprocessor" foreground="{red}"/>
  <style name="def:identifier" foreground="{fg}"/>
  <style name="def:operator" foreground="{fg_dark}"/>
  <style name="def:error" foreground="{red}" underline="true"/>
  <style name="def:warning" foreground="{yellow}" underline="true"/>
  <style name="def:note" foreground="{blue}" italic="true"/>
  <style name="def:net-address" foreground="{blue}" underline="single"/>
  <style name="def:heading" foreground="{cyan}" bold="true"/>
  <style name="def:statement" foreground="{magenta}"/>
  <style name="def:special-constant" foreground="{orange}"/>
  <style name="def:underlined" underline="single"/>
  <style name="def:deletion" foreground="{red}" strikethrough="true"/>
  <style name="def:insertion" foreground="{green}"/>

</style-scheme>"##,
        id = scheme_id,
        name = display_name,
        bg = theme.bg,
        bg_dark = theme.bg_dark,
        bg_highlight = theme.bg_highlight,
        fg = theme.fg,
        fg_dark = theme.fg_dark,
        cyan = theme.cyan,
        blue = theme.blue,
        green = theme.green,
        magenta = theme.magenta,
        red = theme.red,
        yellow = theme.yellow,
        orange = theme.orange,
        comment = theme.comment,
    );

    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let styles_dir =
        std::path::PathBuf::from(&home).join(".config/impulse/styles");
    let _ = std::fs::create_dir_all(&styles_dir);
    let _ = std::fs::write(styles_dir.join(format!("{}.xml", scheme_id)), xml);

    // Clean up stale scheme file from the old location (if any)
    let old_dir =
        std::path::PathBuf::from(&home).join(".local/share/gtksourceview-5/styles");
    let _ = std::fs::remove_file(old_dir.join(format!("{}.xml", scheme_id)));

    scheme_id
}

/// Generate and apply the application-wide CSS for the given theme.
///
/// Returns the `CssProvider` so callers can hold onto it and later replace it
/// when switching themes at runtime.
pub fn load_css(theme: &ThemeColors) -> gtk4::CssProvider {
    let css = format!(
        r#"
        /* --- Sidebar --- */
        .sidebar {{
            background-color: {bg_dark};
            border-right: 1px solid {bg_highlight};
        }}
        .sidebar-switcher {{
            padding: 4px 6px;
            background-color: {bg_dark};
        }}
        .sidebar-tab {{
            border-radius: 6px;
            padding: 4px 12px;
            font-size: 12px;
            font-weight: 500;
            color: {comment};
            background: transparent;
            border: none;
            box-shadow: none;
            min-height: 28px;
        }}
        .sidebar-tab:hover {{
            color: {fg_dark};
            background-color: {bg_highlight};
        }}
        .sidebar-tab-active {{
            color: {cyan};
            background-color: {bg_highlight};
        }}
        .file-tree {{
            background-color: transparent;
        }}
        .file-tree row:hover {{
            background-color: {bg_highlight};
        }}
        .file-tree row:selected {{
            background-color: {bg_highlight};
        }}
        .file-entry {{
            padding: 3px 10px;
        }}
        .file-entry-dir {{
            color: {cyan};
            font-weight: bold;
        }}
        .file-entry-file {{
            color: {fg};
        }}
        .git-modified {{
            color: {yellow};
        }}
        .git-added {{
            color: {green};
        }}
        .git-untracked {{
            color: {comment};
        }}
        /* --- Search --- */
        .search-entry {{
            margin: 6px 8px;
        }}
        .search-result {{
            padding: 4px 10px;
        }}
        .search-result:hover {{
            background-color: {bg_highlight};
        }}
        .search-result-path {{
            font-size: 11px;
            color: {comment};
        }}
        .search-result-line {{
            font-size: 12px;
            color: {fg};
        }}
        /* --- Status bar --- */
        .status-bar {{
            background-color: {bg_dark};
            padding: 2px 12px;
            min-height: 24px;
            border-top: 1px solid {bg_highlight};
        }}
        .status-bar label {{
            font-size: 12px;
            color: {fg_dark};
        }}
        .status-bar .git-branch {{
            color: {magenta};
        }}
        .status-bar .shell-name {{
            color: {cyan};
        }}
        .status-bar .cwd {{
            color: {fg};
        }}
        .status-bar .cursor-pos {{
            color: {fg_dark};
            padding-left: 12px;
        }}
        .status-bar .language-name {{
            color: {blue};
            padding-left: 12px;
        }}
        .status-bar .encoding {{
            color: {comment};
            padding-left: 12px;
        }}
        .status-bar .indent-info {{
            color: {comment};
            padding-left: 12px;
        }}
        .status-bar .blame-info {{
            color: {comment};
            font-size: 11px;
        }}
        /* --- Completion popup --- */
        .completion-list row {{
            padding: 2px 4px;
        }}
        .completion-kind {{
            color: {cyan};
            font-size: 10px;
            font-weight: bold;
        }}
        .completion-detail {{
            color: {comment};
            font-size: 11px;
        }}
        /* --- Terminal --- */
        vte-terminal {{
            padding: 8px 12px;
        }}
        /* --- Header bar --- */
        headerbar {{
            background-color: {bg_dark};
            border-bottom: 1px solid {bg_highlight};
            box-shadow: none;
            min-height: 38px;
        }}
        tabbar {{
            background-color: {bg_dark};
        }}
        tabbar tabbox {{
            background-color: {bg_dark};
        }}
        tabbar tab {{
            min-height: 32px;
            padding: 0 8px;
            background-color: {bg_dark};
            color: {comment};
            border-radius: 6px 6px 0 0;
        }}
        tabbar tab:selected {{
            background-color: {bg};
            color: {cyan};
        }}
        tabbar tab:hover:not(:selected) {{
            background-color: {bg_highlight};
            color: {fg_dark};
        }}
        tabbar tab label {{
            font-size: 13px;
            font-weight: 500;
        }}
        headerbar button {{
            color: {fg_dark};
        }}
        headerbar button:hover {{
            color: {cyan};
            background-color: {bg_highlight};
        }}
        window.background {{
            background-color: {bg};
        }}
        /* --- Quick open --- */
        .quick-open {{
            background-color: {bg};
            border-radius: 8px;
            border: 1px solid {bg_highlight};
        }}
        .quick-open entry {{
            margin: 8px;
            font-size: 14px;
        }}
        .quick-open list row:hover {{
            background-color: {bg_highlight};
        }}
        .quick-open list row:selected {{
            background-color: {bg_highlight};
        }}
        .quick-open list row label {{
            padding: 6px 12px;
            color: {fg};
        }}
        /* --- Terminal search bar --- */
        .terminal-search-bar {{
            background-color: {bg_dark};
            padding: 4px 8px;
            border-bottom: 1px solid {bg_highlight};
        }}
        .terminal-search-bar entry {{
            min-height: 28px;
        }}
        .terminal-search-bar button {{
            min-height: 24px;
            min-width: 24px;
            padding: 2px 6px;
        }}
        .terminal-search-bar .dim-label {{
            color: {comment};
            font-size: 11px;
            margin: 0 4px;
        }}
        /* --- Editor (GtkSourceView) --- */
        textview.view text {{
            background-color: {bg};
            color: {fg};
        }}
        textview.view {{
            font-family: monospace;
            font-size: 11pt;
            padding: 8px 12px;
        }}
        textview.view .line-numbers {{
            background-color: {bg_dark};
            color: {comment};
        }}
        textview.view .current-line-number {{
            color: {fg};
        }}
        /* --- Scrollbars --- */
        scrollbar slider {{
            background-color: {comment};
            border-radius: 3px;
            min-width: 6px;
            min-height: 6px;
        }}
        scrollbar slider:hover {{
            background-color: {fg_dark};
        }}
        "#,
        bg_dark = theme.bg_dark,
        bg = theme.bg,
        bg_highlight = theme.bg_highlight,
        fg = theme.fg,
        fg_dark = theme.fg_dark,
        cyan = theme.cyan,
        blue = theme.blue,
        magenta = theme.magenta,
        green = theme.green,
        yellow = theme.yellow,
        comment = theme.comment,
    );

    let provider = gtk4::CssProvider::new();
    provider.load_from_string(&css);
    gtk4::style_context_add_provider_for_display(
        &gtk4::gdk::Display::default().expect("Could not get default display"),
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    provider
}
