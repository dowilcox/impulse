// SPDX-License-Identifier: GPL-3.0-only
//
// Central window state QObject for QML.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(bool, sidebar_visible)]
        #[qproperty(i32, sidebar_width)]
        #[qproperty(QString, current_directory)]
        #[qproperty(i32, active_tab_index)]
        #[qproperty(i32, tab_count)]
        #[qproperty(QString, tab_display_infos_json)]
        #[qproperty(QString, git_branch)]
        #[qproperty(QString, shell_name)]
        #[qproperty(i32, cursor_line)]
        #[qproperty(i32, cursor_column)]
        #[qproperty(QString, language)]
        #[qproperty(QString, encoding)]
        #[qproperty(QString, indent_info)]
        #[qproperty(QString, blame_info)]
        #[qproperty(QString, project_root)]
        type WindowModel = super::WindowModelRust;

        #[qinvokable]
        fn toggle_sidebar(self: Pin<&mut WindowModel>);

        #[qinvokable]
        fn set_directory(self: Pin<&mut WindowModel>, path: &QString);

        #[qinvokable]
        fn create_tab(self: Pin<&mut WindowModel>, tab_type: &QString);

        #[qinvokable]
        fn close_tab(self: Pin<&mut WindowModel>, index: i32);

        #[qinvokable]
        fn select_tab(self: Pin<&mut WindowModel>, index: i32);

        #[qinvokable]
        fn move_tab(self: Pin<&mut WindowModel>, from_index: i32, to_index: i32);

        #[qinvokable]
        fn create_editor_tab(self: Pin<&mut WindowModel>, path: &QString);

        #[qinvokable]
        fn create_image_tab(self: Pin<&mut WindowModel>, path: &QString);

        #[qinvokable]
        fn set_tab_title(self: Pin<&mut WindowModel>, index: i32, title: &QString);

        #[qinvokable]
        fn set_tab_title_by_id(self: Pin<&mut WindowModel>, tab_id: i32, title: &QString);

        #[qinvokable]
        fn get_initial_directory(self: &WindowModel) -> QString;

        #[qinvokable]
        fn has_startup_path(self: &WindowModel) -> bool;

        #[qsignal]
        fn directory_changed(self: Pin<&mut WindowModel>);

        #[qsignal]
        fn tab_switched(self: Pin<&mut WindowModel>);

        #[qsignal]
        fn file_open_requested(self: Pin<&mut WindowModel>, path: QString);
    }
}

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

fn startup_args() -> impl Iterator<Item = String> {
    std::env::args().skip(1).filter(|arg| !arg.starts_with('-'))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TabDisplayInfo {
    id: u64,
    title: String,
    icon: String,
    #[serde(rename = "isModified")]
    is_modified: bool,
    #[serde(rename = "tabType")]
    tab_type: String,
    #[serde(rename = "filePath")]
    file_path: String,
}

pub struct WindowModelRust {
    sidebar_visible: bool,
    sidebar_width: i32,
    current_directory: QString,
    active_tab_index: i32,
    tab_count: i32,
    tab_display_infos_json: QString,
    git_branch: QString,
    shell_name: QString,
    cursor_line: i32,
    cursor_column: i32,
    language: QString,
    encoding: QString,
    indent_info: QString,
    blame_info: QString,
    project_root: QString,
    // Internal state not exposed as properties
    tabs: Vec<TabDisplayInfo>,
    next_tab_id: u64,
}

impl Default for WindowModelRust {
    fn default() -> Self {
        let shell = impulse_core::shell::get_default_shell_name();
        Self {
            sidebar_visible: true,
            sidebar_width: 250,
            current_directory: QString::default(),
            active_tab_index: -1,
            tab_count: 0,
            tab_display_infos_json: QString::from("[]"),
            git_branch: QString::default(),
            shell_name: QString::from(shell.as_str()),
            cursor_line: 1,
            cursor_column: 1,
            language: QString::default(),
            encoding: QString::from("UTF-8"),
            indent_info: QString::from("Spaces: 4"),
            blame_info: QString::default(),
            project_root: {
                let exe = std::env::current_exe().unwrap_or_default();
                let exe_str = exe.to_string_lossy();
                if let Some(idx) = exe_str.find("/target/") {
                    QString::from(&exe_str[..idx + 1])
                } else {
                    let cwd = std::env::current_dir().unwrap_or_default();
                    QString::from(format!("{}/", cwd.display()).as_str())
                }
            },
            tabs: Vec::new(),
            next_tab_id: 0,
        }
    }
}

impl qobject::WindowModel {
    pub fn toggle_sidebar(self: Pin<&mut Self>) {
        let new_val = !*self.sidebar_visible();
        self.set_sidebar_visible(new_val);
    }

    pub fn set_directory(mut self: Pin<&mut Self>, path: &QString) {
        let path_str = path.to_string();
        if path_str.is_empty() {
            return;
        }

        self.as_mut().set_current_directory(path.clone());

        // Update git branch
        match impulse_core::filesystem::get_git_branch(&path_str) {
            Ok(Some(branch)) => {
                self.as_mut().set_git_branch(QString::from(branch.as_str()));
            }
            _ => {
                self.as_mut().set_git_branch(QString::default());
            }
        }

        self.as_mut().directory_changed();
    }

    pub fn create_tab(mut self: Pin<&mut Self>, tab_type: &QString) {
        let tab_type_str = tab_type.to_string();
        let title = match tab_type_str.as_str() {
            "terminal" => format!("Terminal {}", self.tabs.len() + 1),
            "editor" => "Untitled".to_string(),
            "image" => "Image".to_string(),
            _ => "Tab".to_string(),
        };

        let icon = match tab_type_str.as_str() {
            "terminal" => "terminal",
            "editor" => "file",
            "image" => "image",
            _ => "file",
        }
        .to_string();

        let tab_id = self.as_ref().rust().next_tab_id;
        self.as_mut().rust_mut().next_tab_id += 1;

        let info = TabDisplayInfo {
            id: tab_id,
            title,
            icon,
            is_modified: false,
            tab_type: tab_type_str,
            file_path: String::new(),
        };

        self.as_mut().rust_mut().tabs.push(info);
        let new_count = self.tabs.len() as i32;
        let new_index = new_count - 1;

        self.as_mut().set_tab_count(new_count);
        self.as_mut().set_active_tab_index(new_index);
        self.as_mut().rebuild_tabs_json();
        self.as_mut().tab_switched();
    }

    pub fn close_tab(mut self: Pin<&mut Self>, index: i32) {
        let idx = index as usize;
        if idx >= self.tabs.len() {
            return;
        }

        self.as_mut().rust_mut().tabs.remove(idx);
        let new_count = self.tabs.len() as i32;
        self.as_mut().set_tab_count(new_count);

        // Adjust active tab index
        let current = *self.active_tab_index();
        if new_count == 0 {
            self.as_mut().set_active_tab_index(-1);
        } else if current >= new_count {
            self.as_mut().set_active_tab_index(new_count - 1);
        } else if current > index {
            self.as_mut().set_active_tab_index(current - 1);
        }

        self.as_mut().rebuild_tabs_json();
        self.as_mut().tab_switched();
    }

    pub fn select_tab(mut self: Pin<&mut Self>, index: i32) {
        if index < 0 || index >= self.tabs.len() as i32 {
            return;
        }
        self.as_mut().set_active_tab_index(index);
        self.as_mut().tab_switched();
    }

    pub fn move_tab(mut self: Pin<&mut Self>, from_index: i32, to_index: i32) {
        let from = from_index as usize;
        let to = to_index as usize;
        let len = self.tabs.len();

        if from >= len || to >= len || from == to {
            return;
        }

        let tab = self.as_mut().rust_mut().tabs.remove(from);
        self.as_mut().rust_mut().tabs.insert(to, tab);

        // Adjust active tab index to follow the moved tab
        let active = *self.active_tab_index();
        if active == from_index {
            self.as_mut().set_active_tab_index(to_index);
        } else if from_index < active && to_index >= active {
            self.as_mut().set_active_tab_index(active - 1);
        } else if from_index > active && to_index <= active {
            self.as_mut().set_active_tab_index(active + 1);
        }

        self.as_mut().rebuild_tabs_json();
    }

    pub fn create_editor_tab(mut self: Pin<&mut Self>, path: &QString) {
        let path_str = path.to_string();
        let title = std::path::Path::new(&path_str)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".to_string());

        let tab_id = self.as_ref().rust().next_tab_id;
        self.as_mut().rust_mut().next_tab_id += 1;

        let info = TabDisplayInfo {
            id: tab_id,
            title,
            icon: "file".to_string(),
            is_modified: false,
            tab_type: "editor".to_string(),
            file_path: path_str,
        };

        self.as_mut().rust_mut().tabs.push(info);
        let new_count = self.tabs.len() as i32;
        let new_index = new_count - 1;

        self.as_mut().set_tab_count(new_count);
        self.as_mut().set_active_tab_index(new_index);
        self.as_mut().rebuild_tabs_json();
        self.as_mut().tab_switched();
    }

    pub fn get_initial_directory(&self) -> QString {
        // Check CLI args for a directory path
        for arg in startup_args() {
            let path = std::path::Path::new(&arg);
            if path.is_dir() {
                if let Ok(canonical) = path.canonicalize() {
                    return QString::from(canonical.to_string_lossy().as_ref());
                }
            }
            // If arg is a file, use its parent directory
            if path.is_file() {
                if let Some(parent) = path.parent() {
                    if let Ok(canonical) = parent.canonicalize() {
                        return QString::from(canonical.to_string_lossy().as_ref());
                    }
                }
            }
        }

        // Fall back to CWD
        if let Ok(cwd) = std::env::current_dir() {
            return QString::from(cwd.to_string_lossy().as_ref());
        }

        // Last resort: home directory
        if let Some(home) = dirs::home_dir() {
            return QString::from(home.to_string_lossy().as_ref());
        }

        QString::from("/")
    }

    pub fn has_startup_path(&self) -> bool {
        startup_args().next().is_some()
    }

    pub fn set_tab_title(mut self: Pin<&mut Self>, index: i32, title: &QString) {
        let idx = index as usize;
        if idx < self.tabs.len() {
            self.as_mut().rust_mut().tabs[idx].title = title.to_string();
            self.as_mut().rebuild_tabs_json();
        }
    }

    pub fn set_tab_title_by_id(mut self: Pin<&mut Self>, tab_id: i32, title: &QString) {
        let target_id = tab_id as u64;
        if let Some(tab) = self
            .as_mut()
            .rust_mut()
            .tabs
            .iter_mut()
            .find(|tab| tab.id == target_id)
        {
            tab.title = title.to_string();
            self.as_mut().rebuild_tabs_json();
        }
    }

    pub fn create_image_tab(mut self: Pin<&mut Self>, path: &QString) {
        let path_str = path.to_string();
        let title = std::path::Path::new(&path_str)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Image".to_string());

        let tab_id = self.as_ref().rust().next_tab_id;
        self.as_mut().rust_mut().next_tab_id += 1;

        let info = TabDisplayInfo {
            id: tab_id,
            title,
            icon: "image".to_string(),
            is_modified: false,
            tab_type: "image".to_string(),
            file_path: path_str,
        };

        self.as_mut().rust_mut().tabs.push(info);
        let new_count = self.tabs.len() as i32;
        let new_index = new_count - 1;

        self.as_mut().set_tab_count(new_count);
        self.as_mut().set_active_tab_index(new_index);
        self.as_mut().rebuild_tabs_json();
        self.as_mut().tab_switched();
    }

    fn rebuild_tabs_json(self: Pin<&mut Self>) {
        let json = serde_json::to_string(&self.tabs).unwrap_or_else(|_| "[]".to_string());
        self.set_tab_display_infos_json(QString::from(json.as_str()));
    }
}
