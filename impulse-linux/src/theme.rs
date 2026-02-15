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

/// Kanagawa Wave — warm golden tones inspired by Hokusai's Great Wave (default).
pub static KANAGAWA: ThemeColors = ThemeColors {
    bg: "#1F1F28",
    bg_dark: "#16161D",
    bg_highlight: "#2A2A37",
    fg: "#DCD7BA",
    fg_dark: "#C8C093",
    cyan: "#7AA89F",
    blue: "#7E9CD8",
    green: "#98BB6C",
    magenta: "#957FB8",
    red: "#E46876",
    yellow: "#E6C384",
    orange: "#FFA066",
    comment: "#727169",
    terminal_palette: [
        "#090618", "#C34043", "#76946A", "#C0A36E", "#7E9CD8", "#957FB8", "#6A9589", "#C8C093",
        "#727169", "#E82424", "#98BB6C", "#E6C384", "#7FB4CA", "#938AA9", "#7AA89F", "#DCD7BA",
    ],
};

/// Rosé Pine — muted pastels on warm dark purple, "soho vibes".
pub static ROSE_PINE: ThemeColors = ThemeColors {
    bg: "#191724",
    bg_dark: "#1f1d2e",
    bg_highlight: "#26233a",
    fg: "#e0def4",
    fg_dark: "#908caa",
    cyan: "#9ccfd8",
    blue: "#31748f",
    green: "#9ccfd8",
    magenta: "#c4a7e7",
    red: "#eb6f92",
    yellow: "#f6c177",
    orange: "#ebbcba",
    comment: "#6e6a86",
    terminal_palette: [
        "#26233a", "#eb6f92", "#31748f", "#f6c177", "#9ccfd8", "#c4a7e7", "#ebbcba", "#e0def4",
        "#6e6a86", "#eb6f92", "#31748f", "#f6c177", "#9ccfd8", "#c4a7e7", "#ebbcba", "#e0def4",
    ],
};

/// Nord — arctic, clean, minimal blue palette.
pub static NORD: ThemeColors = ThemeColors {
    bg: "#2E3440",
    bg_dark: "#272C36",
    bg_highlight: "#434C5E",
    fg: "#D8DEE9",
    fg_dark: "#E5E9F0",
    cyan: "#88C0D0",
    blue: "#81A1C1",
    green: "#A3BE8C",
    magenta: "#B48EAD",
    red: "#BF616A",
    yellow: "#EBCB8B",
    orange: "#D08770",
    comment: "#4C566A",
    terminal_palette: [
        "#3B4252", "#BF616A", "#A3BE8C", "#EBCB8B", "#81A1C1", "#B48EAD", "#88C0D0", "#E5E9F0",
        "#4C566A", "#BF616A", "#A3BE8C", "#EBCB8B", "#81A1C1", "#B48EAD", "#8FBCBB", "#ECEFF4",
    ],
};

/// Gruvbox Dark — warm retro palette with earthy tones.
pub static GRUVBOX: ThemeColors = ThemeColors {
    bg: "#282828",
    bg_dark: "#1d2021",
    bg_highlight: "#3c3836",
    fg: "#ebdbb2",
    fg_dark: "#d5c4a1",
    cyan: "#8ec07c",
    blue: "#83a598",
    green: "#b8bb26",
    magenta: "#d3869b",
    red: "#fb4934",
    yellow: "#fabd2f",
    orange: "#fe8019",
    comment: "#928374",
    terminal_palette: [
        "#282828", "#cc241d", "#98971a", "#d79921", "#458588", "#b16286", "#689d6a", "#a89984",
        "#928374", "#fb4934", "#b8bb26", "#fabd2f", "#83a598", "#d3869b", "#8ec07c", "#ebdbb2",
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

/// Tokyo Night Storm — deeper blue-tinted variant of Tokyo Night.
pub static TOKYO_NIGHT_STORM: ThemeColors = ThemeColors {
    bg: "#24283b",
    bg_dark: "#1f2335",
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
        "#1d202f", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#a9b1d6",
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

/// Dracula — iconic purple-tinted dark theme with vibrant accents.
pub static DRACULA: ThemeColors = ThemeColors {
    bg: "#282a36",
    bg_dark: "#21222c",
    bg_highlight: "#44475a",
    fg: "#f8f8f2",
    fg_dark: "#6272a4",
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

/// Solarized Dark — precision-engineered palette with balanced contrast.
pub static SOLARIZED_DARK: ThemeColors = ThemeColors {
    bg: "#002b36",
    bg_dark: "#001e26",
    bg_highlight: "#073642",
    fg: "#839496",
    fg_dark: "#586e75",
    cyan: "#2aa198",
    blue: "#268bd2",
    green: "#859900",
    magenta: "#d33682",
    red: "#dc322f",
    yellow: "#b58900",
    orange: "#cb4b16",
    comment: "#586e75",
    terminal_palette: [
        "#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198", "#eee8d5",
        "#002b36", "#cb4b16", "#586e75", "#657b83", "#839496", "#6c71c4", "#93a1a1", "#fdf6e3",
    ],
};

/// One Dark — Atom-inspired balanced dark theme.
pub static ONE_DARK: ThemeColors = ThemeColors {
    bg: "#282c34",
    bg_dark: "#21252b",
    bg_highlight: "#2c313a",
    fg: "#abb2bf",
    fg_dark: "#5c6370",
    cyan: "#56b6c2",
    blue: "#61afef",
    green: "#98c379",
    magenta: "#c678dd",
    red: "#e06c75",
    yellow: "#e5c07b",
    orange: "#d19a66",
    comment: "#5c6370",
    terminal_palette: [
        "#21252b", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#abb2bf",
        "#5c6370", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#ffffff",
    ],
};

/// Ayu Dark — minimal dark theme with warm accent colors.
pub static AYU_DARK: ThemeColors = ThemeColors {
    bg: "#0b0e14",
    bg_dark: "#07090d",
    bg_highlight: "#131721",
    fg: "#bfbdb6",
    fg_dark: "#565b66",
    cyan: "#73b8ff",
    blue: "#59c2ff",
    green: "#aad94c",
    magenta: "#d2a6ff",
    red: "#f07178",
    yellow: "#ffb454",
    orange: "#ff8f40",
    comment: "#565b66",
    terminal_palette: [
        "#07090d", "#f07178", "#aad94c", "#ffb454", "#59c2ff", "#d2a6ff", "#73b8ff", "#bfbdb6",
        "#565b66", "#f07178", "#aad94c", "#ffb454", "#59c2ff", "#d2a6ff", "#73b8ff", "#ffffff",
    ],
};

// ---------------------------------------------------------------------------
// Theme lookup helpers
// ---------------------------------------------------------------------------

/// Return the theme matching `name` (case-insensitive). Falls back to `KANAGAWA`.
pub fn get_theme(name: &str) -> &'static ThemeColors {
    match name.to_ascii_lowercase().as_str() {
        "kanagawa" => &KANAGAWA,
        "rose-pine" | "rose_pine" | "rosepine" => &ROSE_PINE,
        "nord" => &NORD,
        "gruvbox" | "gruvbox-dark" | "gruvbox_dark" => &GRUVBOX,
        "tokyo-night" | "tokyo_night" | "tokyonight" => &TOKYO_NIGHT,
        "tokyo-night-storm" | "tokyo_night_storm" | "tokyonightstorm" => &TOKYO_NIGHT_STORM,
        "catppuccin-mocha" | "catppuccin_mocha" | "catppuccinmocha" => &CATPPUCCIN_MOCHA,
        "dracula" => &DRACULA,
        "solarized-dark" | "solarized_dark" | "solarizeddark" => &SOLARIZED_DARK,
        "one-dark" | "one_dark" | "onedark" => &ONE_DARK,
        "ayu-dark" | "ayu_dark" | "ayudark" => &AYU_DARK,
        _ => &NORD,
    }
}

/// Return the list of built-in theme names.
pub fn get_available_themes() -> Vec<&'static str> {
    vec![
        "kanagawa",
        "rose-pine",
        "nord",
        "gruvbox",
        "tokyo-night",
        "tokyo-night-storm",
        "catppuccin-mocha",
        "dracula",
        "solarized-dark",
        "one-dark",
        "ayu-dark",
    ]
}

// ---------------------------------------------------------------------------
// CSS loading
// ---------------------------------------------------------------------------

fn parse_color(hex: &str) -> gdk::RGBA {
    gdk::RGBA::parse(hex).unwrap_or_else(|_| {
        log::warn!("Invalid color value: '{}', using fallback", hex);
        gdk::RGBA::new(1.0, 0.0, 1.0, 1.0) // Magenta fallback makes errors visible
    })
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
            padding: 6px 8px;
            background-color: {bg_dark};
        }}
        .sidebar-tab {{
            border-radius: 6px;
            padding: 4px 12px;
            font-size: 12px;
            font-weight: 500;
            color: {fg_dark};
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
        .sidebar-toolbar-btn {{
            min-width: 24px;
            min-height: 24px;
            padding: 2px;
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
            color: {fg};
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
        .git-deleted {{
            color: {red};
        }}
        .git-renamed {{
            color: {blue};
        }}
        .git-conflict {{
            color: {orange};
        }}
        .file-entry-git-modified {{
            color: {yellow};
        }}
        .file-entry-git-added {{
            color: {green};
        }}
        .file-entry-git-untracked {{
            color: {comment};
        }}
        .file-entry-git-deleted {{
            color: {red};
        }}
        .file-entry-git-renamed {{
            color: {blue};
        }}
        .file-entry-git-conflict {{
            color: {orange};
        }}
        .drop-target {{
            background-color: alpha({cyan}, 0.10);
            outline: 1px dashed {cyan};
            outline-offset: -1px;
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
            color: {fg_dark};
            padding-left: 12px;
        }}
        .status-bar .indent-info {{
            color: {fg_dark};
            padding-left: 12px;
        }}
        .status-bar .blame-info {{
            color: {fg_dark};
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
            box-shadow: none;
            border-bottom: none;
        }}
        tabbar tabbox {{
            background-color: {bg_dark};
            box-shadow: none;
            border-bottom: none;
        }}
        tabbar tab {{
            min-height: 32px;
            padding: 0 8px;
            background-color: {bg_dark};
            color: {fg_dark};
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
        tabbar tab image {{
            margin-right: 2px;
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
        /* --- Project search panel --- */
        .project-search-panel {{
            background-color: {bg_dark};
            border-top: 1px solid {bg_highlight};
        }}
        .project-search-row {{
            padding: 4px 8px;
        }}
        .project-search-row entry,
        .project-search-row search {{
            min-height: 28px;
        }}
        .project-search-toggle {{
            min-height: 24px;
            min-width: 24px;
            padding: 2px 8px;
            font-size: 12px;
        }}
        .project-search-count {{
            font-size: 11px;
            color: {comment};
            padding: 2px 8px;
        }}
        .project-search-results {{
            background-color: transparent;
        }}
        .project-search-results row:hover {{
            background-color: {bg_highlight};
        }}
        .project-search-results row:selected {{
            background-color: {bg_highlight};
        }}
        .project-search-file-header {{
            padding: 4px 8px;
            background-color: {bg_dark};
        }}
        .project-search-filename {{
            color: {cyan};
            font-size: 12px;
            font-weight: bold;
        }}
        .project-search-match-count {{
            color: {comment};
            font-size: 11px;
        }}
        .project-search-match {{
            padding: 2px 8px 2px 16px;
        }}
        .project-search-line-num {{
            color: {comment};
            font-size: 11px;
            font-family: monospace;
        }}
        .project-search-line-content {{
            color: {fg};
            font-size: 12px;
            font-family: monospace;
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
        red = theme.red,
        orange = theme.orange,
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
