//! Font settings that name several families, CSS style ("Fira Code,
//! Menlo"), as the Swift app's Monaco and terminal take them: the first one
//! installed is used, else the first named.

use std::collections::HashSet;
use std::sync::OnceLock;

use gpui_kit::{App, SharedString};

static INSTALLED: OnceLock<HashSet<String>> = OnceLock::new();

/// Note the installed families (after the bundled fonts are added).
pub fn init(cx: &App) {
    let _ = INSTALLED.set(cx.text_system().all_font_names().into_iter().collect());
}

/// The family to draw with for a font setting.
pub fn family(setting: &str) -> SharedString {
    if !setting.contains(',') {
        return SharedString::from(setting.trim().to_string());
    }
    let names: Vec<&str> = setting
        .split(',')
        .map(|name| name.trim().trim_matches(['"', '\'']))
        .filter(|name| !name.is_empty())
        .collect();
    let installed = INSTALLED.get();
    names
        .iter()
        .find(|name| installed.is_some_and(|fonts| fonts.contains(**name)))
        .or(names.first())
        .map(|name| SharedString::from(name.to_string()))
        .unwrap_or_default()
}
