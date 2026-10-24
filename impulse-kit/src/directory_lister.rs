//! One level of a folder for the file tree (the Swift `DirectoryLister`,
//! from the listing part of `impulse-core/src/filesystem.rs`). Git-status
//! enrichment is composed by the caller.
//!
//! Entries come sorted directories first, then by lowercased name in byte
//! order. Hidden files are left out unless asked for; OS metadata files
//! (`.DS_Store`, `Thumbs.db`, `desktop.ini`) always are. A symlink is
//! described as itself, not its target: never a directory, its own size.

use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    #[serde(default)]
    pub is_symlink: bool,
    #[serde(default)]
    pub size: u64,
    /// Seconds since the epoch; 0 when unknown.
    #[serde(default)]
    pub modified: u64,
    #[serde(default)]
    pub git_status: Option<String>,
}

const ALWAYS_HIDDEN: [&str; 3] = [".DS_Store", "Thumbs.db", "desktop.ini"];

/// Read a folder's entries. Names that aren't UTF-8 are shown with
/// replacement characters.
pub fn read_directory_entries(path: &str, show_hidden: bool) -> Result<Vec<FileEntry>, String> {
    if !Path::new(path).is_dir() {
        return Err(format!("Not a directory: {path}"));
    }
    let reader = fs::read_dir(path).map_err(|e| format!("Failed to read directory: {e}"))?;

    let mut entries = Vec::new();
    for entry in reader.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        if ALWAYS_HIDDEN.contains(&name.as_str()) {
            continue;
        }
        let entry_path = Path::new(path).join(&name);
        let Ok(metadata) = fs::symlink_metadata(&entry_path) else {
            continue;
        };
        let is_symlink = metadata.file_type().is_symlink();
        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        entries.push(FileEntry {
            name,
            path: entry_path.to_string_lossy().into_owned(),
            is_dir: !is_symlink && metadata.is_dir(),
            is_symlink,
            size: metadata.len(),
            modified,
            git_status: None,
        });
    }

    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_directories_first_and_hides_dotfiles() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir(root.join("zeta")).unwrap();
        fs::write(root.join("Beta.txt"), "b").unwrap();
        fs::write(root.join("alpha.txt"), "a").unwrap();
        fs::write(root.join(".hidden"), "h").unwrap();
        fs::write(root.join(".DS_Store"), "x").unwrap();
        let root_str = root.to_str().unwrap();

        let names = |entries: Vec<FileEntry>| -> Vec<String> {
            entries.into_iter().map(|e| e.name).collect()
        };
        assert_eq!(
            names(read_directory_entries(root_str, false).unwrap()),
            vec!["zeta", "alpha.txt", "Beta.txt"]
        );
        assert_eq!(
            names(read_directory_entries(root_str, true).unwrap()),
            vec!["zeta", ".hidden", "alpha.txt", "Beta.txt"]
        );
        let entries = read_directory_entries(root_str, false).unwrap();
        assert!(entries[0].is_dir);
        assert_eq!(entries[1].size, 1);
        assert_eq!(entries[1].path, format!("{root_str}/alpha.txt"));
        assert!(read_directory_entries(&format!("{root_str}/alpha.txt"), false).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_describe_themselves() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir(root.join("real")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
        let entries = read_directory_entries(root.to_str().unwrap(), false).unwrap();
        let link = entries.iter().find(|e| e.name == "link").unwrap();
        assert!(link.is_symlink);
        assert!(!link.is_dir);
    }
}
