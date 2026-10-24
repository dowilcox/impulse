//! Incremental file-tree updates (the Swift `FileTreePatcher`, ported from
//! `impulse-core/src/file_tree.rs`): stable node ids, the operations that
//! turn a folder's old child list into its new one, and patch batches built
//! from file watcher events.
//!
//! A patch for a folder removes the children that went away (and those
//! that changed between file, folder and symlink), then upserts the ones
//! that are new or changed. When the children both lists share keep their
//! order, an entry that only shifted because another arrived or left ahead
//! of it gets no upsert: the applier keeps the order of entries a patch
//! doesn't name, so one new file at the top of a big folder is one
//! operation, not hundreds.
//!
//! The JSON is the original Rust serde shape: snake_case keys, `None` as
//! `null`, operations tagged `{"type": "remove"|"upsert", ...}`.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::directory_lister::{read_directory_entries, FileEntry};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileTreePatch {
    pub parent_id: String,
    pub operations: Vec<FileTreeOperation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileTreePatchBatch {
    pub root_id: String,
    pub patches: Vec<FileTreePatch>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileTreeWatchEventKind {
    Create,
    Modify,
    Remove,
    Rename,
    Any,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileTreeWatchEvent {
    pub kind: FileTreeWatchEventKind,
    pub paths: Vec<String>,
}

impl FileTreeWatchEvent {
    pub fn new(kind: FileTreeWatchEventKind, paths: &[&str]) -> Self {
        FileTreeWatchEvent {
            kind,
            paths: paths.iter().map(|p| p.to_string()).collect(),
        }
    }
}

/// A node's id: its path without trailing separators (the path itself when
/// that would leave nothing, so `/` stays `/`).
pub fn stable_node_id(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        path.to_string()
    } else {
        trimmed.to_string()
    }
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

/// The operations that turn `parent_path`'s children `before` into `after`.
pub fn build_child_patch(
    parent_path: &str,
    before: &[FileEntry],
    after: &[FileEntry],
) -> FileTreePatch {
    let parent_id = stable_node_id(parent_path);
    let before_nodes: Vec<FileTreeNode> = before
        .iter()
        .map(|e| node_from_entry(parent_path, e))
        .collect();
    let after_nodes: Vec<FileTreeNode> = after
        .iter()
        .map(|e| node_from_entry(parent_path, e))
        .collect();

    let before_by_id: HashMap<&str, (usize, &FileTreeNode)> = before_nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), (i, n)))
        .collect();
    let after_by_id: HashMap<&str, &FileTreeNode> =
        after_nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    let mut operations = Vec::new();
    let mut replace_ids: HashSet<&str> = HashSet::new();

    for node in &before_nodes {
        match after_by_id.get(node.id.as_str()) {
            Some(after_node) => {
                if requires_replacement(node, after_node) {
                    replace_ids.insert(&node.id);
                    operations.push(FileTreeOperation::Remove {
                        id: node.id.clone(),
                    });
                }
            }
            None => operations.push(FileTreeOperation::Remove {
                id: node.id.clone(),
            }),
        }
    }

    let kept = |nodes: &[FileTreeNode], in_other: &dyn Fn(&str) -> bool| -> Vec<String> {
        nodes
            .iter()
            .map(|n| n.id.as_str())
            .filter(|id| in_other(id) && !replace_ids.contains(id))
            .map(String::from)
            .collect()
    };
    let only_shifted = kept(&before_nodes, &|id| after_by_id.contains_key(id))
        == kept(&after_nodes, &|id| before_by_id.contains_key(id));

    for (index, node) in after_nodes.iter().enumerate() {
        let should_upsert = match before_by_id.get(node.id.as_str()) {
            Some((before_index, before_node)) => {
                replace_ids.contains(node.id.as_str())
                    || *before_node != node
                    || (!only_shifted && *before_index != index)
            }
            None => true,
        };
        if should_upsert {
            operations.push(FileTreeOperation::Upsert {
                parent_id: parent_id.clone(),
                index,
                node: node.clone(),
            });
        }
    }

    FileTreePatch {
        parent_id,
        operations,
    }
}

/// Patches for every folder the events touched, from the children each had
/// before and has after (a folder missing from either map had none).
/// Folders with no changes are left out.
pub fn build_patch_batch(
    root_path: &str,
    events: &[FileTreeWatchEvent],
    before_by_parent: &HashMap<String, Vec<FileEntry>>,
    after_by_parent: &HashMap<String, Vec<FileEntry>>,
) -> FileTreePatchBatch {
    let parent_paths = affected_parent_paths(root_path, events, before_by_parent, after_by_parent);
    let patches = parent_paths
        .iter()
        .map(|parent_path| {
            build_child_patch(
                parent_path,
                before_by_parent.get(parent_path).map_or(&[], Vec::as_slice),
                after_by_parent.get(parent_path).map_or(&[], Vec::as_slice),
            )
        })
        .filter(|patch| !patch.operations.is_empty())
        .collect();
    FileTreePatchBatch {
        root_id: stable_node_id(root_path),
        patches,
    }
}

/// `build_patch_batch` with each touched folder read again through
/// `read_directory` (a folder that's gone has no children). `None` when a
/// folder that exists can't be read.
pub fn build_patch_batch_from_filesystem_with(
    root_path: &str,
    events: &[FileTreeWatchEvent],
    before_by_parent: &HashMap<String, Vec<FileEntry>>,
    show_hidden: bool,
    read_directory: impl Fn(&str, bool) -> Option<Vec<FileEntry>>,
) -> Option<FileTreePatchBatch> {
    let parent_paths = affected_parent_paths(root_path, events, before_by_parent, &HashMap::new());
    let mut after_by_parent = HashMap::new();
    for parent_path in parent_paths {
        let after = if Path::new(&parent_path).is_dir() {
            read_directory(&parent_path, show_hidden)?
        } else {
            Vec::new()
        };
        after_by_parent.insert(parent_path, after);
    }
    Some(build_patch_batch(
        root_path,
        events,
        before_by_parent,
        &after_by_parent,
    ))
}

/// `build_patch_batch_from_filesystem_with` the plain directory lister.
pub fn build_patch_batch_from_filesystem(
    root_path: &str,
    events: &[FileTreeWatchEvent],
    before_by_parent: &HashMap<String, Vec<FileEntry>>,
    show_hidden: bool,
) -> Option<FileTreePatchBatch> {
    build_patch_batch_from_filesystem_with(
        root_path,
        events,
        before_by_parent,
        show_hidden,
        |path, hidden| read_directory_entries(path, hidden).ok(),
    )
}

/// The folders whose children the events may have changed: each path's
/// parent inside the root (the root for anything outside it), plus a path
/// that is itself a loaded folder. Shallowest first, then in byte order.
pub fn affected_parent_paths(
    root_path: &str,
    events: &[FileTreeWatchEvent],
    before_by_parent: &HashMap<String, Vec<FileEntry>>,
    after_by_parent: &HashMap<String, Vec<FileEntry>>,
) -> Vec<String> {
    let root = normalized_path(root_path);
    let mut parents: HashSet<String> = HashSet::new();
    for event in events {
        for raw_path in &event.paths {
            let path = normalized_path(raw_path);
            parents.insert(event_parent_path(&root, &path));
            if before_by_parent.contains_key(&path) || after_by_parent.contains_key(&path) {
                parents.insert(path);
            }
        }
    }
    let mut sorted: Vec<String> = parents.into_iter().collect();
    sorted.sort_by(|a, b| {
        path_depth(a)
            .cmp(&path_depth(b))
            .then_with(|| a.as_bytes().cmp(b.as_bytes()))
    });
    sorted
}

fn requires_replacement(before: &FileTreeNode, after: &FileTreeNode) -> bool {
    before.is_dir != after.is_dir || before.is_symlink != after.is_symlink
}

fn event_parent_path(root: &str, path: &str) -> String {
    if path == root || !path_is_within_root(root, path) {
        return root.to_string();
    }
    let parent = deleting_last_component(path);
    if !parent.is_empty() && path_is_within_root(root, &parent) {
        parent
    } else {
        root.to_string()
    }
}

/// The path without its last component, as `NSString` gives it: `/` for a
/// top-level path, empty for a bare name.
fn deleting_last_component(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => "/".to_string(),
        Some(i) => trimmed[..i].trim_end_matches('/').to_string(),
        None if path.starts_with('/') => "/".to_string(),
        None => String::new(),
    }
}

fn normalized_path(path: &str) -> String {
    stable_node_id(if path.is_empty() { "." } else { path })
}

fn path_depth(path: &str) -> usize {
    let segments = path.split('/').filter(|s| !s.is_empty()).count();
    if path.starts_with('/') {
        segments + 1
    } else {
        segments.max(usize::from(!path.is_empty()))
    }
}

fn path_is_within_root(root: &str, path: &str) -> bool {
    if path == root {
        return true;
    }
    if root.ends_with('/') {
        path.starts_with(root)
    } else {
        path.strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use FileTreeWatchEventKind::*;

    fn entry(name: &str, path: &str, dir: bool, status: Option<&str>) -> FileEntry {
        FileEntry {
            name: name.into(),
            path: path.into(),
            is_dir: dir,
            is_symlink: false,
            size: 10,
            modified: 20,
            git_status: status.map(String::from),
        }
    }

    fn file(name: &str, path: &str) -> FileEntry {
        entry(name, path, false, None)
    }

    fn map(pairs: Vec<(&str, Vec<FileEntry>)>) -> HashMap<String, Vec<FileEntry>> {
        pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    #[test]
    fn unchanged_children_emit_no_operations() {
        let before = vec![
            entry("src", "/repo/src", true, None),
            file("main.rs", "/repo/main.rs"),
        ];
        assert!(build_child_patch("/repo", &before, &before)
            .operations
            .is_empty());
    }

    #[test]
    fn rename_is_remove_before_upsert() {
        let patch = build_child_patch(
            "/repo",
            &[file("old.rs", "/repo/old.rs")],
            &[file("new.rs", "/repo/new.rs")],
        );
        assert_eq!(patch.operations.len(), 2);
        assert_eq!(
            patch.operations[0],
            FileTreeOperation::Remove {
                id: "/repo/old.rs".into()
            }
        );
        let FileTreeOperation::Upsert { index, node, .. } = &patch.operations[1] else {
            panic!("expected upsert second");
        };
        assert_eq!(*index, 0);
        assert_eq!(node.id, "/repo/new.rs");
    }

    #[test]
    fn metadata_or_position_changes_emit_upsert() {
        let before = vec![file("a.rs", "/repo/a.rs"), file("b.rs", "/repo/b.rs")];
        let after = vec![
            file("b.rs", "/repo/b.rs"),
            entry("a.rs", "/repo/a.rs", false, Some("M")),
        ];
        let patch = build_child_patch("/repo", &before, &after);
        assert_eq!(patch.operations.len(), 2);
        assert!(patch
            .operations
            .iter()
            .all(|op| matches!(op, FileTreeOperation::Upsert { .. })));
    }

    #[test]
    fn pure_index_shifts_emit_no_upserts() {
        let before = vec![
            entry("src", "/repo/src", true, None),
            file("b.rs", "/repo/b.rs"),
            file("c.rs", "/repo/c.rs"),
            file("d.rs", "/repo/d.rs"),
        ];
        let after = vec![
            entry("src", "/repo/src", true, None),
            file("a.rs", "/repo/a.rs"),
            file("b.rs", "/repo/b.rs"),
            entry("c.rs", "/repo/c.rs", false, Some("M")),
        ];
        let patch = build_child_patch("/repo", &before, &after);
        // b.rs moved from 1 to 2 only because a.rs arrived: no operation.
        assert_eq!(
            patch.operations,
            vec![
                FileTreeOperation::Remove {
                    id: "/repo/d.rs".into()
                },
                FileTreeOperation::Upsert {
                    parent_id: "/repo".into(),
                    index: 1,
                    node: node_from_entry("/repo", &after[1]),
                },
                FileTreeOperation::Upsert {
                    parent_id: "/repo".into(),
                    index: 3,
                    node: node_from_entry("/repo", &after[3]),
                },
            ]
        );
    }

    #[test]
    fn type_replacement_removes_before_upsert() {
        let patch = build_child_patch(
            "/repo",
            &[entry("target", "/repo/target", true, None)],
            &[entry("target", "/repo/target", false, None)],
        );
        assert_eq!(patch.operations.len(), 2);
        assert_eq!(
            patch.operations[0],
            FileTreeOperation::Remove {
                id: "/repo/target".into()
            }
        );
        let FileTreeOperation::Upsert { node, .. } = &patch.operations[1] else {
            panic!("unexpected operations: {:?}", patch.operations);
        };
        assert_eq!(node.id, "/repo/target");
        assert!(!node.is_dir);
    }

    #[test]
    fn affected_parent_paths_are_stable_and_deduped() {
        let events = vec![
            FileTreeWatchEvent::new(Modify, &["/repo/src/lib.rs"]),
            FileTreeWatchEvent::new(Create, &["/repo/src/main.rs"]),
            FileTreeWatchEvent::new(Remove, &["/tmp/outside.txt"]),
        ];
        let parents = affected_parent_paths("/repo", &events, &HashMap::new(), &HashMap::new());
        assert_eq!(parents, vec!["/repo", "/repo/src"]);
    }

    #[test]
    fn batch_represents_move_across_parents() {
        let events = vec![FileTreeWatchEvent::new(
            Rename,
            &["/repo/src/item.rs", "/repo/tests/item.rs"],
        )];
        let batch = build_patch_batch(
            "/repo",
            &events,
            &map(vec![
                ("/repo/src", vec![file("item.rs", "/repo/src/item.rs")]),
                ("/repo/tests", vec![]),
            ]),
            &map(vec![
                ("/repo/src", vec![]),
                ("/repo/tests", vec![file("item.rs", "/repo/tests/item.rs")]),
            ]),
        );
        assert_eq!(batch.patches.len(), 2);
        assert_eq!(
            batch.patches[0].operations[0],
            FileTreeOperation::Remove {
                id: "/repo/src/item.rs".into()
            }
        );
        let FileTreeOperation::Upsert { node, .. } = &batch.patches[1].operations[0] else {
            panic!("unexpected batch: {batch:?}");
        };
        assert_eq!(node.id, "/repo/tests/item.rs");
    }

    #[test]
    fn loaded_directory_event_refreshes_that_directory_too() {
        let events = vec![FileTreeWatchEvent::new(Modify, &["/repo/src"])];
        let batch = build_patch_batch(
            "/repo",
            &events,
            &map(vec![(
                "/repo/src",
                vec![file("old.rs", "/repo/src/old.rs")],
            )]),
            &map(vec![(
                "/repo/src",
                vec![file("new.rs", "/repo/src/new.rs")],
            )]),
        );
        assert_eq!(batch.patches.len(), 1);
        assert_eq!(batch.patches[0].parent_id, "/repo/src");
        assert_eq!(batch.patches[0].operations.len(), 2);
    }

    #[test]
    fn parent_paths_follow_nsstring() {
        assert_eq!(deleting_last_component("/repo/src/lib.rs"), "/repo/src");
        assert_eq!(deleting_last_component("/repo"), "/");
        assert_eq!(deleting_last_component("repo"), "");
        assert_eq!(deleting_last_component("a/b"), "a");
        assert_eq!(path_depth("/"), 1);
        assert_eq!(path_depth("/repo/src"), 3);
        assert_eq!(path_depth("a/b"), 2);
        assert_eq!(path_depth(""), 0);
    }
}
