// SPDX-License-Identifier: GPL-3.0-only
//
// File tree data model QObject for QML. Maintains an in-memory tree of
// expanded nodes and exposes a flat JSON representation for QML TreeView.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, root_path)]
        #[qproperty(QString, tree_json)]
        #[qproperty(bool, show_hidden)]
        #[qproperty(QString, active_file_path)]
        type FileTreeModel = super::FileTreeModelRust;

        #[qinvokable]
        fn load_root(self: Pin<&mut FileTreeModel>, path: &QString);

        #[qinvokable]
        fn toggle_expand(self: Pin<&mut FileTreeModel>, path: &QString);

        #[qinvokable]
        fn refresh(self: Pin<&mut FileTreeModel>);

        #[qinvokable]
        fn create_file(self: Pin<&mut FileTreeModel>, parent_path: &QString, name: &QString);

        #[qinvokable]
        fn create_folder(self: Pin<&mut FileTreeModel>, parent_path: &QString, name: &QString);

        #[qinvokable]
        fn rename_item(self: Pin<&mut FileTreeModel>, old_path: &QString, new_name: &QString);

        #[qinvokable]
        fn delete_item(self: Pin<&mut FileTreeModel>, path: &QString);

        #[qinvokable]
        fn get_git_status_json(self: &FileTreeModel, path: &QString) -> QString;

        #[qinvokable]
        fn toggle_hidden(self: Pin<&mut FileTreeModel>, show: bool);

        #[qinvokable]
        fn set_active_path(self: Pin<&mut FileTreeModel>, path: &QString);

        #[qsignal]
        fn tree_changed(self: Pin<&mut FileTreeModel>);

        #[qsignal]
        fn file_activated(self: Pin<&mut FileTreeModel>, path: QString);
    }
}

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::pin::Pin;

/// A flattened tree node for JSON serialization to QML.
#[derive(Serialize)]
struct FlatTreeNode {
    name: String,
    path: String,
    #[serde(rename = "isDir")]
    is_dir: bool,
    #[serde(rename = "isExpanded")]
    is_expanded: bool,
    depth: i32,
    #[serde(rename = "gitStatus")]
    git_status: Option<String>,
    #[serde(rename = "childCount")]
    child_count: i32,
}

pub struct FileTreeModelRust {
    root_path: QString,
    tree_json: QString,
    show_hidden: bool,
    active_file_path: QString,
    // Internal state
    expanded_paths: HashSet<String>,
    /// Cached batch git statuses: dir_path -> (filename -> status_code)
    git_statuses: HashMap<String, HashMap<String, String>>,
}

impl Default for FileTreeModelRust {
    fn default() -> Self {
        Self {
            root_path: QString::default(),
            tree_json: QString::from("[]"),
            show_hidden: false,
            active_file_path: QString::default(),
            expanded_paths: HashSet::new(),
            git_statuses: HashMap::new(),
        }
    }
}

impl FileTreeModelRust {
    /// Rebuild the flat tree JSON from the current root, expanded paths, and git status.
    fn rebuild_tree(&mut self) {
        let root = self.root_path.to_string();
        if root.is_empty() {
            self.tree_json = QString::from("[]");
            return;
        }

        // Fetch git statuses in one batch for the whole repo
        self.git_statuses =
            impulse_core::filesystem::get_all_git_statuses(&root).unwrap_or_default();

        let mut nodes = Vec::new();
        self.collect_nodes(&root, 0, &mut nodes);

        let json = serde_json::to_string(&nodes).unwrap_or_else(|_| "[]".to_string());
        self.tree_json = QString::from(json.as_str());
    }

    /// Recursively collect visible tree nodes into a flat list.
    fn collect_nodes(&self, dir_path: &str, depth: i32, nodes: &mut Vec<FlatTreeNode>) {
        let entries = match impulse_core::filesystem::read_directory_with_git_status_batch(
            dir_path,
            self.show_hidden,
            &self.git_statuses,
        ) {
            Ok(entries) => entries,
            Err(e) => {
                log::warn!("Failed to read directory '{}': {}", dir_path, e);
                return;
            }
        };

        for entry in &entries {
            let is_expanded = entry.is_dir && self.expanded_paths.contains(&entry.path);

            // Count direct children if this is an expanded directory
            let child_count = if entry.is_dir && is_expanded {
                match impulse_core::filesystem::read_directory_entries(
                    &entry.path,
                    self.show_hidden,
                ) {
                    Ok(children) => children.len() as i32,
                    Err(_) => 0,
                }
            } else if entry.is_dir {
                // Report -1 for unexpanded dirs to indicate "has children but not loaded"
                -1
            } else {
                0
            };

            nodes.push(FlatTreeNode {
                name: entry.name.clone(),
                path: entry.path.clone(),
                is_dir: entry.is_dir,
                is_expanded,
                depth,
                git_status: entry.git_status.clone(),
                child_count,
            });

            // Recurse into expanded directories
            if entry.is_dir && is_expanded {
                self.collect_nodes(&entry.path, depth + 1, nodes);
            }
        }
    }
}

impl qobject::FileTreeModel {
    pub fn load_root(mut self: Pin<&mut Self>, path: &QString) {
        let path_str = path.to_string();
        if path_str.is_empty() {
            return;
        }

        self.as_mut().rust_mut().expanded_paths.clear();
        self.as_mut().set_root_path(path.clone());
        self.as_mut().rust_mut().rebuild_tree();

        let json = self.as_ref().tree_json().clone();
        self.as_mut().set_tree_json(json);
        self.as_mut().tree_changed();
    }

    pub fn toggle_expand(mut self: Pin<&mut Self>, path: &QString) {
        let path_str = path.to_string();

        let was_expanded = self.as_ref().rust().expanded_paths.contains(&path_str);
        if was_expanded {
            // Collapse: remove this path and all descendants
            let prefix = format!("{}/", path_str);
            self.as_mut()
                .rust_mut()
                .expanded_paths
                .retain(|p| !p.starts_with(&prefix) && p != &path_str);
        } else {
            self.as_mut().rust_mut().expanded_paths.insert(path_str);
        }

        self.as_mut().rust_mut().rebuild_tree();
        let json = self.as_ref().tree_json().clone();
        self.as_mut().set_tree_json(json);
        self.as_mut().tree_changed();
    }

    pub fn refresh(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().rebuild_tree();
        let json = self.as_ref().tree_json().clone();
        self.as_mut().set_tree_json(json);
        self.as_mut().tree_changed();
    }

    pub fn create_file(mut self: Pin<&mut Self>, parent_path: &QString, name: &QString) {
        let parent = parent_path.to_string();
        let name_str = name.to_string();

        if parent.is_empty() || name_str.is_empty() {
            return;
        }

        let file_path = std::path::Path::new(&parent).join(&name_str);
        if let Err(e) = std::fs::write(&file_path, "") {
            log::warn!("Failed to create file '{}': {}", file_path.display(), e);
            return;
        }

        // Ensure parent is expanded
        self.as_mut().rust_mut().expanded_paths.insert(parent);
        self.as_mut().rust_mut().rebuild_tree();
        let json = self.as_ref().tree_json().clone();
        self.as_mut().set_tree_json(json);

        let activated_path = QString::from(file_path.to_string_lossy().as_ref());
        self.as_mut().tree_changed();
        self.as_mut().file_activated(activated_path);
    }

    pub fn create_folder(mut self: Pin<&mut Self>, parent_path: &QString, name: &QString) {
        let parent = parent_path.to_string();
        let name_str = name.to_string();

        if parent.is_empty() || name_str.is_empty() {
            return;
        }

        let dir_path = std::path::Path::new(&parent).join(&name_str);
        if let Err(e) = std::fs::create_dir_all(&dir_path) {
            log::warn!("Failed to create folder '{}': {}", dir_path.display(), e);
            return;
        }

        // Ensure parent is expanded
        self.as_mut().rust_mut().expanded_paths.insert(parent);
        self.as_mut().rust_mut().rebuild_tree();
        let json = self.as_ref().tree_json().clone();
        self.as_mut().set_tree_json(json);
        self.as_mut().tree_changed();
    }

    pub fn rename_item(mut self: Pin<&mut Self>, old_path: &QString, new_name: &QString) {
        let old = old_path.to_string();
        let new_name_str = new_name.to_string();

        if old.is_empty() || new_name_str.is_empty() {
            return;
        }

        let old_pb = std::path::Path::new(&old);
        let new_path = match old_pb.parent() {
            Some(parent) => parent.join(&new_name_str),
            None => {
                log::warn!("Cannot determine parent directory of '{}'", old);
                return;
            }
        };

        if let Err(e) = std::fs::rename(&old, &new_path) {
            log::warn!(
                "Failed to rename '{}' to '{}': {}",
                old,
                new_path.display(),
                e
            );
            return;
        }

        // Update expanded paths if we renamed a directory
        let old_prefix = format!("{}/", old);
        let new_prefix = format!("{}/", new_path.display());
        let updated_paths: Vec<String> = self
            .as_ref()
            .rust()
            .expanded_paths
            .iter()
            .map(|p| {
                if p == &old {
                    new_path.to_string_lossy().to_string()
                } else if p.starts_with(&old_prefix) {
                    format!("{}{}", new_prefix, &p[old_prefix.len()..])
                } else {
                    p.clone()
                }
            })
            .collect();

        self.as_mut().rust_mut().expanded_paths = updated_paths.into_iter().collect();
        self.as_mut().rust_mut().rebuild_tree();
        let json = self.as_ref().tree_json().clone();
        self.as_mut().set_tree_json(json);
        self.as_mut().tree_changed();
    }

    pub fn delete_item(mut self: Pin<&mut Self>, path: &QString) {
        let path_str = path.to_string();
        if path_str.is_empty() {
            return;
        }

        let pb = std::path::Path::new(&path_str);
        let result = if pb.is_dir() {
            std::fs::remove_dir_all(pb)
        } else {
            std::fs::remove_file(pb)
        };

        if let Err(e) = result {
            log::warn!("Failed to delete '{}': {}", path_str, e);
            return;
        }

        // Remove from expanded paths
        let prefix = format!("{}/", path_str);
        self.as_mut()
            .rust_mut()
            .expanded_paths
            .retain(|p| !p.starts_with(&prefix) && p != &path_str);

        self.as_mut().rust_mut().rebuild_tree();
        let json = self.as_ref().tree_json().clone();
        self.as_mut().set_tree_json(json);
        self.as_mut().tree_changed();
    }

    pub fn get_git_status_json(&self, path: &QString) -> QString {
        let path_str = path.to_string();
        match impulse_core::filesystem::get_git_status_for_directory(&path_str) {
            Ok(status_map) => {
                let json = serde_json::to_string(&status_map).unwrap_or_else(|_| "{}".to_string());
                QString::from(json.as_str())
            }
            Err(e) => {
                log::warn!("Failed to get git status for '{}': {}", path_str, e);
                QString::from("{}")
            }
        }
    }

    pub fn toggle_hidden(mut self: Pin<&mut Self>, show: bool) {
        self.as_mut().set_show_hidden(show);
        self.as_mut().rust_mut().rebuild_tree();
        let json = self.as_ref().tree_json().clone();
        self.as_mut().set_tree_json(json);
        self.as_mut().tree_changed();
    }

    pub fn set_active_path(self: Pin<&mut Self>, path: &QString) {
        self.set_active_file_path(path.clone());
    }
}
