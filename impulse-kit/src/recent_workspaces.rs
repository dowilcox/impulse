//! Every workspace folder ever opened, newest first, for the workspace
//! switcher (the Swift `RecentWorkspaces`), kept until cleared. Stored as a
//! JSON list of paths (`recent-workspaces.json` in the data directory).

use std::path::Path;

/// The list in the file; empty when it's missing or unreadable.
pub fn load(file: &Path) -> Vec<String> {
    std::fs::read(file)
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

fn save(file: &Path, folders: &[String]) {
    if let Ok(data) = serde_json::to_vec_pretty(folders) {
        let _ = crate::settings::write_private_file(file, &data);
    }
}

/// Put `folder` first.
pub fn note(file: &Path, folder: &str) {
    let mut list: Vec<String> = load(file).into_iter().filter(|f| f != folder).collect();
    list.insert(0, folder.to_string());
    save(file, &list);
}

pub fn forget(file: &Path, folder: &str) {
    let list: Vec<String> = load(file).into_iter().filter(|f| f != folder).collect();
    save(file, &list);
}

/// Empty the list, returning what it held for `restore`.
pub fn clear(file: &Path) -> Vec<String> {
    let cleared = load(file);
    let _ = std::fs::remove_file(file);
    cleared
}

/// Put back a cleared list behind any folder opened since.
pub fn restore(file: &Path, cleared: &[String]) {
    let mut list = load(file);
    let extra: Vec<String> = cleared
        .iter()
        .filter(|f| !list.contains(f))
        .cloned()
        .collect();
    list.extend(extra);
    save(file, &list);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_clears_and_restores() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("recent-workspaces.json");
        assert!(load(&file).is_empty());
        note(&file, "/a");
        note(&file, "/b");
        note(&file, "/a");
        assert_eq!(load(&file), ["/a", "/b"]);
        forget(&file, "/b");
        assert_eq!(load(&file), ["/a"]);
        note(&file, "/b");
        let cleared = clear(&file);
        assert_eq!(cleared, ["/b", "/a"]);
        assert!(load(&file).is_empty());
        note(&file, "/c");
        note(&file, "/a");
        restore(&file, &cleared);
        assert_eq!(load(&file), ["/a", "/c", "/b"]);
    }
}
