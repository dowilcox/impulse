//! Parity of the workbench modules (command palette, file tree, Monaco and
//! Markdown themes) with the golden fixtures shared with the Swift port,
//! read in place from the Swift test tree. Never regenerate the fixtures
//! from either port's output.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use impulse_kit::command_palette::{
    builtin_items, custom_command_item, CommandPaletteItem, RecentCommandStore,
};
use impulse_kit::directory_lister::{read_directory_entries, FileEntry};
use impulse_kit::file_tree::{
    build_patch_batch_from_filesystem_with, stable_node_id, FileTreeWatchEvent,
};
use impulse_kit::markdown_theme::theme_to_markdown_colors;
use impulse_kit::monaco_theme::theme_to_monaco;
use impulse_kit::theme::{builtin_theme, builtin_theme_names, theme_display_name};
use serde::Deserialize;
use serde_json::Value;

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../impulse-macos/Tests/ImpulseKitTests/Fixtures")
}

fn fixture(name: &str) -> String {
    let path = fixtures_root().join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn fixture_json(name: &str) -> Value {
    serde_json::from_str(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// Every fixture JSON file, relative to the fixtures root, sorted.
fn all_fixture_paths() -> Vec<String> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else if path.extension().is_some_and(|e| e == "json") {
                let relative = path.strip_prefix(root).unwrap();
                out.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let root = fixtures_root();
    let mut out = Vec::new();
    walk(&root, &root, &mut out);
    out.sort();
    out
}

#[test]
fn fixtures_are_present_and_parse() {
    let paths = all_fixture_paths();
    assert!(
        paths.len() >= 60,
        "expected the full fixture corpus, found {}",
        paths.len()
    );
    for name in [
        "shell_parser.json",
        "close_risk.json",
        "glob.json",
        "palette_items.json",
        "file_tree_patch.json",
        "path_to_uri.json",
        "language_from_uri.json",
        "themes/nord.monaco.json",
        "themes/display_names.json",
    ] {
        assert!(paths.iter().any(|p| p == name), "missing fixture {name}");
    }
    for path in &paths {
        fixture_json(path);
    }
    let monaco = paths
        .iter()
        .filter(|p| p.starts_with("themes/") && p.ends_with(".monaco.json"))
        .count();
    assert_eq!(monaco, 19, "expected 19 built-in themes");
}

// Command palette

#[test]
fn builtin_palette_items_match_fixture() {
    let expected: Vec<CommandPaletteItem> =
        serde_json::from_str(&fixture("palette_items.json")).unwrap();
    assert_eq!(builtin_items(), expected);
    // And as JSON: optional fields left out, as the fixture has them.
    assert_eq!(
        serde_json::to_value(builtin_items()).unwrap(),
        fixture_json("palette_items.json")
    );
}

#[test]
fn custom_palette_item_matches_fixture() {
    let item = custom_command_item(
        "Deploy Staging",
        Some("cmd+shift+d"),
        "./scripts/deploy.sh",
        &["staging".to_string(), "--verbose".to_string()],
    );
    assert_eq!(
        serde_json::to_value(&item).unwrap(),
        fixture_json("palette_custom_item.json")
    );
}

#[test]
fn palette_recents_match_fixture() {
    #[derive(Deserialize)]
    struct Recents {
        store: RecentCommandStore,
    }
    let expected: Recents = serde_json::from_str(&fixture("palette_recents.json")).unwrap();
    let items = builtin_items();
    let now_ms: u64 = 1_700_000_000_000;
    let mut recents = RecentCommandStore::default();
    recents.record(&items[4], now_ms - 60_000, 50);
    recents.record(&items[1], now_ms, 50);
    assert_eq!(recents, expected.store);
    assert_eq!(
        serde_json::to_value(&recents).unwrap(),
        fixture_json("palette_recents.json")["store"]
    );
}

// File tree

#[test]
fn stable_node_ids_match_fixture() {
    #[derive(Deserialize)]
    struct Case {
        path: String,
        id: String,
    }
    let cases: Vec<Case> = serde_json::from_str(&fixture("stable_node_id.json")).unwrap();
    assert!(!cases.is_empty());
    for c in cases {
        assert_eq!(stable_node_id(&c.path), c.id, "path={:?}", c.path);
    }
}

/// The fixture was captured on APFS, where a folder's size grows with its
/// entries (64 bytes plus 32 per entry); other file systems report a fixed
/// size, which would hide the change to `alpha`. Folders are read with
/// APFS's sizes so the scenario replays the same way everywhere.
fn read_with_apfs_folder_sizes(path: &str, show_hidden: bool) -> Option<Vec<FileEntry>> {
    let mut entries = read_directory_entries(path, show_hidden).ok()?;
    for entry in entries.iter_mut().filter(|e| e.is_dir) {
        entry.size = 64 + 32 * fs::read_dir(&entry.path).ok()?.count() as u64;
    }
    Some(entries)
}

/// `$ROOT` for the scenario's folder, and no `modified` times (they depend
/// on when the test runs).
fn normalized(value: &Value, root: &str) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| key.as_str() != "modified")
                .map(|(key, v)| (key.clone(), normalized(v, root)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(|v| normalized(v, root)).collect()),
        Value::String(s) => Value::String(s.replace(root, "$ROOT")),
        other => other.clone(),
    }
}

#[test]
fn file_tree_patch_batch_matches_fixture() {
    let fixture = fixture_json("file_tree_patch.json");
    let scratch = tempfile::tempdir().unwrap();
    let root_path = scratch.path().canonicalize().unwrap();
    let root = root_path.to_str().unwrap().to_string();
    let write =
        |relative: &str, contents: &str| fs::write(root_path.join(relative), contents).unwrap();

    fs::create_dir(root_path.join("alpha")).unwrap();
    fs::create_dir(root_path.join("zeta")).unwrap();
    write("alpha/a1.txt", "alpha one\n");
    write("beta.txt", "beta\n");
    write("zeta/z1.txt", "zeta one\n");
    write(".hidden", "hidden\n");

    let alpha = format!("{root}/alpha");
    let before_by_parent: HashMap<String, Vec<FileEntry>> = HashMap::from([
        (
            root.clone(),
            read_with_apfs_folder_sizes(&root, false).unwrap(),
        ),
        (
            alpha.clone(),
            read_with_apfs_folder_sizes(&alpha, false).unwrap(),
        ),
    ]);
    assert_eq!(
        normalized(
            &serde_json::to_value(&before_by_parent[&root]).unwrap(),
            &root
        ),
        fixture["before_by_parent"]["root"]
    );
    assert_eq!(
        normalized(
            &serde_json::to_value(&before_by_parent[&alpha]).unwrap(),
            &root
        ),
        fixture["before_by_parent"]["alpha"]
    );

    write("gamma.txt", "gamma\n");
    fs::remove_file(root_path.join("beta.txt")).unwrap();
    write("alpha/a2.txt", "alpha two\n");
    write("alpha/a1.txt", "alpha one, edited\n");

    let events: Vec<FileTreeWatchEvent> = serde_json::from_str(
        &serde_json::to_string(&fixture["events"])
            .unwrap()
            .replace("$ROOT", &root),
    )
    .unwrap();
    assert_eq!(events.len(), 4);

    let batch = build_patch_batch_from_filesystem_with(
        &root,
        &events,
        &before_by_parent,
        false,
        read_with_apfs_folder_sizes,
    )
    .unwrap();
    assert_eq!(
        normalized(&serde_json::to_value(&batch).unwrap(), &root),
        normalized(&fixture["batch"], &root),
        "patch batch diverges from the fixture"
    );
}

// Themes

#[test]
fn builtin_theme_names_match_display_names_fixture() {
    let display_names: HashMap<String, String> =
        serde_json::from_str(&fixture("themes/display_names.json")).unwrap();
    let names = builtin_theme_names();
    assert_eq!(names.len(), 19);
    let mut sorted: Vec<&str> = names.clone();
    sorted.sort();
    let mut fixture_ids: Vec<&str> = display_names.keys().map(String::as_str).collect();
    fixture_ids.sort();
    assert_eq!(sorted, fixture_ids);
    // The registry's order.
    assert_eq!(
        names,
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
            "harbor",
        ]
    );
    for (id, want) in &display_names {
        assert_eq!(&theme_display_name(id), want, "display name for {id}");
    }
}

#[test]
fn monaco_themes_match_fixtures() {
    for id in builtin_theme_names() {
        let theme = builtin_theme(id).unwrap();
        let actual = serde_json::to_value(theme_to_monaco(&theme)).unwrap();
        let expected = fixture_json(&format!("themes/{id}.monaco.json"));
        assert_eq!(actual, expected, "Monaco theme {id}");
    }
}

#[test]
fn markdown_colors_match_fixtures() {
    for id in builtin_theme_names() {
        let theme = builtin_theme(id).unwrap();
        let actual = serde_json::to_value(theme_to_markdown_colors(&theme)).unwrap();
        let expected = fixture_json(&format!("themes/{id}.markdown.json"));
        assert_eq!(actual, expected, "Markdown colors {id}");
    }
}
