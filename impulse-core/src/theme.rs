use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// TOML schema types (what theme authors write)
// ---------------------------------------------------------------------------

/// Top-level structure of a `.toml` theme file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeFile {
    pub name: String,
    /// `"dark"` or `"light"`.
    pub variant: String,
    pub palette: ThemePalette,
    #[serde(default)]
    pub ui: SemanticUI,
    #[serde(default)]
    pub syntax: SemanticSyntax,
    pub terminal: Option<TerminalPalette>,
}

/// The core color palette — 10 required hues plus 4 optional derived shades.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemePalette {
    pub bg: String,
    pub fg: String,
    pub accent: String,
    pub red: String,
    pub orange: String,
    pub yellow: String,
    pub green: String,
    pub cyan: String,
    pub blue: String,
    pub magenta: String,
    /// Darker surface shade — derived from `bg` if omitted.
    #[serde(default)]
    pub surface: Option<String>,
    /// Lighter overlay shade — derived from `bg` if omitted.
    #[serde(default)]
    pub overlay: Option<String>,
    /// Muted foreground — derived from `fg` if omitted.
    #[serde(default)]
    pub muted: Option<String>,
    /// Subtle foreground (comments) — derived from `fg` if omitted.
    #[serde(default)]
    pub subtle: Option<String>,
}

/// Semantic UI color overrides. All fields optional — derived from palette.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SemanticUI {
    pub bg_dark: Option<String>,
    pub bg_highlight: Option<String>,
    pub bg_surface: Option<String>,
    pub border: Option<String>,
    pub fg_muted: Option<String>,
    pub fg_comment: Option<String>,
    pub selection: Option<String>,
    pub cursor: Option<String>,
    pub git_added: Option<String>,
    pub git_modified: Option<String>,
    pub git_deleted: Option<String>,
    pub git_renamed: Option<String>,
    pub git_conflict: Option<String>,
    pub git_ignored: Option<String>,
}

/// Semantic syntax color overrides. All fields optional — derived from palette.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SemanticSyntax {
    pub keyword: Option<String>,
    pub function: Option<String>,
    pub r#type: Option<String>,
    pub string: Option<String>,
    pub number: Option<String>,
    pub constant: Option<String>,
    pub comment: Option<String>,
    pub operator: Option<String>,
    pub tag: Option<String>,
    pub attribute: Option<String>,
    pub variable: Option<String>,
    pub delimiter: Option<String>,
    pub escape: Option<String>,
    pub regexp: Option<String>,
    pub link: Option<String>,
}

/// 16-color terminal palette, compatible with Alacritty/Ghostty/Kitty.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalPalette {
    pub black: String,
    pub red: String,
    pub green: String,
    pub yellow: String,
    pub blue: String,
    pub magenta: String,
    pub cyan: String,
    pub white: String,
    pub bright_black: String,
    pub bright_red: String,
    pub bright_green: String,
    pub bright_yellow: String,
    pub bright_blue: String,
    pub bright_magenta: String,
    pub bright_cyan: String,
    pub bright_white: String,
}

// ---------------------------------------------------------------------------
// Resolved theme (what consumers use — no Options, all fields populated)
// ---------------------------------------------------------------------------

/// Fully resolved theme with every field populated. This is the type that
/// frontends, the Monaco converter, and the FFI layer consume.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedTheme {
    pub id: String,
    pub name: String,
    pub is_light: bool,
    // UI backgrounds
    pub bg: String,
    pub bg_dark: String,
    pub bg_highlight: String,
    pub bg_surface: String,
    pub border: String,
    // UI foregrounds
    pub fg: String,
    pub fg_muted: String,
    pub fg_comment: String,
    pub accent: String,
    pub selection: String,
    pub cursor: String,
    // Raw palette hues (for icons, git badges, status bar accents)
    pub red: String,
    pub orange: String,
    pub yellow: String,
    pub green: String,
    pub cyan: String,
    pub blue: String,
    pub magenta: String,
    // Git indicators
    pub git_added: String,
    pub git_modified: String,
    pub git_deleted: String,
    pub git_renamed: String,
    pub git_conflict: String,
    pub git_ignored: String,
    // Syntax (semantic names)
    pub syntax_keyword: String,
    pub syntax_function: String,
    pub syntax_type: String,
    pub syntax_string: String,
    pub syntax_number: String,
    pub syntax_constant: String,
    pub syntax_comment: String,
    pub syntax_operator: String,
    pub syntax_tag: String,
    pub syntax_attribute: String,
    pub syntax_variable: String,
    pub syntax_delimiter: String,
    pub syntax_escape: String,
    pub syntax_regexp: String,
    pub syntax_link: String,
    // Terminal
    pub terminal_fg: String,
    pub terminal_bg: String,
    pub terminal_palette: [String; 16],
}

// ---------------------------------------------------------------------------
// HSL color math
// ---------------------------------------------------------------------------

struct Hsl {
    h: f64,
    s: f64,
    l: f64,
}

fn hex_to_rgb(hex: &str) -> (u8, u8, u8) {
    let hex = hex.trim_start_matches('#');
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
    (r, g, b)
}

fn rgb_to_hsl(r: u8, g: u8, b: u8) -> Hsl {
    let r = r as f64 / 255.0;
    let g = g as f64 / 255.0;
    let b = b as f64 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;

    if (max - min).abs() < f64::EPSILON {
        return Hsl { h: 0.0, s: 0.0, l };
    }

    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };

    let h = if (max - r).abs() < f64::EPSILON {
        let mut h = (g - b) / d;
        if g < b {
            h += 6.0;
        }
        h
    } else if (max - g).abs() < f64::EPSILON {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };

    Hsl {
        h: h * 60.0,
        s,
        l,
    }
}

fn hsl_to_rgb(hsl: &Hsl) -> (u8, u8, u8) {
    if hsl.s.abs() < f64::EPSILON {
        let v = (hsl.l * 255.0).round() as u8;
        return (v, v, v);
    }

    let q = if hsl.l < 0.5 {
        hsl.l * (1.0 + hsl.s)
    } else {
        hsl.l + hsl.s - hsl.l * hsl.s
    };
    let p = 2.0 * hsl.l - q;
    let h = hsl.h / 360.0;

    let hue_to_rgb = |p: f64, q: f64, mut t: f64| -> f64 {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            return p + (q - p) * 6.0 * t;
        }
        if t < 1.0 / 2.0 {
            return q;
        }
        if t < 2.0 / 3.0 {
            return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
        }
        p
    };

    let r = (hue_to_rgb(p, q, h + 1.0 / 3.0) * 255.0).round() as u8;
    let g = (hue_to_rgb(p, q, h) * 255.0).round() as u8;
    let b = (hue_to_rgb(p, q, h - 1.0 / 3.0) * 255.0).round() as u8;
    (r, g, b)
}

fn rgb_to_hex(r: u8, g: u8, b: u8) -> String {
    format!("#{:02x}{:02x}{:02x}", r, g, b)
}

/// Shift the lightness of a hex color by `delta` (range: -1.0 to 1.0).
fn shift_lightness(hex: &str, delta: f64) -> String {
    let (r, g, b) = hex_to_rgb(hex);
    let mut hsl = rgb_to_hsl(r, g, b);
    hsl.l = (hsl.l + delta).clamp(0.0, 1.0);
    let (r, g, b) = hsl_to_rgb(&hsl);
    rgb_to_hex(r, g, b)
}

/// Desaturate/mute a color by reducing saturation and shifting lightness toward middle.
fn mute_color(hex: &str, saturation_factor: f64, lightness_target: f64) -> String {
    let (r, g, b) = hex_to_rgb(hex);
    let mut hsl = rgb_to_hsl(r, g, b);
    hsl.s *= saturation_factor;
    hsl.l = hsl.l + (lightness_target - hsl.l) * 0.3;
    let (r, g, b) = hsl_to_rgb(&hsl);
    rgb_to_hex(r, g, b)
}

// ---------------------------------------------------------------------------
// Theme resolution
// ---------------------------------------------------------------------------

/// Parse a TOML theme string into a `ThemeFile`.
pub fn parse_theme(toml_str: &str) -> Result<ThemeFile, String> {
    toml::from_str(toml_str).map_err(|e| format!("Failed to parse theme TOML: {e}"))
}

/// Resolve a parsed `ThemeFile` into a fully-populated `ResolvedTheme`.
/// Fills in all defaults and derived values from the palette.
/// The `id` parameter is the lookup key (e.g. `"rose-pine"`, not the display
/// name `"Rosé Pine"`). It is stored so consumers can look up related
/// resources (like the Monaco theme) without a display-name → ID mapping.
pub fn resolve_theme(id: &str, tf: &ThemeFile) -> ResolvedTheme {
    let p = &tf.palette;
    let is_light = tf.variant == "light";

    // Derive background layers
    let bg_surface = tf
        .ui
        .bg_surface
        .clone()
        .or_else(|| p.surface.clone())
        .unwrap_or_else(|| {
            if is_light {
                shift_lightness(&p.bg, 0.08)
            } else {
                shift_lightness(&p.bg, -0.10)
            }
        });
    let bg_dark = tf.ui.bg_dark.clone().unwrap_or_else(|| {
        if is_light {
            shift_lightness(&p.bg, 0.04)
        } else {
            shift_lightness(&p.bg, -0.05)
        }
    });
    let bg_highlight = tf.ui.bg_highlight.clone().unwrap_or_else(|| {
        if is_light {
            shift_lightness(&p.bg, -0.05)
        } else {
            shift_lightness(&p.bg, 0.08)
        }
    });
    let border = tf
        .ui
        .border
        .clone()
        .or_else(|| p.overlay.clone())
        .unwrap_or_else(|| {
            if is_light {
                shift_lightness(&p.bg, -0.08)
            } else {
                shift_lightness(&p.bg, 0.04)
            }
        });

    // Derive foreground shades
    let fg_muted = tf
        .ui
        .fg_muted
        .clone()
        .or_else(|| p.muted.clone())
        .unwrap_or_else(|| mute_color(&p.fg, 0.6, 0.55));
    let fg_comment = tf
        .ui
        .fg_comment
        .clone()
        .or_else(|| p.subtle.clone())
        .unwrap_or_else(|| {
            let target = if is_light { 0.60 } else { 0.40 };
            mute_color(&p.fg, 0.4, target)
        });

    // Selection & cursor
    let selection = tf
        .ui
        .selection
        .clone()
        .unwrap_or_else(|| format!("{}40", p.accent));
    let cursor = tf.ui.cursor.clone().unwrap_or_else(|| p.accent.clone());

    // Git indicators
    let git_added = tf
        .ui
        .git_added
        .clone()
        .unwrap_or_else(|| p.green.clone());
    let git_modified = tf
        .ui
        .git_modified
        .clone()
        .unwrap_or_else(|| p.yellow.clone());
    let git_deleted = tf
        .ui
        .git_deleted
        .clone()
        .unwrap_or_else(|| p.red.clone());
    let git_renamed = tf
        .ui
        .git_renamed
        .clone()
        .unwrap_or_else(|| p.blue.clone());
    let git_conflict = tf
        .ui
        .git_conflict
        .clone()
        .unwrap_or_else(|| p.orange.clone());
    let git_ignored = tf
        .ui
        .git_ignored
        .clone()
        .unwrap_or_else(|| fg_muted.clone());

    // Syntax colors
    let syntax_keyword = tf
        .syntax
        .keyword
        .clone()
        .unwrap_or_else(|| p.magenta.clone());
    let syntax_function = tf
        .syntax
        .function
        .clone()
        .unwrap_or_else(|| p.blue.clone());
    let syntax_type = tf
        .syntax
        .r#type
        .clone()
        .unwrap_or_else(|| p.yellow.clone());
    let syntax_string = tf
        .syntax
        .string
        .clone()
        .unwrap_or_else(|| p.green.clone());
    let syntax_number = tf
        .syntax
        .number
        .clone()
        .unwrap_or_else(|| p.orange.clone());
    let syntax_constant = tf
        .syntax
        .constant
        .clone()
        .unwrap_or_else(|| p.orange.clone());
    let syntax_comment = tf
        .syntax
        .comment
        .clone()
        .unwrap_or_else(|| fg_comment.clone());
    let syntax_operator = tf
        .syntax
        .operator
        .clone()
        .unwrap_or_else(|| p.cyan.clone());
    let syntax_tag = tf.syntax.tag.clone().unwrap_or_else(|| p.red.clone());
    let syntax_attribute = tf
        .syntax
        .attribute
        .clone()
        .unwrap_or_else(|| p.yellow.clone());
    let syntax_variable = tf
        .syntax
        .variable
        .clone()
        .unwrap_or_else(|| p.fg.clone());
    let syntax_delimiter = tf
        .syntax
        .delimiter
        .clone()
        .unwrap_or_else(|| fg_muted.clone());
    let syntax_escape = tf
        .syntax
        .escape
        .clone()
        .unwrap_or_else(|| p.orange.clone());
    let syntax_regexp = tf
        .syntax
        .regexp
        .clone()
        .unwrap_or_else(|| p.red.clone());
    let syntax_link = tf
        .syntax
        .link
        .clone()
        .unwrap_or_else(|| p.blue.clone());

    // Terminal palette
    let (terminal_fg, terminal_bg, terminal_palette) = if let Some(ref tp) = tf.terminal {
        (
            p.fg.clone(),
            p.bg.clone(),
            [
                tp.black.clone(),
                tp.red.clone(),
                tp.green.clone(),
                tp.yellow.clone(),
                tp.blue.clone(),
                tp.magenta.clone(),
                tp.cyan.clone(),
                tp.white.clone(),
                tp.bright_black.clone(),
                tp.bright_red.clone(),
                tp.bright_green.clone(),
                tp.bright_yellow.clone(),
                tp.bright_blue.clone(),
                tp.bright_magenta.clone(),
                tp.bright_cyan.clone(),
                tp.bright_white.clone(),
            ],
        )
    } else {
        // Derive terminal palette from theme palette
        (
            p.fg.clone(),
            p.bg.clone(),
            [
                shift_lightness(&p.bg, 0.10),
                p.red.clone(),
                p.green.clone(),
                p.yellow.clone(),
                p.blue.clone(),
                p.magenta.clone(),
                p.cyan.clone(),
                fg_muted.clone(),
                fg_comment.clone(),
                p.red.clone(),
                p.green.clone(),
                p.yellow.clone(),
                p.blue.clone(),
                p.magenta.clone(),
                p.cyan.clone(),
                p.fg.clone(),
            ],
        )
    };

    ResolvedTheme {
        id: id.to_string(),
        name: tf.name.clone(),
        is_light,
        bg: p.bg.clone(),
        bg_dark,
        bg_highlight,
        bg_surface,
        border,
        fg: p.fg.clone(),
        fg_muted,
        fg_comment,
        accent: p.accent.clone(),
        selection,
        cursor,
        red: p.red.clone(),
        orange: p.orange.clone(),
        yellow: p.yellow.clone(),
        green: p.green.clone(),
        cyan: p.cyan.clone(),
        blue: p.blue.clone(),
        magenta: p.magenta.clone(),
        git_added,
        git_modified,
        git_deleted,
        git_renamed,
        git_conflict,
        git_ignored,
        syntax_keyword,
        syntax_function,
        syntax_type,
        syntax_string,
        syntax_number,
        syntax_constant,
        syntax_comment,
        syntax_operator,
        syntax_tag,
        syntax_attribute,
        syntax_variable,
        syntax_delimiter,
        syntax_escape,
        syntax_regexp,
        syntax_link,
        terminal_fg,
        terminal_bg,
        terminal_palette,
    }
}

// ---------------------------------------------------------------------------
// Built-in themes
// ---------------------------------------------------------------------------

const BUILTIN_THEMES: &[(&str, &str)] = &[
    ("kanagawa", include_str!("../themes/kanagawa.toml")),
    ("rose-pine", include_str!("../themes/rose-pine.toml")),
    ("nord", include_str!("../themes/nord.toml")),
    ("gruvbox", include_str!("../themes/gruvbox.toml")),
    ("tokyo-night", include_str!("../themes/tokyo-night.toml")),
    (
        "tokyo-night-storm",
        include_str!("../themes/tokyo-night-storm.toml"),
    ),
    (
        "catppuccin-mocha",
        include_str!("../themes/catppuccin-mocha.toml"),
    ),
    ("dracula", include_str!("../themes/dracula.toml")),
    (
        "solarized-dark",
        include_str!("../themes/solarized-dark.toml"),
    ),
    ("one-dark", include_str!("../themes/one-dark.toml")),
    ("ayu-dark", include_str!("../themes/ayu-dark.toml")),
    (
        "everforest-dark",
        include_str!("../themes/everforest-dark.toml"),
    ),
    ("github-dark", include_str!("../themes/github-dark.toml")),
    ("monokai-pro", include_str!("../themes/monokai-pro.toml")),
    ("palenight", include_str!("../themes/palenight.toml")),
    (
        "solarized-light",
        include_str!("../themes/solarized-light.toml"),
    ),
    (
        "catppuccin-latte",
        include_str!("../themes/catppuccin-latte.toml"),
    ),
    ("github-light", include_str!("../themes/github-light.toml")),
];

/// Return the list of built-in theme IDs in display order.
pub fn builtin_theme_names() -> Vec<&'static str> {
    BUILTIN_THEMES.iter().map(|(id, _)| *id).collect()
}

/// Load and resolve a built-in theme by ID.
pub fn builtin_theme(name: &str) -> Option<ResolvedTheme> {
    let normalized = normalize_theme_id(name);
    BUILTIN_THEMES
        .iter()
        .find(|(id, _)| *id == normalized)
        .and_then(|(id, toml_str)| {
            parse_theme(toml_str)
                .map_err(|e| log::warn!("Failed to parse built-in theme '{name}': {e}"))
                .ok()
                .map(|tf| (id, tf))
        })
        .map(|(id, tf)| resolve_theme(id, &tf))
}

/// Discover user themes from the platform config directory.
///
/// Returns `(theme_id, file_path)` pairs. The theme ID is the filename stem.
pub fn discover_user_themes() -> Vec<(String, std::path::PathBuf)> {
    let dir = match user_themes_dir() {
        Some(d) if d.is_dir() => d,
        _ => return vec![],
    };

    let mut themes = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    themes.push((stem.to_string(), path));
                }
            }
        }
    }
    themes.sort_by(|a, b| a.0.cmp(&b.0));
    themes
}

/// Load a user theme from a file path.
/// The theme ID is derived from the filename stem (e.g. `my-theme.toml` → `"my-theme"`).
pub fn load_user_theme(path: &std::path::Path) -> Result<ResolvedTheme, String> {
    let id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("custom")
        .to_string();
    let contents =
        std::fs::read_to_string(path).map_err(|e| format!("Failed to read theme file: {e}"))?;
    let tf = parse_theme(&contents)?;
    Ok(resolve_theme(&id, &tf))
}

/// Return all available theme names: built-in first, then user themes.
pub fn available_themes() -> Vec<String> {
    let mut names: Vec<String> = builtin_theme_names()
        .into_iter()
        .map(String::from)
        .collect();

    for (id, _) in discover_user_themes() {
        if !names.contains(&id) {
            names.push(id);
        }
    }
    names
}

/// Resolve a theme by name. Checks user themes first (allows overrides),
/// then built-in themes, then falls back to Nord.
pub fn get_theme(name: &str) -> ResolvedTheme {
    let normalized = normalize_theme_id(name);

    // Check user themes first
    for (id, path) in discover_user_themes() {
        if id == normalized {
            match load_user_theme(&path) {
                Ok(theme) => return theme,
                Err(e) => {
                    log::warn!("Failed to load user theme '{id}': {e}");
                    break;
                }
            }
        }
    }

    // Built-in themes
    if let Some(theme) = builtin_theme(&normalized) {
        return theme;
    }

    // Fallback
    builtin_theme("nord").expect("Nord theme must always be available")
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
                        format!("{upper}{}", chars.as_str())
                    }
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// Serialize a `ResolvedTheme` to JSON.
pub fn theme_to_json(theme: &ResolvedTheme) -> String {
    serde_json::to_string(theme).unwrap_or_else(|_| "{}".to_string())
}

/// Deserialize a `ResolvedTheme` from JSON.
pub fn theme_from_json(json: &str) -> Result<ResolvedTheme, String> {
    serde_json::from_str(json).map_err(|e| format!("Failed to parse theme JSON: {e}"))
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Normalize theme name variants (underscore, no-separator) to canonical kebab-case ID.
fn normalize_theme_id(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    // Handle common alternative formats: underscores, no separators
    match lower.as_str() {
        "rose_pine" | "rosepine" => "rose-pine".to_string(),
        "gruvbox_dark" | "gruvbox-dark" => "gruvbox".to_string(),
        "tokyo_night" | "tokyonight" => "tokyo-night".to_string(),
        "tokyo_night_storm" | "tokyonightstorm" | "tokyo-night-storm" => {
            "tokyo-night-storm".to_string()
        }
        "catppuccin_mocha" | "catppuccinmocha" => "catppuccin-mocha".to_string(),
        "solarized_dark" | "solarizeddark" => "solarized-dark".to_string(),
        "one_dark" | "onedark" => "one-dark".to_string(),
        "ayu_dark" | "ayudark" => "ayu-dark".to_string(),
        "everforest_dark" | "everforestdark" => "everforest-dark".to_string(),
        "github_dark" | "githubdark" => "github-dark".to_string(),
        "monokai_pro" | "monokaipro" => "monokai-pro".to_string(),
        "solarized_light" | "solarizedlight" => "solarized-light".to_string(),
        "catppuccin_latte" | "catppuccinlatte" => "catppuccin-latte".to_string(),
        "github_light" | "githublight" => "github-light".to_string(),
        other => other.to_string(),
    }
}

fn user_themes_dir() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().map(|h| h.join("Library/Application Support/impulse/themes"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        dirs::config_dir().map(|c| c.join("impulse/themes"))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_builtin_themes_parse_and_resolve() {
        for (id, toml_str) in BUILTIN_THEMES {
            let tf = parse_theme(toml_str)
                .unwrap_or_else(|e| panic!("Failed to parse built-in theme '{id}': {e}"));
            let resolved = resolve_theme(id, &tf);
            assert_eq!(resolved.id, *id, "Theme '{id}' has wrong id");
            assert!(!resolved.bg.is_empty(), "Theme '{id}' has empty bg");
            assert!(!resolved.fg.is_empty(), "Theme '{id}' has empty fg");
            assert_eq!(
                resolved.terminal_palette.len(),
                16,
                "Theme '{id}' terminal palette wrong length"
            );
        }
    }

    #[test]
    fn builtin_theme_lookup() {
        let theme = builtin_theme("nord").expect("Nord should exist");
        assert_eq!(theme.name, "Nord");
        assert!(!theme.is_light);
    }

    #[test]
    fn normalized_id_lookup() {
        assert_eq!(normalize_theme_id("tokyo_night"), "tokyo-night");
        assert_eq!(normalize_theme_id("tokyonight"), "tokyo-night");
        assert_eq!(normalize_theme_id("catppuccin_mocha"), "catppuccin-mocha");
        assert_eq!(normalize_theme_id("Nord"), "nord");
    }

    #[test]
    fn get_theme_fallback() {
        let theme = get_theme("nonexistent-theme-xyz");
        assert_eq!(theme.name, "Nord", "Should fall back to Nord");
    }

    #[test]
    fn json_roundtrip() {
        let theme = get_theme("kanagawa");
        let json = theme_to_json(&theme);
        let roundtripped = theme_from_json(&json).expect("JSON roundtrip should succeed");
        assert_eq!(theme.name, roundtripped.name);
        assert_eq!(theme.bg, roundtripped.bg);
        assert_eq!(theme.syntax_keyword, roundtripped.syntax_keyword);
        assert_eq!(theme.terminal_palette, roundtripped.terminal_palette);
    }

    #[test]
    fn light_theme_detection() {
        let theme = get_theme("solarized-light");
        assert!(theme.is_light);
        let theme = get_theme("catppuccin-latte");
        assert!(theme.is_light);
        let theme = get_theme("github-light");
        assert!(theme.is_light);
    }

    #[test]
    fn dark_theme_detection() {
        let theme = get_theme("nord");
        assert!(!theme.is_light);
        let theme = get_theme("dracula");
        assert!(!theme.is_light);
    }

    #[test]
    fn kanagawa_values_match() {
        let theme = get_theme("kanagawa");
        assert_eq!(theme.bg, "#1a1a23");
        assert_eq!(theme.fg, "#DCD7BA");
        assert_eq!(theme.accent, "#56C8B0");
        assert_eq!(theme.syntax_keyword, "#B07FD8");
        assert_eq!(theme.syntax_function, "#6EA0E8");
        assert_eq!(theme.syntax_string, "#7EC850");
        assert_eq!(theme.bg_dark, "#111116");
        assert_eq!(theme.bg_surface, "#0c0c10");
        assert_eq!(theme.border, "#222230");
    }

    #[test]
    fn available_themes_includes_all_builtins() {
        let themes = available_themes();
        assert!(themes.len() >= 18);
        assert!(themes.contains(&"kanagawa".to_string()));
        assert!(themes.contains(&"nord".to_string()));
        assert!(themes.contains(&"github-light".to_string()));
    }

    #[test]
    fn display_names() {
        assert_eq!(theme_display_name("tokyo-night-storm"), "Tokyo Night Storm");
        assert_eq!(theme_display_name("rose-pine"), "Rosé Pine");
        assert_eq!(theme_display_name("github-dark"), "GitHub Dark");
        assert_eq!(theme_display_name("nord"), "Nord");
    }

    #[test]
    fn hsl_roundtrip() {
        // Test that hex -> HSL -> hex roundtrip is stable
        let colors = ["#ff0000", "#00ff00", "#0000ff", "#1a1a23", "#ffffff", "#000000"];
        for hex in colors {
            let (r, g, b) = hex_to_rgb(hex);
            let hsl = rgb_to_hsl(r, g, b);
            let (r2, g2, b2) = hsl_to_rgb(&hsl);
            assert!(
                (r as i16 - r2 as i16).unsigned_abs() <= 1
                    && (g as i16 - g2 as i16).unsigned_abs() <= 1
                    && (b as i16 - b2 as i16).unsigned_abs() <= 1,
                "HSL roundtrip failed for {hex}: ({r},{g},{b}) -> ({r2},{g2},{b2})"
            );
        }
    }

    #[test]
    fn shift_lightness_works() {
        // Shifting lightness of black up should produce a non-black color
        let lighter = shift_lightness("#000000", 0.5);
        assert_ne!(lighter, "#000000");
        // Shifting lightness of white down should produce a non-white color
        let darker = shift_lightness("#ffffff", -0.5);
        assert_ne!(darker, "#ffffff");
    }

    #[test]
    fn minimal_theme_resolves() {
        let toml = r##"
            name = "Minimal"
            variant = "dark"

            [palette]
            bg = "#1a1b26"
            fg = "#c0caf5"
            accent = "#7aa2f7"
            red = "#f7768e"
            orange = "#ff9e64"
            yellow = "#e0af68"
            green = "#9ece6a"
            cyan = "#7dcfff"
            blue = "#7aa2f7"
            magenta = "#bb9af7"
        "##;
        let tf = parse_theme(toml).expect("Minimal theme should parse");
        let resolved = resolve_theme("minimal", &tf);
        assert_eq!(resolved.id, "minimal");
        assert_eq!(resolved.name, "Minimal");
        assert!(!resolved.is_light);
        // All derived fields should be populated
        assert!(!resolved.bg_dark.is_empty());
        assert!(!resolved.bg_highlight.is_empty());
        assert!(!resolved.bg_surface.is_empty());
        assert!(!resolved.border.is_empty());
        assert!(!resolved.fg_muted.is_empty());
        assert!(!resolved.fg_comment.is_empty());
        assert!(!resolved.syntax_keyword.is_empty());
        // Terminal palette should be auto-derived
        assert_eq!(resolved.terminal_palette.len(), 16);
    }
}
