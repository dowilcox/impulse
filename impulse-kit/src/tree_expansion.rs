//! The file tree's open folders, remembered per project (the Swift
//! `FileTreeDataController`'s `impulse.fileTree.expandedPaths.<root>`
//! defaults): `file-tree-expanded.json` in the data directory, a map from
//! each root to the folders open under it, so a project opens with the
//! folders you left open.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

type Store = BTreeMap<String, Vec<String>>;

fn read(file: &Path) -> Store {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// The folders open under `root` (ones still there, inside it), shallow
/// first so each one's parent is listed before it.
pub fn load(file: &Path, root: &Path) -> Vec<PathBuf> {
    let key = root.to_string_lossy();
    let mut paths: Vec<PathBuf> = read(file)
        .remove(key.as_ref())
        .unwrap_or_default()
        .into_iter()
        .map(PathBuf::from)
        .filter(|p| p.starts_with(root) && p != root && p.is_dir())
        .collect();
    paths.sort_by_key(|p| p.components().count());
    paths
}

/// Remember `paths` as the folders open under `root` (none forgets it).
pub fn save<'a>(file: &Path, root: &Path, paths: impl IntoIterator<Item = &'a PathBuf>) {
    let mut store = read(file);
    let mut list: Vec<String> = paths
        .into_iter()
        .filter(|p| p.starts_with(root))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    list.sort();
    let key = root.to_string_lossy().into_owned();
    if list.is_empty() {
        if store.remove(&key).is_none() {
            return;
        }
    } else if store.get(&key) == Some(&list) {
        return;
    } else {
        store.insert(key, list);
    }
    if let Ok(data) = serde_json::to_vec_pretty(&store) {
        let _ = crate::settings::write_private_file(file, &data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_open_folders_per_root() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("data/file-tree-expanded.json");
        let root = dir.path().join("proj");
        let other = dir.path().join("other");
        for folder in ["proj/src/lib", "proj/test", "other/docs"] {
            std::fs::create_dir_all(dir.path().join(folder)).unwrap();
        }
        assert!(load(&file, &root).is_empty());
        let open = [root.join("src/lib"), root.join("src"), root.join("test")];
        save(&file, &root, &open);
        save(&file, &other, &[other.join("docs")]);
        assert_eq!(
            load(&file, &root),
            vec![root.join("src"), root.join("test"), root.join("src/lib")]
        );
        assert_eq!(load(&file, &other), vec![other.join("docs")]);
        // A folder deleted since, or outside the root, isn't opened.
        std::fs::remove_dir(root.join("test")).unwrap();
        assert_eq!(
            load(&file, &root),
            vec![root.join("src"), root.join("src/lib")]
        );
        save(&file, &root, &[]);
        assert!(load(&file, &root).is_empty());
        assert_eq!(load(&file, &other), vec![other.join("docs")]);
    }
}
