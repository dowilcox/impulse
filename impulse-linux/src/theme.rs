use gtk4::gdk;

/// A color theme definition for the entire application.
pub struct ThemeColors {
    pub bg: &'static str,
    pub bg_dark: &'static str,
    pub bg_highlight: &'static str,
    /// Darkest background layer — used for status bar, sidebar header, deepest panels.
    pub bg_surface: &'static str,
    /// Explicit border/separator color — sits between `bg_dark` and `bg_highlight`.
    pub border: &'static str,
    /// Primary UI accent — used for active tab underline, active sidebar tab, focus rings.
    pub accent: &'static str,
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
    /// Monaco base theme: `"vs-dark"` for dark themes, `"vs"` for light themes.
    pub base: &'static str,
    /// Editor selection background — a hex color with alpha (e.g. `"#7E9CD850"`).
    pub selection: &'static str,
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
    bg: "#1a1a23",
    bg_dark: "#141419",
    bg_highlight: "#262633",
    bg_surface: "#0f0f14",
    border: "#1e1e2a",
    accent: "#7AA89F",
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
    base: "vs-dark",
    selection: "#7E9CD850",
    terminal_palette: [
        "#090618", "#C34043", "#76946A", "#C0A36E", "#7E9CD8", "#957FB8", "#6A9589", "#C8C093",
        "#727169", "#E82424", "#98BB6C", "#E6C384", "#7FB4CA", "#938AA9", "#7AA89F", "#DCD7BA",
    ],
};

/// Rosé Pine — muted pastels on warm dark purple, "soho vibes".
pub static ROSE_PINE: ThemeColors = ThemeColors {
    bg: "#14121f",
    bg_dark: "#0f0e19",
    bg_highlight: "#211f30",
    bg_surface: "#0a0914",
    border: "#1a1828",
    accent: "#9ccfd8",
    fg: "#e0def4",
    fg_dark: "#908caa",
    cyan: "#9ccfd8",
    blue: "#4392b5",
    green: "#9ccfd8",
    magenta: "#c4a7e7",
    red: "#eb6f92",
    yellow: "#f6c177",
    orange: "#ebbcba",
    comment: "#6e6a86",
    base: "vs-dark",
    selection: "#c4a7e740",
    terminal_palette: [
        "#26233a", "#eb6f92", "#31748f", "#f6c177", "#9ccfd8", "#c4a7e7", "#ebbcba", "#e0def4",
        "#6e6a86", "#eb6f92", "#31748f", "#f6c177", "#9ccfd8", "#c4a7e7", "#ebbcba", "#e0def4",
    ],
};

/// Nord — arctic, clean, minimal blue palette.
pub static NORD: ThemeColors = ThemeColors {
    bg: "#282e3a",
    bg_dark: "#222830",
    bg_highlight: "#3b4455",
    bg_surface: "#1b2028",
    border: "#2e3542",
    accent: "#88C0D0",
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
    base: "vs-dark",
    selection: "#81A1C150",
    terminal_palette: [
        "#3B4252", "#BF616A", "#A3BE8C", "#EBCB8B", "#81A1C1", "#B48EAD", "#88C0D0", "#E5E9F0",
        "#4C566A", "#BF616A", "#A3BE8C", "#EBCB8B", "#81A1C1", "#B48EAD", "#8FBCBB", "#ECEFF4",
    ],
};

/// Gruvbox Dark — warm retro palette with earthy tones.
pub static GRUVBOX: ThemeColors = ThemeColors {
    bg: "#222222",
    bg_dark: "#1a1a1a",
    bg_highlight: "#353230",
    bg_surface: "#131313",
    border: "#2a2826",
    accent: "#8ec07c",
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
    base: "vs-dark",
    selection: "#83a59850",
    terminal_palette: [
        "#282828", "#cc241d", "#98971a", "#d79921", "#458588", "#b16286", "#689d6a", "#a89984",
        "#928374", "#fb4934", "#b8bb26", "#fabd2f", "#83a598", "#d3869b", "#8ec07c", "#ebdbb2",
    ],
};

/// Tokyo Night — cool blue-purple palette.
pub static TOKYO_NIGHT: ThemeColors = ThemeColors {
    bg: "#151620",
    bg_dark: "#111119",
    bg_highlight: "#242938",
    bg_surface: "#0c0c13",
    border: "#1a1b28",
    accent: "#7dcfff",
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
    base: "vs-dark",
    selection: "#7aa2f740",
    terminal_palette: [
        "#15161e", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#a9b1d6",
        "#414868", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#c0caf5",
    ],
};

/// Tokyo Night Storm — deeper blue-tinted variant of Tokyo Night.
pub static TOKYO_NIGHT_STORM: ThemeColors = ThemeColors {
    bg: "#1e2235",
    bg_dark: "#191c2e",
    bg_highlight: "#252a3c",
    bg_surface: "#131627",
    border: "#1e2134",
    accent: "#7dcfff",
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
    base: "vs-dark",
    selection: "#7aa2f740",
    terminal_palette: [
        "#1d202f", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#a9b1d6",
        "#414868", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#c0caf5",
    ],
};

/// Catppuccin Mocha — warm pastel palette on dark base.
pub static CATPPUCCIN_MOCHA: ThemeColors = ThemeColors {
    bg: "#191928",
    bg_dark: "#131320",
    bg_highlight: "#2a2a3c",
    bg_surface: "#0e0e19",
    border: "#1f1f2e",
    accent: "#94e2d5",
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
    base: "vs-dark",
    selection: "#89b4fa40",
    terminal_palette: [
        "#45475a", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#cba6f7", "#94e2d5", "#bac2de",
        "#585b70", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#cba6f7", "#94e2d5", "#cdd6f4",
    ],
};

/// Dracula — iconic purple-tinted dark theme with vibrant accents.
pub static DRACULA: ThemeColors = ThemeColors {
    bg: "#22242f",
    bg_dark: "#1b1c26",
    bg_highlight: "#3b3e52",
    bg_surface: "#14151d",
    border: "#292b3a",
    accent: "#8be9fd",
    fg: "#f8f8f2",
    fg_dark: "#8490b7",
    cyan: "#8be9fd",
    blue: "#7c89b4",
    green: "#50fa7b",
    magenta: "#ff79c6",
    red: "#ff5555",
    yellow: "#f1fa8c",
    orange: "#ffb86c",
    comment: "#6272a4",
    base: "vs-dark",
    selection: "#bd93f940",
    terminal_palette: [
        "#21222c", "#ff5555", "#50fa7b", "#f1fa8c", "#bd93f9", "#ff79c6", "#8be9fd", "#f8f8f2",
        "#6272a4", "#ff6e6e", "#69ff94", "#ffffa5", "#d6acff", "#ff92df", "#a4ffff", "#ffffff",
    ],
};

/// Solarized Dark — precision-engineered palette with balanced contrast.
pub static SOLARIZED_DARK: ThemeColors = ThemeColors {
    bg: "#00242e",
    bg_dark: "#001920",
    bg_highlight: "#053340",
    bg_surface: "#001018",
    border: "#012a34",
    accent: "#2aa198",
    fg: "#839496",
    fg_dark: "#748e97",
    cyan: "#2aa198",
    blue: "#268bd2",
    green: "#859900",
    magenta: "#d33682",
    red: "#dc322f",
    yellow: "#b58900",
    orange: "#cb4b16",
    comment: "#586e75",
    base: "vs-dark",
    selection: "#268bd240",
    terminal_palette: [
        "#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198", "#eee8d5",
        "#002b36", "#cb4b16", "#586e75", "#657b83", "#839496", "#6c71c4", "#93a1a1", "#fdf6e3",
    ],
};

/// One Dark — Atom-inspired balanced dark theme.
pub static ONE_DARK: ThemeColors = ThemeColors {
    bg: "#22262e",
    bg_dark: "#1c1f25",
    bg_highlight: "#282d36",
    bg_surface: "#15181d",
    border: "#21252c",
    accent: "#56b6c2",
    fg: "#abb2bf",
    fg_dark: "#8c93a1",
    cyan: "#56b6c2",
    blue: "#61afef",
    green: "#98c379",
    magenta: "#c678dd",
    red: "#e06c75",
    yellow: "#e5c07b",
    orange: "#d19a66",
    comment: "#5c6370",
    base: "vs-dark",
    selection: "#61afef40",
    terminal_palette: [
        "#21252b", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#abb2bf",
        "#5c6370", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#ffffff",
    ],
};

/// Ayu Dark — minimal dark theme with warm accent colors.
pub static AYU_DARK: ThemeColors = ThemeColors {
    bg: "#080b10",
    bg_dark: "#05070b",
    bg_highlight: "#101420",
    bg_surface: "#020305",
    border: "#0b0e16",
    accent: "#73b8ff",
    fg: "#bfbdb6",
    fg_dark: "#797f8e",
    cyan: "#73b8ff",
    blue: "#59c2ff",
    green: "#aad94c",
    magenta: "#d2a6ff",
    red: "#f07178",
    yellow: "#ffb454",
    orange: "#ff8f40",
    comment: "#565b66",
    base: "vs-dark",
    selection: "#59c2ff30",
    terminal_palette: [
        "#07090d", "#f07178", "#aad94c", "#ffb454", "#59c2ff", "#d2a6ff", "#73b8ff", "#bfbdb6",
        "#565b66", "#f07178", "#aad94c", "#ffb454", "#59c2ff", "#d2a6ff", "#73b8ff", "#ffffff",
    ],
};

/// Everforest Dark — soft green tones inspired by nature.
pub static EVERFOREST_DARK: ThemeColors = ThemeColors {
    bg: "#272e34",
    bg_dark: "#21282d",
    bg_highlight: "#353f44",
    bg_surface: "#1a2025",
    border: "#2c3338",
    accent: "#83c092",
    fg: "#d3c6aa",
    fg_dark: "#9da9a0",
    cyan: "#83c092",
    blue: "#7fbbb3",
    green: "#a7c080",
    magenta: "#d699b6",
    red: "#e67e80",
    yellow: "#dbbc7f",
    orange: "#e69875",
    comment: "#7a8478",
    base: "vs-dark",
    selection: "#7fbbb340",
    terminal_palette: [
        "#272e33", "#e67e80", "#a7c080", "#dbbc7f", "#7fbbb3", "#d699b6", "#83c092", "#9da9a0",
        "#7a8478", "#e67e80", "#a7c080", "#dbbc7f", "#7fbbb3", "#d699b6", "#83c092", "#d3c6aa",
    ],
};

/// GitHub Dark — GitHub's official dark theme.
pub static GITHUB_DARK: ThemeColors = ThemeColors {
    bg: "#090d13",
    bg_dark: "#050709",
    bg_highlight: "#131820",
    bg_surface: "#010203",
    border: "#0a0f15",
    accent: "#79c0ff",
    fg: "#e6edf3",
    fg_dark: "#8b949e",
    cyan: "#79c0ff",
    blue: "#79c0ff",
    green: "#7ee787",
    magenta: "#d2a8ff",
    red: "#ff7b72",
    yellow: "#ffa657",
    orange: "#f0883e",
    comment: "#8b949e",
    base: "vs-dark",
    selection: "#79c0ff30",
    terminal_palette: [
        "#010409", "#ff7b72", "#7ee787", "#ffa657", "#79c0ff", "#d2a8ff", "#a5d6ff", "#8b949e",
        "#6e7681", "#ffa198", "#7ee787", "#ffa657", "#79c0ff", "#d2a8ff", "#a5d6ff", "#e6edf3",
    ],
};

/// Monokai Pro — iconic warm dark theme with vibrant syntax colors.
pub static MONOKAI_PRO: ThemeColors = ThemeColors {
    bg: "#272428",
    bg_dark: "#1e1b1e",
    bg_highlight: "#38363a",
    bg_surface: "#161416",
    border: "#2c292c",
    accent: "#78dce8",
    fg: "#fcfcfa",
    fg_dark: "#939293",
    cyan: "#78dce8",
    blue: "#78dce8",
    green: "#a9dc76",
    magenta: "#ab9df2",
    red: "#ff6188",
    yellow: "#ffd866",
    orange: "#fc9867",
    comment: "#727072",
    base: "vs-dark",
    selection: "#ab9df240",
    terminal_palette: [
        "#221f22", "#ff6188", "#a9dc76", "#ffd866", "#78dce8", "#ab9df2", "#78dce8", "#939293",
        "#727072", "#ff6188", "#a9dc76", "#ffd866", "#78dce8", "#ab9df2", "#78dce8", "#fcfcfa",
    ],
};

/// Palenight — Material Design-inspired dark theme with purple tones.
pub static PALENIGHT: ThemeColors = ThemeColors {
    bg: "#232738",
    bg_dark: "#1a1d28",
    bg_highlight: "#2d3246",
    bg_surface: "#131520",
    border: "#212435",
    accent: "#89ddff",
    fg: "#a6accd",
    fg_dark: "#868bab",
    cyan: "#89ddff",
    blue: "#82aaff",
    green: "#c3e88d",
    magenta: "#c792ea",
    red: "#f07178",
    yellow: "#ffcb6b",
    orange: "#f78c6c",
    comment: "#676e95",
    base: "vs-dark",
    selection: "#82aaff35",
    terminal_palette: [
        "#1b1e2b", "#f07178", "#c3e88d", "#ffcb6b", "#82aaff", "#c792ea", "#89ddff", "#676e95",
        "#676e95", "#f07178", "#c3e88d", "#ffcb6b", "#82aaff", "#c792ea", "#89ddff", "#a6accd",
    ],
};

/// Solarized Light — precision-engineered light palette with balanced contrast.
pub static SOLARIZED_LIGHT: ThemeColors = ThemeColors {
    bg: "#fdf6e3",
    bg_dark: "#f0eadb",
    bg_highlight: "#eee8d5",
    bg_surface: "#e2dcc8",
    border: "#d3ccb6",
    accent: "#217e77",
    fg: "#657b83",
    fg_dark: "#576464",
    cyan: "#217e77",
    blue: "#1d6da3",
    green: "#859900",
    magenta: "#d33682",
    red: "#dc322f",
    yellow: "#b58900",
    orange: "#cb4b16",
    comment: "#93a1a1",
    base: "vs",
    selection: "#268bd230",
    terminal_palette: [
        "#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198", "#eee8d5",
        "#002b36", "#cb4b16", "#586e75", "#657b83", "#839496", "#6c71c4", "#93a1a1", "#fdf6e3",
    ],
};

/// Catppuccin Latte — warm pastel light theme.
pub static CATPPUCCIN_LATTE: ThemeColors = ThemeColors {
    bg: "#eff1f5",
    bg_dark: "#e4e7ed",
    bg_highlight: "#dce0e8",
    bg_surface: "#d5d9e2",
    border: "#c9cdd6",
    accent: "#137a80",
    fg: "#4c4f69",
    fg_dark: "#65677c",
    cyan: "#137a80",
    blue: "#1559de",
    green: "#40a02b",
    magenta: "#8839ef",
    red: "#d20f39",
    yellow: "#df8e1d",
    orange: "#fe640b",
    comment: "#9ca0b0",
    base: "vs",
    selection: "#1e66f525",
    terminal_palette: [
        "#5c5f77", "#d20f39", "#40a02b", "#df8e1d", "#1e66f5", "#8839ef", "#179299", "#acb0be",
        "#6c6f85", "#d20f39", "#40a02b", "#df8e1d", "#1e66f5", "#8839ef", "#179299", "#4c4f69",
    ],
};

/// GitHub Light — GitHub's official light theme.
pub static GITHUB_LIGHT: ThemeColors = ThemeColors {
    bg: "#ffffff",
    bg_dark: "#f4f6f8",
    bg_highlight: "#eef0f3",
    bg_surface: "#e6e9ed",
    border: "#d4dae0",
    accent: "#0969da",
    fg: "#1f2328",
    fg_dark: "#656d76",
    cyan: "#0a3069",
    blue: "#0969da",
    green: "#1a7f37",
    magenta: "#8250df",
    red: "#cf222e",
    yellow: "#9a6700",
    orange: "#bc4c00",
    comment: "#6e7781",
    base: "vs",
    selection: "#0969da25",
    terminal_palette: [
        "#24292f", "#cf222e", "#1a7f37", "#9a6700", "#0969da", "#8250df", "#0a3069", "#6e7781",
        "#57606a", "#a40e26", "#2da44e", "#bf8700", "#218bff", "#a475f9", "#0a3069", "#1f2328",
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
        "everforest-dark" | "everforest_dark" | "everforestdark" => &EVERFOREST_DARK,
        "github-dark" | "github_dark" | "githubdark" => &GITHUB_DARK,
        "monokai-pro" | "monokai_pro" | "monokaipro" => &MONOKAI_PRO,
        "palenight" => &PALENIGHT,
        "solarized-light" | "solarized_light" | "solarizedlight" => &SOLARIZED_LIGHT,
        "catppuccin-latte" | "catppuccin_latte" | "catppuccinlatte" => &CATPPUCCIN_LATTE,
        "github-light" | "github_light" | "githublight" => &GITHUB_LIGHT,
        _ => &NORD,
    }
}

/// Convert a theme ID like `"tokyo-night-storm"` to a display name like `"Tokyo Night Storm"`.
pub fn theme_display_name(id: &str) -> String {
    match id {
        "rose-pine" => "Rosé Pine".to_string(),
        "catppuccin-mocha" => "Catppuccin Mocha".to_string(),
        "catppuccin-latte" => "Catppuccin Latte".to_string(),
        "github-dark" => "GitHub Dark".to_string(),
        "github-light" => "GitHub Light".to_string(),
        "monokai-pro" => "Monokai Pro".to_string(),
        _ => id
            .split('-')
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    Some(c) => {
                        let upper: String = c.to_uppercase().collect();
                        format!("{}{}", upper, chars.as_str())
                    }
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
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
        "everforest-dark",
        "github-dark",
        "monokai-pro",
        "palenight",
        "solarized-light",
        "catppuccin-latte",
        "github-light",
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
        /* --- Global font --- */
        window, popover, menu {{
            font-family: 'Inter', sans-serif;
        }}

        /* --- Sidebar --- */
        .sidebar {{
            background-color: {bg_dark};
            border-right: 1px solid {border};
        }}
        .sidebar-switcher {{
            padding: 6px 8px;
            background-color: {bg_surface};
            border-bottom: 1px solid {border};
        }}
        .sidebar-tab {{
            border-radius: 6px;
            padding: 4px 14px;
            font-size: 12px;
            font-weight: 600;
            color: {fg_dark};
            background: transparent;
            border: none;
            box-shadow: none;
            min-height: 26px;
            transition: color 0.15s ease, background-color 0.15s ease;
        }}
        .sidebar-tab:hover {{
            color: {fg};
            background-color: {bg_highlight};
        }}
        .sidebar-tab-active {{
            color: {accent};
            background-color: alpha({accent}, 0.15);
        }}
        .sidebar-project-header {{
            padding: 4px 8px;
            background-color: {bg_surface};
        }}
        .sidebar-project-name {{
            font-size: 11px;
            font-weight: bold;
            letter-spacing: 1px;
            color: {fg_dark};
        }}
        .sidebar-toolbar-btn {{
            min-width: 24px;
            min-height: 24px;
            padding: 2px;
            border-radius: 4px;
            transition: background-color 0.15s ease;
        }}
        .sidebar-toolbar-btn:hover {{
            background-color: {bg_highlight};
        }}
        .file-tree {{
            background-color: transparent;
        }}
        .file-tree row {{
            padding: 0;
            border-radius: 4px;
            margin: 0 4px;
            transition: background-color 0.1s ease;
        }}
        .file-tree row:hover {{
            background-color: alpha({accent}, 0.08);
        }}
        .file-tree row:selected {{
            background-color: alpha({accent}, 0.15);
        }}
        .sidebar-indent-guide {{
            color: alpha({border}, 0.4);
        }}
        .file-entry {{
            padding: 0px 8px;
            min-height: 28px;
        }}
        .file-entry-dir {{
            color: {fg};
        }}
        .file-entry-file {{
            color: {fg};
        }}
        .git-badge {{
            font-size: 11px;
            font-weight: 600;
            font-family: 'JetBrains Mono', monospace;
            margin-right: 4px;
            min-width: 14px;
        }}
        .git-modified {{
            color: {yellow};
        }}
        .git-added {{
            color: {green};
        }}
        .git-untracked {{
            color: {green};
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
            color: {green};
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
        .file-entry-git-ignored {{
            color: {fg_dark};
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
        .search-result {{
            transition: background-color 0.1s ease;
        }}
        .search-result:hover {{
            background-color: alpha({accent}, 0.08);
        }}
        .search-result-path {{
            font-size: 11px;
            color: {fg_dark};
        }}
        .search-result-line {{
            font-size: 12px;
            color: {fg};
        }}
        /* --- Split pane dividers --- */
        paned > separator {{
            background-color: {border};
            min-width: 1px;
            min-height: 1px;
            transition: min-width 0.15s ease, min-height 0.15s ease, background-color 0.15s ease;
        }}
        paned > separator:hover {{
            background-color: {accent};
            min-width: 3px;
            min-height: 3px;
        }}
        /* --- Status bar --- */
        .status-bar {{
            background-color: {bg_surface};
            padding: 2px 12px;
            min-height: 28px;
            border-top: 1px solid {border};
        }}
        .status-bar-separator {{
            background-color: alpha({border}, 0.4);
            min-width: 1px;
            margin: 5px 6px;
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
        .status-bar .status-bar-preview-btn {{
            min-height: 16px;
            min-width: 0;
            padding: 0 8px;
            margin: 3px 4px 3px 8px;
            border-radius: 3px;
            background: none;
            border: 1px solid {green};
            box-shadow: none;
            cursor: pointer;
        }}
        .status-bar .status-bar-preview-btn label {{
            font-size: 11px;
            color: {green};
        }}
        .status-bar .status-bar-preview-btn:hover {{
            background: alpha({green}, 0.1);
        }}
        .status-bar .status-bar-preview-btn.previewing {{
            background: {green};
            border-color: {green};
        }}
        .status-bar .status-bar-preview-btn.previewing label {{
            color: {bg_dark};
        }}
        .status-bar .status-bar-preview-btn.previewing:hover {{
            background: alpha({green}, 0.85);
        }}
        .status-bar .status-bar-update-btn {{
            min-height: 16px;
            min-width: 0;
            padding: 0 8px;
            border-radius: 3px;
            border: none;
            background: none;
            box-shadow: none;
            cursor: pointer;
        }}
        .status-bar .status-bar-update-btn label {{
            font-size: 11px;
            color: {yellow};
        }}
        .status-bar .status-bar-update-btn:hover {{
            background: alpha({yellow}, 0.1);
        }}
        /* --- Terminal --- */
        vte-terminal {{
            padding: 8px 12px;
        }}
        /* --- Header bar --- */
        headerbar {{
            background-color: {bg_surface};
            box-shadow: none;
            min-height: 38px;
            border-bottom: 1px solid {border};
        }}
        headerbar button {{
            color: {fg_dark};
            transition: color 0.15s ease, background-color 0.15s ease;
        }}
        headerbar button:hover {{
            color: {accent};
            background-color: {bg_highlight};
        }}
        tabbar {{
            background-color: {bg_surface};
            border-bottom: 1px solid {border};
        }}
        tabbar revealer > box {{
            box-shadow: none;
            padding: 0;
        }}
        tabbar tabbox {{
            background-color: {bg_surface};
        }}
        tabbar tab {{
            min-height: 34px;
            padding: 0 10px;
            background-color: {bg_surface};
            color: {fg_dark};
            border-radius: 0;
            border-bottom: 2px solid transparent;
            cursor: pointer;
            transition: background-color 0.15s ease, color 0.15s ease, border-color 0.15s ease;
        }}
        tabbar tab:selected {{
            background-color: {bg};
            color: {accent};
            border-bottom: 2px solid {accent};
        }}
        tabbar tab:hover:not(:selected) {{
            background-color: {bg_highlight};
            color: {fg};
        }}
        tabbar tab image {{
            margin-right: 2px;
        }}
        tabbar tab label {{
            font-size: 13px;
            font-weight: 500;
        }}
        window.background {{
            background-color: {bg};
        }}
        /* --- Quick open --- */
        .quick-open {{
            background-color: {bg};
            border-radius: 8px;
            border: 1px solid {border};
        }}
        .quick-open entry {{
            margin: 8px;
            font-size: 14px;
        }}
        .quick-open list row {{
            border-radius: 4px;
            margin: 0 4px;
            transition: background-color 0.1s ease;
        }}
        .quick-open list row:hover {{
            background-color: alpha({accent}, 0.08);
        }}
        .quick-open list row:selected {{
            background-color: alpha({accent}, 0.15);
        }}
        .quick-open list row label {{
            padding: 6px 12px;
            color: {fg};
        }}
        /* --- Terminal search bar --- */
        .terminal-search-bar {{
            background-color: {bg_dark};
            padding: 4px 8px;
            border-bottom: 1px solid {border};
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
            color: {fg_dark};
            font-size: 11px;
            margin: 0 4px;
        }}
        /* --- Scrollbars --- */
        scrollbar slider {{
            background-color: {border};
            border-radius: 4px;
            min-width: 4px;
            min-height: 4px;
            transition: background-color 0.15s ease;
        }}
        scrollbar slider:hover {{
            background-color: {comment};
        }}
        /* --- Project search panel --- */
        .project-search-panel {{
            background-color: {bg_dark};
            border-top: 1px solid {border};
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
            color: {fg_dark};
            padding: 2px 8px;
        }}
        .project-search-results {{
            background-color: transparent;
        }}
        .project-search-results row {{
            transition: background-color 0.1s ease;
        }}
        .project-search-results row:hover {{
            background-color: alpha({accent}, 0.08);
        }}
        .project-search-results row:selected {{
            background-color: alpha({accent}, 0.15);
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
            color: {fg_dark};
            font-size: 11px;
        }}
        .project-search-match {{
            padding: 2px 8px 2px 16px;
        }}
        .project-search-line-num {{
            color: {fg_dark};
            font-size: 11px;
            font-family: 'JetBrains Mono', monospace;
        }}
        .project-search-line-content {{
            color: {fg};
            font-size: 12px;
            font-family: 'JetBrains Mono', monospace;
        }}
        "#,
        bg_dark = theme.bg_dark,
        bg = theme.bg,
        bg_highlight = theme.bg_highlight,
        bg_surface = theme.bg_surface,
        border = theme.border,
        accent = theme.accent,
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
        gtk4::STYLE_PROVIDER_PRIORITY_USER,
    );
    provider
}
