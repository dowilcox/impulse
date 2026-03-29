use gtk4::gdk;
use impulse_core::theme::ResolvedTheme;

// ---------------------------------------------------------------------------
// Re-export helpers — delegate to impulse_core::theme
// ---------------------------------------------------------------------------

/// Return the theme matching `name` (case-insensitive). Falls back to Nord.
pub fn get_theme(name: &str) -> ResolvedTheme {
    impulse_core::theme::get_theme(name)
}

/// Return the list of available theme names (built-in + user themes).
pub fn get_available_themes() -> Vec<String> {
    impulse_core::theme::available_themes()
}

/// Convert a theme ID like `"tokyo-night-storm"` to a display name like `"Tokyo Night Storm"`.
pub fn theme_display_name(id: &str) -> String {
    impulse_core::theme::theme_display_name(id)
}

// ---------------------------------------------------------------------------
// RGBA helpers for GTK
// ---------------------------------------------------------------------------

pub fn parse_color(hex: &str) -> gdk::RGBA {
    gdk::RGBA::parse(hex).unwrap_or_else(|_| {
        log::warn!("Invalid color value: '{}', using fallback", hex);
        gdk::RGBA::new(1.0, 0.0, 1.0, 1.0) // Magenta fallback makes errors visible
    })
}

pub fn fg_rgba(theme: &ResolvedTheme) -> gdk::RGBA {
    parse_color(&theme.fg)
}

pub fn bg_rgba(theme: &ResolvedTheme) -> gdk::RGBA {
    parse_color(&theme.bg)
}

pub fn terminal_palette_rgba(theme: &ResolvedTheme) -> Vec<gdk::RGBA> {
    theme
        .terminal_palette
        .iter()
        .map(|hex| parse_color(hex))
        .collect()
}

// ---------------------------------------------------------------------------
// CSS loading
// ---------------------------------------------------------------------------

/// Generate and apply the application-wide CSS for the given theme.
///
/// Returns the `CssProvider` so callers can hold onto it and later replace it
/// when switching themes at runtime.
pub fn load_css(theme: &ResolvedTheme) -> gtk4::CssProvider {
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
            color: {fg_muted};
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
            color: {fg_muted};
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
            color: {fg_muted};
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
            color: {fg_muted};
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
            color: {fg_muted};
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
            color: {fg_muted};
            padding-left: 12px;
        }}
        .status-bar .language-name {{
            color: {blue};
            padding-left: 12px;
        }}
        .status-bar .encoding {{
            color: {fg_muted};
            padding-left: 12px;
        }}
        .status-bar .indent-info {{
            color: {fg_muted};
            padding-left: 12px;
        }}
        .status-bar .blame-info {{
            color: {fg_muted};
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
            color: {fg_muted};
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
            color: {fg_muted};
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
            color: {fg_muted};
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
            background-color: {fg_comment};
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
            color: {fg_muted};
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
            color: {fg_muted};
            font-size: 11px;
        }}
        .project-search-match {{
            padding: 2px 8px 2px 16px;
        }}
        .project-search-line-num {{
            color: {fg_muted};
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
        fg_muted = theme.fg_muted,
        cyan = theme.cyan,
        blue = theme.blue,
        magenta = theme.magenta,
        green = theme.green,
        yellow = theme.yellow,
        red = theme.red,
        orange = theme.orange,
        fg_comment = theme.fg_comment,
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
