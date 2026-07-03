/// A color theme definition for the entire application.
///
/// Instances are resolved from `impulse_core::theme` (the same audited TOML
/// pipeline the macOS frontend consumes) and leaked once per theme so the
/// existing `&'static` plumbing keeps working.
pub struct ThemeColors {
    pub bg: &'static str,
    pub bg_dark: &'static str,
    pub bg_highlight: &'static str,
    /// Window-chrome surface behind the tab bar / titlebar.
    pub bg_surface: &'static str,
    /// Audited hairline stroke color.
    pub border: &'static str,
    pub fg: &'static str,
    pub fg_dark: &'static str,
    /// Interactive/selected accent (NOT the same as `cyan` — Harbor's accent
    /// is copper, for example).
    pub accent: &'static str,
    pub cyan: &'static str,
    pub blue: &'static str,
    pub green: &'static str,
    pub magenta: &'static str,
    pub red: &'static str,
    pub yellow: &'static str,
    pub orange: &'static str,
    pub comment: &'static str,
    // Contrast-audited git indicator tones (readable on the sidebar canvas).
    pub git_added: &'static str,
    pub git_modified: &'static str,
    pub git_deleted: &'static str,
    pub git_renamed: &'static str,
    pub git_conflict: &'static str,
    pub git_ignored: &'static str,
    /// Monaco base theme: `"vs-dark"` for dark themes, `"vs"` for light themes.
    pub base: &'static str,
    /// Content-surface presentation: `"flat"` renders the terminal/editor area
    /// edge-to-edge; `"card"` floats it as a rounded card with a soft shadow on
    /// the `bg_dark` canvas (Harbor style). Mirrors
    /// `impulse_core::theme::ResolvedTheme::surface_style`.
    pub surface_style: &'static str,
    /// Editor selection background — a hex color with alpha (e.g. `"#7E9CD850"`).
    pub selection: &'static str,
    pub terminal_palette: [&'static str; 16],
}

// ---------------------------------------------------------------------------
// Theme lookup (resolved from impulse-core's audited TOML pipeline)
// ---------------------------------------------------------------------------

fn leak(value: &str) -> &'static str {
    Box::leak(value.to_string().into_boxed_str())
}

/// Build a leaked `ThemeColors` from a core `ResolvedTheme`. Leaking is
/// bounded: at most one allocation set per distinct theme id per run.
fn from_resolved(rt: &impulse_core::theme::ResolvedTheme) -> &'static ThemeColors {
    Box::leak(Box::new(ThemeColors {
        bg: leak(&rt.bg),
        bg_dark: leak(&rt.bg_dark),
        bg_highlight: leak(&rt.bg_highlight),
        bg_surface: leak(&rt.bg_surface),
        border: leak(&rt.border),
        fg: leak(&rt.fg),
        fg_dark: leak(&rt.fg_muted),
        accent: leak(&rt.accent),
        cyan: leak(&rt.cyan),
        blue: leak(&rt.blue),
        green: leak(&rt.green),
        magenta: leak(&rt.magenta),
        red: leak(&rt.red),
        yellow: leak(&rt.yellow),
        orange: leak(&rt.orange),
        comment: leak(&rt.fg_comment),
        git_added: leak(&rt.git_added),
        git_modified: leak(&rt.git_modified),
        git_deleted: leak(&rt.git_deleted),
        git_renamed: leak(&rt.git_renamed),
        git_conflict: leak(&rt.git_conflict),
        git_ignored: leak(&rt.git_ignored),
        base: if rt.is_light { "vs" } else { "vs-dark" },
        surface_style: leak(&rt.surface_style),
        selection: leak(&rt.selection),
        terminal_palette: {
            let mut palette = [""; 16];
            for (slot, color) in palette.iter_mut().zip(rt.terminal_palette.iter()) {
                *slot = leak(color);
            }
            palette
        },
    }))
}

/// Return the theme matching `name` (case-insensitive), resolved through
/// `impulse_core::theme` so both frontends share the same audited palettes
/// (including user themes from `~/.config/impulse/themes/*.toml`). Falls
/// back to core's default when unknown.
pub fn get_theme(name: &str) -> &'static ThemeColors {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, &'static ThemeColors>>> = OnceLock::new();
    let key = name.to_ascii_lowercase().replace('_', "-");
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache.lock().expect("theme cache poisoned");
    if let Some(theme) = cache.get(&key) {
        return theme;
    }
    let resolved = impulse_core::theme::get_theme(&key);
    let theme = from_resolved(&resolved);
    cache.insert(key, theme);
    theme
}

/// Convert a theme ID like `"tokyo-night-storm"` to a display name like
/// `"Tokyo Night Storm"` (shared with macOS via impulse-core).
pub fn theme_display_name(id: &str) -> String {
    impulse_core::theme::theme_display_name(id)
}

/// All available theme ids: built-ins plus user themes discovered in
/// `~/.config/impulse/themes/*.toml`.
pub fn get_available_themes() -> Vec<String> {
    impulse_core::theme::available_themes()
}

// ---------------------------------------------------------------------------
// CSS loading
// ---------------------------------------------------------------------------

/// Generate and apply the application-wide CSS for the given theme.
///
/// Returns the `CssProvider` so callers can hold onto it and later replace it
/// when switching themes at runtime.
pub fn load_css(theme: &ThemeColors) -> gtk4::CssProvider {
    let mut css = format!(
        r#"
        /* --- Global font --- */
        window, popover, menu {{
            font-family: 'Inter', sans-serif;
        }}

        window.background {{
            background-color: {bg_dark};
        }}
        .impulse-root {{
            background-color: {bg_dark};
        }}
        .workspace-paned {{
            background-color: {bg_dark};
        }}
        .workspace-content {{
            margin: 0;
            background-color: {bg};
            border: none;
            border-radius: 0;
            box-shadow: none;
        }}
        .impulse-tab-view {{
            background-color: {bg};
            border-radius: 0;
        }}

        /* --- Sidebar --- */
        .sidebar {{
            margin: 0;
            background-color: {bg_dark};
            border-right: 1px solid {border};
            border-radius: 0;
            box-shadow: none;
        }}
        .sidebar-project-header {{
            padding: 4px 8px;
            border-bottom: none;
        }}
        .sidebar-toolbar-btn {{
            min-width: 24px;
            min-height: 24px;
            padding: 2px;
            border-radius: 6px;
            color: {fg_dark};
        }}
        .sidebar-toolbar-btn:hover {{
            color: {fg};
            background-color: alpha({fg}, 0.08);
        }}
        .sidebar-toolbar-btn:checked {{
            color: {accent};
            background-color: alpha({accent}, 0.14);
        }}
        .file-tree {{
            background-color: transparent;
        }}
        .file-tree row {{
            padding: 0;
            margin: 0;
            border-radius: 0;
        }}
        .file-tree row:hover {{
            background-color: alpha({fg}, 0.06);
        }}
        .file-tree row:selected {{
            background-color: alpha({fg}, 0.10);
        }}
        .sidebar-indent-guide {{
            color: alpha({comment}, 0.25);
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
        /* Per-theme contrast-audited git tones, shared with macOS via
           impulse-core's ResolvedTheme. */
        .git-modified, .file-entry-git-modified {{
            color: {git_modified};
        }}
        .git-added, .git-untracked,
        .file-entry-git-added, .file-entry-git-untracked {{
            color: {git_added};
        }}
        .git-deleted, .file-entry-git-deleted {{
            color: {git_deleted};
        }}
        .git-renamed, .file-entry-git-renamed {{
            color: {git_renamed};
        }}
        .git-conflict, .file-entry-git-conflict {{
            color: {git_conflict};
        }}
        .file-entry-git-ignored {{
            color: {git_ignored};
        }}
        .drop-target {{
            background-color: alpha({accent}, 0.10);
            outline: 1px dashed {accent};
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
            color: {fg_dark};
        }}
        .search-result-line {{
            font-size: 12px;
            color: {fg};
        }}
        /* --- Split pane dividers --- */
        paned > separator {{
            background-color: alpha({fg}, 0.10);
            min-width: 1px;
            min-height: 1px;
        }}
        .workspace-paned > separator {{
            margin: 0;
            background-color: alpha({fg}, 0.08);
        }}
        /* --- Status bar --- */
        .status-bar {{
            background-color: {bg_dark};
            padding: 3px 12px;
            min-height: 26px;
            border-top: 1px solid {border};
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
        }}
        .status-bar .status-bar-update-btn label {{
            font-size: 11px;
            color: {yellow};
        }}
        .status-bar .status-bar-update-btn:hover {{
            background: alpha({yellow}, 0.1);
        }}
        /* --- Terminal --- */
        .terminal-view {{
            background-color: {bg};
        }}
        /* --- Header bar --- */
        headerbar {{
            background-color: {bg_dark};
            border-bottom: 1px solid alpha({fg}, 0.08);
            box-shadow: none;
            min-height: 38px;
            padding: 0;
        }}
        headerbar.impulse-header {{
            background-color: {bg_surface};
        }}
        headerbar button {{
            color: {fg_dark};
        }}
        headerbar button:hover {{
            color: {fg};
            background-color: alpha({fg}, 0.08);
        }}
        .settings-error-banner {{
            background-color: alpha({yellow}, 0.14);
            border-bottom: 1px solid alpha({yellow}, 0.35);
            padding: 8px 12px;
        }}
        .settings-error-banner image {{
            color: {yellow};
        }}
        .settings-error-title {{
            color: {fg};
            font-weight: 600;
        }}
        .settings-error-detail {{
            color: {fg_dark};
            font-size: 11px;
        }}
        button.settings-error-action {{
            min-height: 24px;
            padding: 2px 10px;
            border-radius: 4px;
            color: {yellow};
            border: 1px solid alpha({yellow}, 0.45);
            background: transparent;
            box-shadow: none;
        }}
        button.settings-error-action:hover {{
            background: alpha({yellow}, 0.12);
        }}
        button.settings-error-dismiss {{
            min-width: 26px;
            min-height: 26px;
            padding: 2px;
            border-radius: 999px;
            background: transparent;
            border: 1px solid transparent;
            box-shadow: none;
        }}
        button.settings-error-dismiss:hover {{
            background: alpha({fg}, 0.08);
        }}
        button.impulse-header-button {{
            min-width: 30px;
            min-height: 30px;
            padding: 3px;
            border-radius: 999px;
            background-color: transparent;
            border: 1px solid transparent;
            box-shadow: none;
        }}
        button.impulse-header-button:hover {{
            color: {fg};
            background-color: alpha({fg}, 0.08);
            border-color: alpha({fg}, 0.08);
        }}
        button.impulse-header-button:checked {{
            color: {accent};
            background-color: alpha({fg}, 0.10);
            border-color: alpha({fg}, 0.10);
        }}
        tabbar {{
            background-color: {bg_dark};
        }}
        tabbar revealer > box {{
            box-shadow: none;
            padding: 0;
        }}
        tabbar tabbox {{
            background-color: {bg_dark};
        }}
        tabbar tab {{
            min-height: 32px;
            padding: 0 8px;
            margin: 0;
            background-color: {bg_dark};
            color: {fg_dark};
            border-radius: 6px 6px 0 0;
            border: 1px solid transparent;
        }}
        tabbar tab:selected {{
            background-color: {bg};
            color: {accent};
            border-color: transparent;
        }}
        tabbar tab:hover:not(:selected) {{
            background-color: alpha({fg}, 0.06);
            color: {fg};
        }}
        tabbar tab image {{
            margin-right: 2px;
        }}
        tabbar tab label {{
            font-size: 13px;
            font-weight: 500;
        }}
        /* --- Quick open --- */
        .quick-open {{
            background-color: {bg_dark};
            border-radius: 10px;
            border: 1px solid alpha({fg}, 0.10);
            box-shadow: 0 8px 24px alpha(#000000, 0.35);
        }}
        .quick-open entry {{
            margin: 8px;
            font-size: 14px;
        }}
        .quick-open list row:hover {{
            background-color: alpha({fg}, 0.08);
        }}
        .quick-open list row:selected {{
            background-color: alpha({fg}, 0.12);
        }}
        .quick-open list row label {{
            padding: 6px 12px;
            color: {fg};
        }}
        /* --- Terminal search bar --- */
        .terminal-search-bar {{
            background-color: alpha({bg_dark}, 0.98);
            padding: 6px 8px;
            border-bottom: 1px solid alpha({fg}, 0.08);
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
            background-color: transparent;
            border-top: 1px solid alpha({fg}, 0.07);
        }}
        .project-search-row {{
            padding: 8px;
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
        .project-search-results row:hover {{
            background-color: alpha({fg}, 0.06);
        }}
        .project-search-results row:selected {{
            background-color: alpha({fg}, 0.10);
        }}
        .project-search-file-header {{
            padding: 4px 8px;
            background-color: transparent;
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
        /* --- Vertical tab list (sidebar) --- */
        .vertical-tabs-list {{
            background-color: transparent;
            padding: 6px 8px 6px 8px;
        }}
        .vertical-tab-attention {{
            color: {accent};
            font-size: 8px;
        }}
        .vertical-tab-pin {{
            opacity: 0.6;
        }}
        .vertical-tabs-resize-handle {{
            min-height: 7px;
            padding: 3px 0;
        }}
        .vertical-tabs-resize-handle:hover separator {{
            background-color: alpha({fg}, 0.35);
            min-height: 2px;
        }}
        .vertical-tabs-list row {{
            border-radius: 6px;
            padding: 4px 8px;
            margin: 1px 0;
        }}
        .vertical-tabs-list row:hover {{
            background-color: alpha({fg}, 0.06);
        }}
        .vertical-tabs-list row:selected {{
            background-color: alpha({accent}, 0.16);
        }}
        .vertical-tab-title {{
            font-size: 12px;
            color: {fg};
        }}
        .vertical-tab-subtitle {{
            font-size: 10px;
            color: {fg_dark};
        }}
        .vertical-tab-close {{
            opacity: 0;
            min-width: 18px;
            min-height: 18px;
            padding: 1px;
            border-radius: 999px;
        }}
        .vertical-tabs-list row:hover .vertical-tab-close,
        .vertical-tabs-list row:selected .vertical-tab-close {{
            opacity: 1;
        }}
        /* --- Terminal context bar --- */
        .context-bar {{
            background-color: {bg};
            border-top: 1px solid alpha({fg}, 0.10);
            padding: 8px 12px;
        }}
        .context-prompt-arrow {{
            color: {blue};
            font-family: 'JetBrains Mono', monospace;
            font-size: 13px;
            font-weight: 600;
        }}
        .context-bar button.context-stop {{
            color: {red};
        }}
        .context-chip {{
            font-size: 11px;
            font-family: 'JetBrains Mono', monospace;
            color: {fg_dark};
            background-color: {bg_highlight};
            border-radius: 999px;
            padding: 2px 10px;
        }}
        .status-bar button.status-bar-review-btn {{
            font-size: 11px;
            font-family: 'JetBrains Mono', monospace;
            color: {fg_dark};
            background-color: {bg_highlight};
            background-image: none;
            border: none;
            box-shadow: none;
            border-radius: 999px;
            padding: 1px 10px;
            min-height: 0;
            min-width: 0;
        }}
        .status-bar button.status-bar-review-btn:hover {{
            color: {fg};
            background-color: alpha({fg}, 0.15);
        }}
        .context-bar menubutton.context-chip-button > button,
        .context-bar button.context-chip-button {{
            font-size: 11px;
            font-family: 'JetBrains Mono', monospace;
            color: {fg_dark};
            background-color: {bg_highlight};
            background-image: none;
            border: none;
            box-shadow: none;
            border-radius: 999px;
            padding: 2px 10px;
            min-height: 0;
            min-width: 0;
        }}
        .context-bar menubutton.context-chip-button > button:hover,
        .context-bar button.context-chip-button:hover {{
            color: {fg};
            background-color: alpha({fg}, 0.15);
        }}
        .branch-popover searchentry {{
            margin: 6px;
        }}
        .branch-popover .branch-list {{
            background: transparent;
        }}
        .branch-popover .branch-list row {{
            border-radius: 6px;
        }}
        .context-chip-ok {{
            color: {green};
        }}
        .context-chip-error {{
            color: {red};
        }}
        .context-input {{
            font-family: 'JetBrains Mono', monospace;
            font-size: 12px;
            min-height: 28px;
            padding-left: 8px;
            padding-right: 8px;
            background-color: {bg};
            border: 1px solid {border};
            border-radius: 8px;
            box-shadow: none;
            outline: none;
        }}
        .context-input:focus-within {{
            border-color: alpha({accent}, 0.6);
        }}
        .context-chip-icon {{
            color: alpha({fg_dark}, 0.9);
            font-size: 10px;
        }}
        .context-run-hint {{
            font-size: 10px;
            color: {comment};
        }}
        .completion-kind {{
            font-size: 10px;
            font-family: 'JetBrains Mono', monospace;
            color: {comment};
        }}
        /* Ghost suggestion overlay: must use the same font metrics and
           horizontal inset as .context-input so the suffix lines up. */
        .context-ghost {{
            font-family: 'JetBrains Mono', monospace;
            font-size: 12px;
            margin-left: 8px;
            color: alpha({fg}, 0.45);
        }}
        .completion-popover contents {{
            padding: 4px;
        }}
        .completion-list {{
            background: transparent;
        }}
        .completion-list row {{
            border-radius: 6px;
            padding: 2px 4px;
        }}
        .completion-label {{
            font-family: 'JetBrains Mono', monospace;
            font-size: 12px;
        }}
        /* --- Review Changes tab --- */
        .review-header {{
            padding: 8px 14px;
            background-color: {bg_dark};
        }}
        .review-repo {{
            font-size: 13px;
            font-weight: 600;
        }}
        .review-branch {{
            font-size: 12px;
            color: {blue};
        }}
        .review-count {{
            font-size: 12px;
            color: {fg_dark};
        }}
        .review-added {{
            font-size: 12px;
            font-family: 'JetBrains Mono', monospace;
            color: {green};
        }}
        .review-removed {{
            font-size: 12px;
            font-family: 'JetBrains Mono', monospace;
            color: {red};
        }}
        .review-commit-bar {{
            padding: 8px 14px;
            background-color: {bg_dark};
        }}
        .review-confirmation {{
            font-size: 12px;
            color: {green};
        }}
        .context-bar button.flat {{
            min-width: 24px;
            min-height: 24px;
            padding: 2px;
            border-radius: 6px;
            color: {fg_dark};
        }}
        .context-bar button.flat:hover {{
            color: {fg};
            background-color: alpha({fg}, 0.08);
        }}
        "#,
        bg_dark = theme.bg_dark,
        bg = theme.bg,
        bg_highlight = theme.bg_highlight,
        fg = theme.fg,
        fg_dark = theme.fg_dark,
        accent = theme.accent,
        bg_surface = theme.bg_surface,
        border = theme.border,
        cyan = theme.cyan,
        blue = theme.blue,
        magenta = theme.magenta,
        green = theme.green,
        yellow = theme.yellow,
        red = theme.red,
        comment = theme.comment,
        git_added = theme.git_added,
        git_modified = theme.git_modified,
        git_deleted = theme.git_deleted,
        git_renamed = theme.git_renamed,
        git_conflict = theme.git_conflict,
        git_ignored = theme.git_ignored,
    );

    if theme.surface_style == "card" {
        // Card-surface themes (Harbor): the content stays EDGE-TO-EDGE to
        // match the Warp-style terminal — no floating card, mirroring the
        // macOS treatment. "Card" is expressed through raised pill tabs
        // with a soft warm shadow and quieter chrome strokes instead.
        css.push_str(&format!(
            r#"
        .workspace-paned > separator {{
            background-color: transparent;
        }}
        .sidebar {{
            border-right: none;
        }}
        tabbar tab {{
            border-radius: 999px;
            margin: 3px 2px;
        }}
        tabbar tab:selected {{
            background-color: {bg};
            color: {fg};
            box-shadow: 0 1px 2px alpha(#5c5142, 0.18);
        }}
        .status-bar {{
            border-top: none;
        }}
        .status-bar .shell-name {{
            color: {fg};
            font-weight: 700;
        }}
        .status-bar .git-branch {{
            color: {fg_dark};
        }}
        .status-bar .cwd {{
            color: {comment};
        }}
        "#,
            bg = theme.bg,
            fg = theme.fg,
            fg_dark = theme.fg_dark,
            comment = theme.comment,
        ));
    }

    let provider = gtk4::CssProvider::new();
    provider.load_from_string(&css);
    gtk4::style_context_add_provider_for_display(
        &gtk4::gdk::Display::default().expect("Could not get default display"),
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_USER,
    );
    provider
}
