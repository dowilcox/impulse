//! The command palette's items and recents (the Swift `CommandPalette`,
//! itself ported from `impulse-core/src/command_palette.rs`): the built-in
//! commands, items for custom commands and search results with stable
//! FNV-1a ids, and the recently used list. Filtering is `fuzzy::rank` over
//! the items' titles.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::search_result::SearchResult;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CommandPaletteItem {
    pub id: String,
    pub title: String,
    pub category: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    /// `"builtin"`, `"custom"` or `"dynamic"`.
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<BTreeMap<String, String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RecentCommandItem {
    pub id: String,
    pub title: String,
    pub last_used_ms: u64,
    pub use_count: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RecentCommandStore {
    #[serde(default)]
    pub items: Vec<RecentCommandItem>,
}

impl RecentCommandStore {
    /// Record a use of `item`: one entry per id (a renamed custom command
    /// keeps its entry and takes the new title), most recent first, then
    /// most used, then by title; at most `max_items` kept.
    pub fn record(&mut self, item: &CommandPaletteItem, now_ms: u64, max_items: usize) {
        if let Some(existing) = self.items.iter_mut().find(|r| r.id == item.id) {
            existing.title = item.title.clone();
            existing.last_used_ms = now_ms;
            existing.use_count = existing.use_count.saturating_add(1);
        } else {
            self.items.push(RecentCommandItem {
                id: item.id.clone(),
                title: item.title.clone(),
                last_used_ms: now_ms,
                use_count: 1,
            });
        }
        self.items.sort_by(|a, b| {
            b.last_used_ms
                .cmp(&a.last_used_ms)
                .then(b.use_count.cmp(&a.use_count))
                .then_with(|| a.title.cmp(&b.title))
        });
        self.items.truncate(max_items);
    }
}

const BUILTIN_COMMANDS: &[(&str, &str, &str, &[&str])] = &[
    ("new_tab", "New Tab", "Tabs", &["terminal", "shell"]),
    ("close_tab", "Close Tab", "Tabs", &["remove"]),
    (
        "reopen_tab",
        "Reopen Closed Tab",
        "Tabs",
        &["restore", "undo"],
    ),
    ("next_tab", "Show Next Tab", "Tabs", &["navigate"]),
    ("prev_tab", "Show Previous Tab", "Tabs", &["navigate"]),
    ("copy", "Copy", "Terminal", &["clipboard"]),
    ("paste", "Paste", "Terminal", &["clipboard"]),
    (
        "review_changes",
        "Review Changes",
        "Navigation",
        &["git", "diff", "commit", "review"],
    ),
    ("new_file", "New File", "Editor", &["editor"]),
    ("save", "Save", "Editor", &["write"]),
    ("find", "Find…", "Editor", &["search"]),
    ("go_to_line", "Go to Line…", "Editor", &["jump", "navigate"]),
    (
        "toggle_markdown_preview",
        "Toggle Markdown Preview",
        "Editor",
        &["markdown", "preview"],
    ),
    ("toggle_sidebar", "Toggle Sidebar", "Navigation", &["files"]),
    (
        "quick_open",
        "Go to File…",
        "Navigation",
        &["file", "finder"],
    ),
    (
        "project_search",
        "Find in Project",
        "Navigation",
        &["search", "files"],
    ),
    (
        "command_palette",
        "Command Palette",
        "Navigation",
        &["commands"],
    ),
    ("open_settings", "Settings…", "Navigation", &["preferences"]),
    ("font_increase", "Increase Font Size", "Font", &["zoom"]),
    ("font_decrease", "Decrease Font Size", "Font", &["zoom"]),
    ("font_reset", "Reset Font Size", "Font", &["zoom"]),
    ("new_window", "New Window", "App", &["window"]),
    ("fullscreen", "Toggle Full Screen", "App", &["window"]),
    (
        "install_lsp",
        "Install Web LSP Servers",
        "Language Servers",
        &["typescript", "php", "html", "css"],
    ),
];

pub fn builtin_items() -> Vec<CommandPaletteItem> {
    BUILTIN_COMMANDS
        .iter()
        .map(|(id, title, category, keywords)| CommandPaletteItem {
            id: id.to_string(),
            title: title.to_string(),
            category: category.to_string(),
            keywords: keywords.iter().map(|k| k.to_string()).collect(),
            source: "builtin".into(),
            shortcut: None,
            payload: None,
        })
        .collect()
}

/// The item for a custom command from the settings: titled by its name
/// (the command when the name is blank), its id stable across renames.
pub fn custom_command_item(
    name: &str,
    shortcut: Option<&str>,
    command: &str,
    args: &[String],
) -> CommandPaletteItem {
    let trimmed_name = name.trim();
    let title = if trimmed_name.is_empty() {
        command
    } else {
        trimmed_name
    };
    let shortcut = shortcut.map(str::trim).filter(|s| !s.is_empty());
    CommandPaletteItem {
        id: custom_command_id(command, args),
        title: title.to_string(),
        category: "Custom".into(),
        keywords: vec![command.to_string()],
        source: "custom".into(),
        shortcut: shortcut.map(String::from),
        payload: None,
    }
}

pub fn custom_command_id(command: &str, args: &[String]) -> String {
    let mut value = command.trim().to_string();
    value.push('\0');
    for arg in args {
        value.push_str(arg);
        value.push('\0');
    }
    format!("custom:external:{:016x}", stable_hash(value.as_bytes()))
}

/// Palette items for raw search results: stable FNV-based ids, a payload
/// with the kind, path, line and column.
pub fn search_result_items(root: &str, results: &[SearchResult]) -> Vec<CommandPaletteItem> {
    results
        .iter()
        .map(|result| search_result_item(root, result))
        .collect()
}

fn search_result_item(root: &str, result: &SearchResult) -> CommandPaletteItem {
    let relative_path = relative_display_path(root, &result.path);
    let is_content = result.match_type == "content";
    let kind = if is_content { "content" } else { "file" };

    let mut id_material = format!("{kind}\0{}", result.path);
    if let Some(line) = result.line_number {
        id_material.push_str(&format!("\0{line}"));
    }
    if let Some(column) = result.column_start {
        id_material.push_str(&format!("\0{column}"));
    }

    let mut payload = BTreeMap::from([
        ("kind".to_string(), kind.to_string()),
        ("path".to_string(), result.path.clone()),
    ]);
    if let Some(line) = result.line_number {
        payload.insert("line".into(), line.to_string());
    }
    if let Some(column) = result.column_start {
        payload.insert("column".into(), column.to_string());
    }

    let mut keywords = vec![
        result.name.clone(),
        relative_path.clone(),
        result.path.clone(),
    ];
    if let Some(content) = &result.line_content {
        keywords.push(content.clone());
    }

    let title = match (is_content, result.line_number) {
        (true, Some(line)) => format!("{relative_path}:{line}"),
        _ => relative_path,
    };

    CommandPaletteItem {
        id: format!("{kind}:{:016x}", stable_hash(id_material.as_bytes())),
        title,
        category: if is_content {
            "Project Search"
        } else {
            "Files"
        }
        .into(),
        keywords,
        source: "dynamic".into(),
        shortcut: None,
        payload: Some(payload),
    }
}

/// FNV-1a, matching the Rust `stable_hash` the ids were first made with.
pub fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn relative_display_path(root: &str, path: &str) -> String {
    let root = root.strip_suffix('/').unwrap_or(root);
    if let Some(relative) = path
        .strip_prefix(root)
        .and_then(|rest| rest.strip_prefix('/'))
    {
        if !relative.is_empty() {
            return relative.to_string();
        }
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recents_dedupe_by_stable_id_across_renames() {
        let args = vec!["test".to_string()];
        let first = custom_command_item("Test Runner", Some("Ctrl+R"), "cargo", &args);
        let renamed = custom_command_item("Run Tests", Some("Ctrl+R"), "cargo", &args);
        assert_eq!(first.id, renamed.id);

        let mut recents = RecentCommandStore::default();
        recents.record(&first, 10, 20);
        recents.record(&renamed, 20, 20);
        assert_eq!(recents.items.len(), 1);
        assert_eq!(recents.items[0].title, "Run Tests");
        assert_eq!(recents.items[0].use_count, 2);
    }

    #[test]
    fn recents_keep_at_most_max_items() {
        let items = builtin_items();
        let mut recents = RecentCommandStore::default();
        for (i, item) in items.iter().take(5).enumerate() {
            recents.record(item, i as u64, 3);
        }
        let ids: Vec<&str> = recents.items.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["prev_tab", "next_tab", "reopen_tab"]);
    }

    #[test]
    fn custom_item_falls_back_to_the_command() {
        let item = custom_command_item("  ", Some("  "), "make", &[]);
        assert_eq!(item.title, "make");
        assert_eq!(item.shortcut, None);
    }

    #[test]
    fn search_result_items_include_payloads() {
        let results = vec![
            SearchResult::file("/repo/src/main.rs", "main.rs"),
            SearchResult {
                path: "/repo/src/lib.rs".into(),
                name: "lib.rs".into(),
                line_number: Some(42),
                line_content: Some("pub fn search_items()".into()),
                column_start: Some(7),
                column_end: Some(19),
                match_type: "content".into(),
            },
        ];
        let items = search_result_items("/repo", &results);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "src/main.rs");
        assert_eq!(items[0].category, "Files");
        assert_eq!(items[0].payload.as_ref().unwrap()["kind"], "file");
        assert_eq!(items[1].title, "src/lib.rs:42");
        assert_eq!(items[1].category, "Project Search");
        assert_eq!(items[1].payload.as_ref().unwrap()["line"], "42");
        assert!(items[1].keywords.iter().any(|k| k.contains("search_items")));
    }

    #[test]
    fn relative_paths_fall_back_to_the_full_path() {
        assert_eq!(relative_display_path("/repo/", "/repo/a.rs"), "a.rs");
        assert_eq!(
            relative_display_path("/repo", "/repository/a.rs"),
            "/repository/a.rs"
        );
        assert_eq!(relative_display_path("/repo", "/repo/"), "/repo/");
    }
}
