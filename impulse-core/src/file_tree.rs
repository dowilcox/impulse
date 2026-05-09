use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::filesystem::FileEntry;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FileTreeNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: u64,
    pub git_status: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FileTreePatch {
    pub parent_id: String,
    pub operations: Vec<FileTreeOperation>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct FileTreePatchBatch {
    pub root_id: String,
    pub patches: Vec<FileTreePatch>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FileTreeOperation {
    Remove {
        id: String,
    },
    Upsert {
        parent_id: String,
        index: usize,
        node: FileTreeNode,
    },
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct FileTreeViewState {
    pub expanded_ids: Vec<String>,
    pub selected_id: Option<String>,
    pub scroll_offset: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileTreeWatchEventKind {
    Create,
    Modify,
    Remove,
    Rename,
    Any,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FileTreeWatchEvent {
    pub kind: FileTreeWatchEventKind,
    pub paths: Vec<String>,
}

pub fn stable_node_id(path: &str) -> String {
    let trimmed = path.trim_end_matches(|ch| ch == '/' || ch == '\\');
    if trimmed.is_empty() {
        path.to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn affected_parent_paths(root_path: &str, events: &[FileTreeWatchEvent]) -> Vec<String> {
    let empty_snapshots: HashMap<String, Vec<FileEntry>> = HashMap::new();
    affected_parent_paths_with_snapshots(root_path, events, &empty_snapshots, &empty_snapshots)
}

pub fn build_patch_batch(
    root_path: &str,
    events: &[FileTreeWatchEvent],
    before_by_parent: &HashMap<String, Vec<FileEntry>>,
    after_by_parent: &HashMap<String, Vec<FileEntry>>,
) -> FileTreePatchBatch {
    let parent_paths =
        affected_parent_paths_with_snapshots(root_path, events, before_by_parent, after_by_parent);
    let patches = parent_paths
        .iter()
        .filter_map(|parent_path| {
            let before = before_by_parent
                .get(parent_path)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let after = after_by_parent
                .get(parent_path)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let patch = build_child_patch(parent_path, before, after);
            if patch.operations.is_empty() {
                None
            } else {
                Some(patch)
            }
        })
        .collect();

    FileTreePatchBatch {
        root_id: stable_node_id(root_path),
        patches,
    }
}

pub fn build_patch_batch_from_filesystem(
    root_path: &str,
    events: &[FileTreeWatchEvent],
    before_by_parent: &HashMap<String, Vec<FileEntry>>,
    show_hidden: bool,
) -> Result<FileTreePatchBatch, String> {
    let empty_snapshots: HashMap<String, Vec<FileEntry>> = HashMap::new();
    let parent_paths =
        affected_parent_paths_with_snapshots(root_path, events, before_by_parent, &empty_snapshots);
    let mut after_by_parent = HashMap::new();

    for parent_path in &parent_paths {
        let after = if Path::new(parent_path).is_dir() {
            crate::filesystem::read_directory_with_git_status(parent_path, show_hidden)?
        } else {
            Vec::new()
        };
        after_by_parent.insert(parent_path.clone(), after);
    }

    Ok(build_patch_batch(
        root_path,
        events,
        before_by_parent,
        &after_by_parent,
    ))
}

pub fn node_from_entry(parent_path: &str, entry: &FileEntry) -> FileTreeNode {
    FileTreeNode {
        id: stable_node_id(&entry.path),
        parent_id: Some(stable_node_id(parent_path)),
        name: entry.name.clone(),
        path: entry.path.clone(),
        is_dir: entry.is_dir,
        is_symlink: entry.is_symlink,
        size: entry.size,
        modified: entry.modified,
        git_status: entry.git_status.clone(),
    }
}

pub fn build_child_patch(
    parent_path: &str,
    before: &[FileEntry],
    after: &[FileEntry],
) -> FileTreePatch {
    let parent_id = stable_node_id(parent_path);
    let before_nodes: Vec<FileTreeNode> = before
        .iter()
        .map(|entry| node_from_entry(parent_path, entry))
        .collect();
    let after_nodes: Vec<FileTreeNode> = after
        .iter()
        .map(|entry| node_from_entry(parent_path, entry))
        .collect();

    let before_by_id: HashMap<&str, (usize, &FileTreeNode)> = before_nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.as_str(), (index, node)))
        .collect();
    let after_by_id: HashMap<&str, (usize, &FileTreeNode)> = after_nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.as_str(), (index, node)))
        .collect();

    let mut operations = Vec::new();
    let mut replace_ids = HashSet::new();

    for node in &before_nodes {
        match after_by_id.get(node.id.as_str()) {
            Some((_, after_node)) if requires_replacement(node, after_node) => {
                replace_ids.insert(node.id.as_str());
                operations.push(FileTreeOperation::Remove {
                    id: node.id.clone(),
                });
            }
            None => operations.push(FileTreeOperation::Remove {
                id: node.id.clone(),
            }),
            Some(_) => {}
        }
    }

    for (index, node) in after_nodes.into_iter().enumerate() {
        let should_upsert = match before_by_id.get(node.id.as_str()) {
            Some((before_index, before_node)) => {
                replace_ids.contains(node.id.as_str())
                    || *before_index != index
                    || *before_node != &node
            }
            None => true,
        };
        if should_upsert {
            operations.push(FileTreeOperation::Upsert {
                parent_id: parent_id.clone(),
                index,
                node,
            });
        }
    }

    FileTreePatch {
        parent_id,
        operations,
    }
}

pub fn reconcile_view_state(state: &FileTreeViewState, patch: &FileTreePatch) -> FileTreeViewState {
    let removed: HashSet<&str> = patch
        .operations
        .iter()
        .filter_map(|operation| match operation {
            FileTreeOperation::Remove { id } => Some(id.as_str()),
            FileTreeOperation::Upsert { .. } => None,
        })
        .collect();
    let upserted: HashSet<&str> = patch
        .operations
        .iter()
        .filter_map(|operation| match operation {
            FileTreeOperation::Remove { .. } => None,
            FileTreeOperation::Upsert { node, .. } => Some(node.id.as_str()),
        })
        .collect();

    let removed_only: HashSet<&str> = removed.difference(&upserted).copied().collect();
    let upserted_as_files: HashSet<&str> = patch
        .operations
        .iter()
        .filter_map(|operation| match operation {
            FileTreeOperation::Remove { .. } => None,
            FileTreeOperation::Upsert { node, .. } if !node.is_dir => Some(node.id.as_str()),
            FileTreeOperation::Upsert { .. } => None,
        })
        .collect();
    FileTreeViewState {
        expanded_ids: state
            .expanded_ids
            .iter()
            .filter(|id| {
                !is_removed_or_descendant(id, &removed_only)
                    && !is_removed_or_descendant(id, &upserted_as_files)
            })
            .cloned()
            .collect(),
        selected_id: state
            .selected_id
            .as_ref()
            .filter(|id| !is_removed_or_descendant(id, &removed_only))
            .cloned(),
        scroll_offset: state.scroll_offset,
    }
}

pub fn reconcile_view_state_for_batch(
    state: &FileTreeViewState,
    batch: &FileTreePatchBatch,
) -> FileTreeViewState {
    batch.patches.iter().fold(state.clone(), |state, patch| {
        reconcile_view_state(&state, patch)
    })
}

fn requires_replacement(before: &FileTreeNode, after: &FileTreeNode) -> bool {
    before.is_dir != after.is_dir || before.is_symlink != after.is_symlink
}

fn affected_parent_paths_with_snapshots(
    root_path: &str,
    events: &[FileTreeWatchEvent],
    before_by_parent: &HashMap<String, Vec<FileEntry>>,
    after_by_parent: &HashMap<String, Vec<FileEntry>>,
) -> Vec<String> {
    let root = normalized_path(root_path);
    let mut parents = HashSet::new();

    for event in events {
        for path in &event.paths {
            let path = normalized_path(path);
            let parent = event_parent_path(&root, &path);
            parents.insert(parent);

            if before_by_parent.contains_key(&path) || after_by_parent.contains_key(&path) {
                parents.insert(path);
            }
        }
    }

    let mut parents: Vec<String> = parents.into_iter().collect();
    parents.sort_by(|left, right| {
        path_depth(left)
            .cmp(&path_depth(right))
            .then_with(|| left.cmp(right))
    });
    parents
}

fn event_parent_path(root: &str, path: &str) -> String {
    if path == root || !path_is_within_root(root, path) {
        return root.to_string();
    }

    Path::new(path)
        .parent()
        .map(path_to_string)
        .filter(|parent| !parent.is_empty() && path_is_within_root(root, parent))
        .unwrap_or_else(|| root.to_string())
}

fn normalized_path(path: &str) -> String {
    stable_node_id(&path_to_string(Path::new(path)))
}

fn path_to_string(path: &Path) -> String {
    let text = path.to_string_lossy().to_string();
    if text.is_empty() {
        ".".to_string()
    } else {
        text
    }
}

fn path_depth(path: &str) -> usize {
    Path::new(path).components().count()
}

fn path_is_within_root(root: &str, path: &str) -> bool {
    let root = Path::new(root);
    let path = Path::new(path);
    path == root || path.starts_with(root)
}

fn is_removed_or_descendant(id: &str, removed_ids: &HashSet<&str>) -> bool {
    removed_ids
        .iter()
        .any(|removed_id| id == *removed_id || path_is_descendant(id, removed_id))
}

fn path_is_descendant(path: &str, ancestor: &str) -> bool {
    PathBuf::from(path).starts_with(ancestor) && path != ancestor
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, path: &str, is_dir: bool) -> FileEntry {
        FileEntry {
            name: name.to_string(),
            path: path.to_string(),
            is_dir,
            is_symlink: false,
            size: 10,
            modified: 20,
            git_status: None,
        }
    }

    fn entry_with_status(name: &str, path: &str, is_dir: bool, status: &str) -> FileEntry {
        let mut entry = entry(name, path, is_dir);
        entry.git_status = Some(status.to_string());
        entry
    }

    fn event(kind: FileTreeWatchEventKind, paths: &[&str]) -> FileTreeWatchEvent {
        FileTreeWatchEvent {
            kind,
            paths: paths.iter().map(|path| (*path).to_string()).collect(),
        }
    }

    #[test]
    fn unchanged_children_emit_no_operations() {
        let before = vec![
            entry("src", "/repo/src", true),
            entry("main.rs", "/repo/main.rs", false),
        ];
        let after = before.clone();

        let patch = build_child_patch("/repo", &before, &after);

        assert!(patch.operations.is_empty());
    }

    #[test]
    fn rename_is_remove_before_upsert() {
        let before = vec![entry("old.rs", "/repo/old.rs", false)];
        let after = vec![entry("new.rs", "/repo/new.rs", false)];

        let patch = build_child_patch("/repo", &before, &after);

        assert_eq!(patch.operations.len(), 2);
        assert!(matches!(
            &patch.operations[0],
            FileTreeOperation::Remove { id } if id == "/repo/old.rs"
        ));
        assert!(matches!(
            &patch.operations[1],
            FileTreeOperation::Upsert { index, node, .. } if *index == 0 && node.id == "/repo/new.rs"
        ));
    }

    #[test]
    fn batch_represents_same_parent_rename() {
        let events = vec![event(
            FileTreeWatchEventKind::Rename,
            &["/repo/old.rs", "/repo/new.rs"],
        )];
        let mut before = HashMap::new();
        before.insert(
            "/repo".to_string(),
            vec![entry("old.rs", "/repo/old.rs", false)],
        );
        let mut after = HashMap::new();
        after.insert(
            "/repo".to_string(),
            vec![entry("new.rs", "/repo/new.rs", false)],
        );

        let batch = build_patch_batch("/repo", &events, &before, &after);

        assert_eq!(batch.patches.len(), 1);
        assert_eq!(batch.patches[0].operations.len(), 2);
        assert!(matches!(
            &batch.patches[0].operations[0],
            FileTreeOperation::Remove { id } if id == "/repo/old.rs"
        ));
        assert!(matches!(
            &batch.patches[0].operations[1],
            FileTreeOperation::Upsert { index, node, .. } if *index == 0 && node.id == "/repo/new.rs"
        ));
    }

    #[test]
    fn batch_represents_delete_without_upsert() {
        let events = vec![event(FileTreeWatchEventKind::Remove, &["/repo/doomed.rs"])];
        let mut before = HashMap::new();
        before.insert(
            "/repo".to_string(),
            vec![
                entry("keep.rs", "/repo/keep.rs", false),
                entry("doomed.rs", "/repo/doomed.rs", false),
            ],
        );
        let mut after = HashMap::new();
        after.insert(
            "/repo".to_string(),
            vec![entry("keep.rs", "/repo/keep.rs", false)],
        );

        let batch = build_patch_batch("/repo", &events, &before, &after);

        assert_eq!(batch.patches.len(), 1);
        assert_eq!(batch.patches[0].operations.len(), 1);
        assert!(matches!(
            &batch.patches[0].operations[0],
            FileTreeOperation::Remove { id } if id == "/repo/doomed.rs"
        ));
    }

    #[test]
    fn metadata_or_position_changes_emit_upsert() {
        let before = vec![
            entry("a.rs", "/repo/a.rs", false),
            entry("b.rs", "/repo/b.rs", false),
        ];
        let mut after = vec![
            entry("b.rs", "/repo/b.rs", false),
            entry("a.rs", "/repo/a.rs", false),
        ];
        after[1].git_status = Some("M".to_string());

        let patch = build_child_patch("/repo", &before, &after);

        assert_eq!(patch.operations.len(), 2);
        assert!(patch
            .operations
            .iter()
            .all(|operation| { matches!(operation, FileTreeOperation::Upsert { .. }) }));
    }

    #[test]
    fn git_status_refresh_emits_upsert_only() {
        let before = vec![entry("lib.rs", "/repo/lib.rs", false)];
        let after = vec![entry_with_status("lib.rs", "/repo/lib.rs", false, "M")];

        let patch = build_child_patch("/repo", &before, &after);

        assert_eq!(patch.operations.len(), 1);
        assert!(matches!(
            &patch.operations[0],
            FileTreeOperation::Upsert { index, node, .. }
                if *index == 0
                    && node.id == "/repo/lib.rs"
                    && node.git_status.as_deref() == Some("M")
        ));
    }

    #[test]
    fn type_replacement_removes_before_upsert() {
        let before = vec![entry("target", "/repo/target", true)];
        let after = vec![entry("target", "/repo/target", false)];

        let patch = build_child_patch("/repo", &before, &after);

        assert_eq!(patch.operations.len(), 2);
        assert!(matches!(
            &patch.operations[0],
            FileTreeOperation::Remove { id } if id == "/repo/target"
        ));
        assert!(matches!(
            &patch.operations[1],
            FileTreeOperation::Upsert { node, .. } if node.id == "/repo/target" && !node.is_dir
        ));
    }

    #[test]
    fn view_state_drops_removed_ids_but_preserves_scroll_and_replacements() {
        let state = FileTreeViewState {
            expanded_ids: vec!["/repo/remove".to_string(), "/repo/replace".to_string()],
            selected_id: Some("/repo/remove".to_string()),
            scroll_offset: 42.5,
        };
        let before = vec![
            entry("remove", "/repo/remove", true),
            entry("replace", "/repo/replace", true),
        ];
        let after = vec![entry("replace", "/repo/replace", false)];
        let patch = build_child_patch("/repo", &before, &after);

        let reconciled = reconcile_view_state(&state, &patch);

        assert!(reconciled.expanded_ids.is_empty());
        assert_eq!(reconciled.selected_id, None);
        assert_eq!(reconciled.scroll_offset, 42.5);
    }

    #[test]
    fn affected_parent_paths_are_stable_and_deduped() {
        let events = vec![
            event(FileTreeWatchEventKind::Modify, &["/repo/src/lib.rs"]),
            event(FileTreeWatchEventKind::Create, &["/repo/src/main.rs"]),
            event(FileTreeWatchEventKind::Remove, &["/tmp/outside.txt"]),
        ];

        let parents = affected_parent_paths("/repo", &events);

        assert_eq!(parents, vec!["/repo", "/repo/src"]);
    }

    #[test]
    fn loaded_directory_event_refreshes_that_directory_too() {
        let events = vec![event(FileTreeWatchEventKind::Modify, &["/repo/src"])];
        let mut before = HashMap::new();
        before.insert(
            "/repo/src".to_string(),
            vec![entry("old.rs", "/repo/src/old.rs", false)],
        );
        let mut after = HashMap::new();
        after.insert(
            "/repo/src".to_string(),
            vec![entry("new.rs", "/repo/src/new.rs", false)],
        );

        let batch = build_patch_batch("/repo", &events, &before, &after);

        assert_eq!(batch.patches.len(), 1);
        assert_eq!(batch.patches[0].parent_id, "/repo/src");
        assert_eq!(batch.patches[0].operations.len(), 2);
    }

    #[test]
    fn batch_coalesces_multiple_events_for_one_parent() {
        let events = vec![
            event(FileTreeWatchEventKind::Create, &["/repo/a.rs"]),
            event(FileTreeWatchEventKind::Modify, &["/repo/b.rs"]),
        ];
        let mut before = HashMap::new();
        before.insert(
            "/repo".to_string(),
            vec![entry("b.rs", "/repo/b.rs", false)],
        );
        let mut after = HashMap::new();
        let mut b = entry("b.rs", "/repo/b.rs", false);
        b.git_status = Some("M".to_string());
        after.insert(
            "/repo".to_string(),
            vec![entry("a.rs", "/repo/a.rs", false), b],
        );

        let batch = build_patch_batch("/repo", &events, &before, &after);

        assert_eq!(batch.patches.len(), 1);
        assert_eq!(batch.patches[0].parent_id, "/repo");
        assert_eq!(batch.patches[0].operations.len(), 2);
    }

    #[test]
    fn batch_represents_move_across_parents() {
        let events = vec![event(
            FileTreeWatchEventKind::Rename,
            &["/repo/src/item.rs", "/repo/tests/item.rs"],
        )];
        let mut before = HashMap::new();
        before.insert(
            "/repo/src".to_string(),
            vec![entry("item.rs", "/repo/src/item.rs", false)],
        );
        before.insert("/repo/tests".to_string(), Vec::new());
        let mut after = HashMap::new();
        after.insert("/repo/src".to_string(), Vec::new());
        after.insert(
            "/repo/tests".to_string(),
            vec![entry("item.rs", "/repo/tests/item.rs", false)],
        );

        let batch = build_patch_batch("/repo", &events, &before, &after);

        assert_eq!(batch.patches.len(), 2);
        assert!(matches!(
            &batch.patches[0].operations[0],
            FileTreeOperation::Remove { id } if id == "/repo/src/item.rs"
        ));
        assert!(matches!(
            &batch.patches[1].operations[0],
            FileTreeOperation::Upsert { node, .. } if node.id == "/repo/tests/item.rs"
        ));
    }

    #[test]
    fn batch_view_state_drops_removed_descendants() {
        let state = FileTreeViewState {
            expanded_ids: vec![
                "/repo/src".to_string(),
                "/repo/src/nested".to_string(),
                "/repo/tests".to_string(),
            ],
            selected_id: Some("/repo/src/nested/lib.rs".to_string()),
            scroll_offset: 12.0,
        };
        let before = vec![
            entry("src", "/repo/src", true),
            entry("tests", "/repo/tests", true),
        ];
        let after = vec![entry("tests", "/repo/tests", true)];
        let batch = FileTreePatchBatch {
            root_id: "/repo".to_string(),
            patches: vec![build_child_patch("/repo", &before, &after)],
        };

        let reconciled = reconcile_view_state_for_batch(&state, &batch);

        assert_eq!(reconciled.expanded_ids, vec!["/repo/tests"]);
        assert_eq!(reconciled.selected_id, None);
        assert_eq!(reconciled.scroll_offset, 12.0);
    }

    #[test]
    fn nested_directory_replacement_removes_loaded_descendants() {
        let events = vec![event(FileTreeWatchEventKind::Modify, &["/repo/src"])];
        let mut before = HashMap::new();
        before.insert("/repo".to_string(), vec![entry("src", "/repo/src", true)]);
        before.insert(
            "/repo/src".to_string(),
            vec![entry("lib.rs", "/repo/src/lib.rs", false)],
        );
        let mut after = HashMap::new();
        after.insert("/repo".to_string(), vec![entry("src", "/repo/src", false)]);
        after.insert("/repo/src".to_string(), Vec::new());

        let batch = build_patch_batch("/repo", &events, &before, &after);
        let state = FileTreeViewState {
            expanded_ids: vec!["/repo/src".to_string()],
            selected_id: Some("/repo/src/lib.rs".to_string()),
            scroll_offset: 99.0,
        };
        let reconciled = reconcile_view_state_for_batch(&state, &batch);

        assert_eq!(batch.patches.len(), 2);
        assert!(matches!(
            &batch.patches[0].operations[0],
            FileTreeOperation::Remove { id } if id == "/repo/src"
        ));
        assert!(matches!(
            &batch.patches[0].operations[1],
            FileTreeOperation::Upsert { node, .. } if node.id == "/repo/src" && !node.is_dir
        ));
        assert!(matches!(
            &batch.patches[1].operations[0],
            FileTreeOperation::Remove { id } if id == "/repo/src/lib.rs"
        ));
        assert!(reconciled.expanded_ids.is_empty());
        assert_eq!(reconciled.selected_id, None);
        assert_eq!(reconciled.scroll_offset, 99.0);
    }
}
