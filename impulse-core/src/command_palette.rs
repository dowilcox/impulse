use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommandPaletteItem {
    pub id: String,
    pub title: String,
    pub category: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    pub source: CommandPaletteSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub payload: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandPaletteSource {
    Builtin,
    Custom,
    Dynamic,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecentCommandItem {
    pub id: String,
    pub title: String,
    pub last_used_ms: u64,
    pub use_count: u32,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct RecentCommandStore {
    pub items: Vec<RecentCommandItem>,
}

#[derive(Clone, Copy, Debug)]
struct BuiltinCommand {
    id: &'static str,
    title: &'static str,
    category: &'static str,
    keywords: &'static [&'static str],
}

const BUILTIN_COMMANDS: &[BuiltinCommand] = &[
    BuiltinCommand {
        id: "new_tab",
        title: "New Terminal Tab",
        category: "Tabs",
        keywords: &["terminal", "shell"],
    },
    BuiltinCommand {
        id: "close_tab",
        title: "Close Tab",
        category: "Tabs",
        keywords: &["remove"],
    },
    BuiltinCommand {
        id: "reopen_tab",
        title: "Reopen Closed Tab",
        category: "Tabs",
        keywords: &["restore", "undo"],
    },
    BuiltinCommand {
        id: "next_tab",
        title: "Next Tab",
        category: "Tabs",
        keywords: &["navigate"],
    },
    BuiltinCommand {
        id: "prev_tab",
        title: "Previous Tab",
        category: "Tabs",
        keywords: &["navigate"],
    },
    BuiltinCommand {
        id: "copy",
        title: "Copy",
        category: "Terminal",
        keywords: &["clipboard"],
    },
    BuiltinCommand {
        id: "paste",
        title: "Paste",
        category: "Terminal",
        keywords: &["clipboard"],
    },
    BuiltinCommand {
        id: "split_horizontal",
        title: "Split Horizontally",
        category: "Terminal",
        keywords: &["pane", "terminal"],
    },
    BuiltinCommand {
        id: "split_vertical",
        title: "Split Vertically",
        category: "Terminal",
        keywords: &["pane", "terminal"],
    },
    BuiltinCommand {
        id: "focus_prev_split",
        title: "Focus Previous Split",
        category: "Terminal",
        keywords: &["pane", "terminal", "navigate"],
    },
    BuiltinCommand {
        id: "focus_next_split",
        title: "Focus Next Split",
        category: "Terminal",
        keywords: &["pane", "terminal", "navigate"],
    },
    BuiltinCommand {
        id: "new_file",
        title: "New File",
        category: "Editor",
        keywords: &["editor"],
    },
    BuiltinCommand {
        id: "save",
        title: "Save File",
        category: "Editor",
        keywords: &["write"],
    },
    BuiltinCommand {
        id: "find",
        title: "Find",
        category: "Editor",
        keywords: &["search"],
    },
    BuiltinCommand {
        id: "go_to_line",
        title: "Go to Line",
        category: "Editor",
        keywords: &["jump", "navigate"],
    },
    BuiltinCommand {
        id: "toggle_markdown_preview",
        title: "Toggle Preview",
        category: "Editor",
        keywords: &["markdown", "preview"],
    },
    BuiltinCommand {
        id: "toggle_sidebar",
        title: "Toggle Sidebar",
        category: "Navigation",
        keywords: &["files"],
    },
    BuiltinCommand {
        id: "quick_open",
        title: "Quick Open File",
        category: "Navigation",
        keywords: &["file", "finder"],
    },
    BuiltinCommand {
        id: "project_search",
        title: "Find in Project",
        category: "Navigation",
        keywords: &["search", "files"],
    },
    BuiltinCommand {
        id: "command_palette",
        title: "Command Palette",
        category: "Navigation",
        keywords: &["commands"],
    },
    BuiltinCommand {
        id: "open_settings",
        title: "Open Settings",
        category: "Navigation",
        keywords: &["preferences"],
    },
    BuiltinCommand {
        id: "font_increase",
        title: "Increase Font Size",
        category: "Font",
        keywords: &["zoom"],
    },
    BuiltinCommand {
        id: "font_decrease",
        title: "Decrease Font Size",
        category: "Font",
        keywords: &["zoom"],
    },
    BuiltinCommand {
        id: "font_reset",
        title: "Reset Font Size",
        category: "Font",
        keywords: &["zoom"],
    },
    BuiltinCommand {
        id: "new_window",
        title: "New Window",
        category: "App",
        keywords: &["window"],
    },
    BuiltinCommand {
        id: "fullscreen",
        title: "Toggle Fullscreen",
        category: "App",
        keywords: &["window"],
    },
    BuiltinCommand {
        id: "install_lsp",
        title: "Install Web LSP Servers",
        category: "Language Servers",
        keywords: &["typescript", "php", "html", "css"],
    },
];

pub fn builtin_items() -> Vec<CommandPaletteItem> {
    BUILTIN_COMMANDS
        .iter()
        .map(|command| CommandPaletteItem {
            id: command.id.to_string(),
            title: command.title.to_string(),
            category: command.category.to_string(),
            keywords: command
                .keywords
                .iter()
                .map(|keyword| (*keyword).to_string())
                .collect(),
            source: CommandPaletteSource::Builtin,
            shortcut: None,
            payload: BTreeMap::new(),
        })
        .collect()
}

pub fn custom_command_item(
    name: &str,
    shortcut: Option<&str>,
    command: &str,
    args: &[String],
) -> CommandPaletteItem {
    let title = name.trim();
    let title = if title.is_empty() { command } else { title };
    CommandPaletteItem {
        id: custom_command_id(command, args),
        title: title.to_string(),
        category: "Custom".to_string(),
        keywords: vec![command.to_string()],
        source: CommandPaletteSource::Custom,
        shortcut: shortcut
            .map(str::trim)
            .filter(|shortcut| !shortcut.is_empty())
            .map(str::to_string),
        payload: BTreeMap::new(),
    }
}

pub fn custom_command_id(command: &str, args: &[String]) -> String {
    let mut value = String::from(command.trim());
    value.push('\0');
    for arg in args {
        value.push_str(arg);
        value.push('\0');
    }
    format!("custom:external:{:016x}", stable_hash(value.as_bytes()))
}

pub fn search_items(
    root: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<CommandPaletteItem>, String> {
    let query = query.trim();
    if query.len() < 2 || limit == 0 {
        return Ok(Vec::new());
    }

    let file_limit = limit.min(12);
    let mut results = crate::search::search_filenames(root, query, file_limit, None)?;

    let remaining = limit.saturating_sub(results.len());
    if remaining > 0 && query.len() >= 3 {
        results.extend(crate::search::search_contents(
            root, query, remaining, false, None,
        )?);
    }

    Ok(search_result_items(root, &results))
}

pub fn search_result_items(
    root: &str,
    results: &[crate::search::SearchResult],
) -> Vec<CommandPaletteItem> {
    results
        .iter()
        .map(|result| search_result_item(root, result))
        .collect()
}

fn search_result_item(root: &str, result: &crate::search::SearchResult) -> CommandPaletteItem {
    let relative_path = relative_display_path(root, &result.path);
    let is_content = result.match_type == "content";
    let kind = if is_content { "content" } else { "file" };

    let mut id_material = String::new();
    id_material.push_str(kind);
    id_material.push('\0');
    id_material.push_str(&result.path);
    if let Some(line) = result.line_number {
        id_material.push('\0');
        id_material.push_str(&line.to_string());
    }
    if let Some(column) = result.column_start {
        id_material.push('\0');
        id_material.push_str(&column.to_string());
    }

    let mut payload = BTreeMap::new();
    payload.insert("kind".to_string(), kind.to_string());
    payload.insert("path".to_string(), result.path.clone());
    if let Some(line) = result.line_number {
        payload.insert("line".to_string(), line.to_string());
    }
    if let Some(column) = result.column_start {
        payload.insert("column".to_string(), column.to_string());
    }

    let mut keywords = vec![
        result.name.clone(),
        relative_path.clone(),
        result.path.clone(),
    ];
    if let Some(line_content) = result.line_content.as_ref() {
        keywords.push(line_content.clone());
    }

    let title = if is_content {
        match result.line_number {
            Some(line) => format!("{relative_path}:{line}"),
            None => relative_path,
        }
    } else {
        relative_path
    };

    CommandPaletteItem {
        id: format!("{}:{:016x}", kind, stable_hash(id_material.as_bytes())),
        title,
        category: if is_content {
            "Project Search".to_string()
        } else {
            "Files".to_string()
        },
        keywords,
        source: CommandPaletteSource::Dynamic,
        shortcut: None,
        payload,
    }
}

pub fn filter_items(
    items: &[CommandPaletteItem],
    recents: &RecentCommandStore,
    query: &str,
) -> Vec<CommandPaletteItem> {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(str::to_lowercase)
        .filter(|term| !term.is_empty())
        .collect();

    let mut seen_ids = HashSet::new();
    let mut scored: Vec<(i64, usize, CommandPaletteItem)> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            if !seen_ids.insert(item.id.clone()) {
                return None;
            }
            let query_score = score_query(item, &terms)?;
            let recent_score = recents.score(&item.id);
            Some((query_score + recent_score, index, item.clone()))
        })
        .collect();

    scored.sort_by(|(a_score, a_index, a_item), (b_score, b_index, b_item)| {
        b_score
            .cmp(a_score)
            .then_with(|| a_index.cmp(b_index))
            .then_with(|| a_item.title.cmp(&b_item.title))
    });

    scored.into_iter().map(|(_, _, item)| item).collect()
}

impl RecentCommandStore {
    pub fn record(&mut self, item: &CommandPaletteItem, now_ms: u64, max_items: usize) {
        if let Some(existing) = self.items.iter_mut().find(|recent| recent.id == item.id) {
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
                .then_with(|| b.use_count.cmp(&a.use_count))
                .then_with(|| a.title.cmp(&b.title))
        });
        self.items.truncate(max_items);
    }

    pub fn score(&self, id: &str) -> i64 {
        self.items
            .iter()
            .position(|recent| recent.id == id)
            .map(|index| {
                let recent = &self.items[index];
                10_000 - (index as i64 * 250) + i64::from(recent.use_count.min(100))
            })
            .unwrap_or(0)
    }
}

fn score_query(item: &CommandPaletteItem, terms: &[String]) -> Option<i64> {
    if terms.is_empty() {
        return Some(0);
    }

    let title = item.title.to_lowercase();
    let category = item.category.to_lowercase();
    let keywords: Vec<String> = item
        .keywords
        .iter()
        .map(|keyword| keyword.to_lowercase())
        .collect();

    let mut score = 0;
    for term in terms {
        if title == *term {
            score += 2_000;
        } else if title.starts_with(term) {
            score += 1_500;
        } else if title.contains(term) {
            score += 1_000;
        } else if category.contains(term) {
            score += 500;
        } else if keywords.iter().any(|keyword| keyword.contains(term)) {
            score += 250;
        } else {
            return None;
        }
    }
    Some(score)
}

fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn relative_display_path(root: &str, path: &str) -> String {
    Path::new(path)
        .strip_prefix(Path::new(root))
        .ok()
        .and_then(|relative| relative.to_str())
        .filter(|relative| !relative.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_has_stable_items() {
        let items = builtin_items();
        assert!(items.iter().any(|item| item.id == "command_palette"));
        assert!(items.iter().any(|item| item.id == "quick_open"));
        assert!(items.iter().any(|item| item.id == "install_lsp"));
        assert!(items.iter().all(|item| !item.title.is_empty()));
    }

    #[test]
    fn filter_matches_title_category_and_keywords() {
        let items = builtin_items();
        let recents = RecentCommandStore::default();

        let title_matches = filter_items(&items, &recents, "settings");
        assert_eq!(
            title_matches.first().map(|item| item.id.as_str()),
            Some("open_settings")
        );

        let category_matches = filter_items(&items, &recents, "font");
        assert!(category_matches
            .iter()
            .any(|item| item.id == "font_increase"));

        let keyword_matches = filter_items(&items, &recents, "typescript");
        assert_eq!(
            keyword_matches.first().map(|item| item.id.as_str()),
            Some("install_lsp")
        );
    }

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
    fn recents_boost_empty_query_results() {
        let items = builtin_items();
        let mut recents = RecentCommandStore::default();
        let settings = items
            .iter()
            .find(|item| item.id == "open_settings")
            .unwrap();
        recents.record(settings, 10, 20);

        let filtered = filter_items(&items, &recents, "");
        assert_eq!(
            filtered.first().map(|item| item.id.as_str()),
            Some("open_settings")
        );
    }

    #[test]
    fn filter_dedupes_stable_ids() {
        let args = vec!["test".to_string()];
        let first = custom_command_item("Test Runner", Some("Ctrl+R"), "cargo", &args);
        let renamed = custom_command_item("Run Tests", Some("Ctrl+R"), "cargo", &args);

        let filtered = filter_items(&[first, renamed], &RecentCommandStore::default(), "");

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].title, "Test Runner");
    }

    #[test]
    fn search_result_items_include_file_payloads() {
        let results = vec![crate::search::SearchResult {
            path: "/repo/src/main.rs".to_string(),
            name: "main.rs".to_string(),
            line_number: None,
            line_content: None,
            column_start: None,
            column_end: None,
            match_type: "file".to_string(),
        }];

        let items = search_result_items("/repo", &results);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "src/main.rs");
        assert_eq!(items[0].category, "Files");
        assert_eq!(items[0].source, CommandPaletteSource::Dynamic);
        assert_eq!(
            items[0].payload.get("path").map(String::as_str),
            Some("/repo/src/main.rs")
        );
        assert_eq!(
            items[0].payload.get("kind").map(String::as_str),
            Some("file")
        );
    }

    #[test]
    fn search_result_items_include_content_payloads() {
        let results = vec![crate::search::SearchResult {
            path: "/repo/src/lib.rs".to_string(),
            name: "lib.rs".to_string(),
            line_number: Some(42),
            line_content: Some("pub fn search_items()".to_string()),
            column_start: Some(7),
            column_end: Some(19),
            match_type: "content".to_string(),
        }];

        let items = search_result_items("/repo", &results);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "src/lib.rs:42");
        assert_eq!(items[0].category, "Project Search");
        assert_eq!(items[0].payload.get("line").map(String::as_str), Some("42"));
        assert!(items[0]
            .keywords
            .iter()
            .any(|keyword| keyword.contains("search_items")));
    }
}
