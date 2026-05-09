use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

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

pub fn stable_node_id(path: &str) -> String {
    let trimmed = path.trim_end_matches(|ch| ch == '/' || ch == '\\');
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
    FileTreeViewState {
        expanded_ids: state
            .expanded_ids
            .iter()
            .filter(|id| !removed_only.contains(id.as_str()))
            .cloned()
            .collect(),
        selected_id: state
            .selected_id
            .as_ref()
            .filter(|id| !removed_only.contains(id.as_str()))
            .cloned(),
        scroll_offset: state.scroll_offset,
    }
}

fn requires_replacement(before: &FileTreeNode, after: &FileTreeNode) -> bool {
    before.is_dir != after.is_dir || before.is_symlink != after.is_symlink
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

        assert_eq!(reconciled.expanded_ids, vec!["/repo/replace"]);
        assert_eq!(reconciled.selected_id, None);
        assert_eq!(reconciled.scroll_offset, 42.5);
    }
}
