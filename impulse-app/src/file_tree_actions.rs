//! The file tree's context menu (the Swift `FileTreeListView`'s): new
//! files and folders, opening to the side, history, revealing, copying
//! paths, the git actions for a changed file, renaming, and moving to the
//! Trash with Undo where the platform can put it back.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::toasts::Toast;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::{ClipboardItem, Context, Window};
use impulse_git::{ChangeStatus, FileChange};
use impulse_kit::layout_tree::SplitAxis;

use crate::workbench::Workbench;

type Act = Rc<dyn Fn(&mut Workbench, &mut Window, &mut Context<Workbench>)>;

/// "Reveal in Finder" on the Mac.
pub(crate) const REVEAL: &str = if cfg!(target_os = "macos") {
    "Reveal in Finder"
} else {
    "Show in File Manager"
};

/// A new name for an item in `dir` that stays in it: no separators, no
/// `..`, not empty.
fn checked_name(name: &str) -> Option<&str> {
    let name = name.trim();
    (!name.is_empty() && !name.contains(['/', '\0']) && !name.contains("..") && name != ".")
        .then_some(name)
}

impl Workbench {
    /// The menu for a tree row.
    pub(crate) fn tree_menu(
        &self,
        path: PathBuf,
        is_dir: bool,
        cx: &Context<Self>,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let workbench = cx.entity().downgrade();
        let root = self.tree_root(cx);
        let status = self
            .git
            .active
            .as_ref()
            .and_then(|r| r.read(cx).status_of(&path).map(str::to_string));
        let repo_root = self.git.active.as_ref().map(|r| r.read(cx).root.clone());
        move |mut menu, _, _| {
            let mut items: Vec<Option<(&'static str, Act)>> = Vec::new();
            let p = path.clone();
            fn push(items: &mut Vec<Option<(&'static str, Act)>>, title: &'static str, act: Act) {
                items.push(Some((title, act)));
            }
            if is_dir {
                let dir = p.clone();
                push(
                    &mut items,
                    "New File…",
                    Rc::new(move |w, window, cx| w.tree_new(&dir, false, window, cx)),
                );
                let dir = p.clone();
                push(
                    &mut items,
                    "New Folder…",
                    Rc::new(move |w, window, cx| w.tree_new(&dir, true, window, cx)),
                );
            } else {
                let file = p.clone();
                push(
                    &mut items,
                    "Open to the Side",
                    Rc::new(move |w, window, cx| w.open_beside(&file, window, cx)),
                );
                let file = p.clone();
                push(
                    &mut items,
                    "Mention in Agent",
                    Rc::new(move |w, window, cx| {
                        let path = w.relative_to_workspace(&file.to_string_lossy());
                        w.send_to_best_agent(&format!("@{path} "), None, window, cx)
                    }),
                );
            }
            items.push(None);
            let file = p.clone();
            push(
                &mut items,
                "Show History",
                Rc::new(move |w, window, cx| w.path_history(&file, window, cx)),
            );
            let file = p.clone();
            push(
                &mut items,
                REVEAL,
                Rc::new(move |_, _, cx| reveal(&file, cx)),
            );
            let file = p.clone();
            push(
                &mut items,
                "Copy Path",
                Rc::new(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        file.to_string_lossy().into_owned(),
                    ))
                }),
            );
            let (file, base) = (p.clone(), root.clone());
            push(
                &mut items,
                "Copy Relative Path",
                Rc::new(move |_, _, cx| {
                    let relative = file
                        .strip_prefix(&base)
                        .map(|r| r.to_string_lossy().into_owned())
                        .unwrap_or_else(|_| {
                            file.file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default()
                        });
                    cx.write_to_clipboard(ClipboardItem::new_string(relative));
                }),
            );
            if !is_dir {
                items.push(None);
                let file = p.clone();
                push(
                    &mut items,
                    "Open with Default App",
                    Rc::new(move |_, _, cx| cx.open_with_system(&file)),
                );
            }
            let changed = status
                .as_deref()
                .and_then(|s| match s {
                    "M" => Some(ChangeStatus::Modified),
                    "A" => Some(ChangeStatus::Added),
                    "?" => Some(ChangeStatus::Untracked),
                    "D" => Some(ChangeStatus::Deleted),
                    "R" => Some(ChangeStatus::Renamed),
                    _ => None,
                })
                .filter(|_| !is_dir);
            let relative = repo_root
                .as_ref()
                .and_then(|r| p.strip_prefix(r).ok())
                .map(|r| r.to_string_lossy().into_owned());
            if let (Some(status), Some(relative)) = (changed, relative) {
                items.push(None);
                let file = p.clone();
                push(
                    &mut items,
                    "Open Changes",
                    Rc::new(move |w, window, cx| {
                        w.open_review_at(impulse_git::DiffScope::Uncommitted, &file, window, cx)
                    }),
                );
                let r = relative.clone();
                push(
                    &mut items,
                    "Stage",
                    Rc::new(move |w, window, cx| {
                        if let Some(repo) = w.git.active.clone() {
                            let change = FileChange::new(r.clone(), ChangeStatus::Modified);
                            w.git_stage(&repo, &[change], window, cx);
                        }
                    }),
                );
                push(
                    &mut items,
                    "Discard Changes…",
                    Rc::new(move |w, window, cx| {
                        let Some(repo) = w.git.active.clone() else {
                            return;
                        };
                        // Untracked files go to the Trash, an added file is
                        // unstaged, and tracked ones go back to HEAD; all
                        // can be undone from the toast.
                        match status {
                            ChangeStatus::Untracked => w.git_discard(
                                &repo,
                                vec![FileChange::new(relative.clone(), ChangeStatus::Untracked)],
                                false,
                                window,
                                cx,
                            ),
                            ChangeStatus::Added => w.git_unstage(
                                &repo,
                                &[FileChange::new(relative.clone(), ChangeStatus::Added)],
                                window,
                                cx,
                            ),
                            _ => w.git_discard(
                                &repo,
                                vec![FileChange::new(relative.clone(), ChangeStatus::Modified)],
                                true,
                                window,
                                cx,
                            ),
                        }
                    }),
                );
            }
            items.push(None);
            let file = p.clone();
            push(
                &mut items,
                "Rename…",
                Rc::new(move |w, window, cx| w.tree_rename(&file, window, cx)),
            );
            let file = p.clone();
            push(
                &mut items,
                "Move to Trash",
                Rc::new(move |w, window, cx| w.tree_trash(&file, window, cx)),
            );
            for item in items {
                menu = match item {
                    None => menu.separator(),
                    Some((title, act)) => {
                        let workbench = workbench.clone();
                        menu.item(PopupMenuItem::new(title).on_click(move |_, window, cx| {
                            let act = act.clone();
                            let _ = workbench.update(cx, |w, cx| act(w, window, cx));
                        }))
                    }
                };
            }
            menu
        }
    }

    /// New File… or New Folder… in `dir`: asks for a name, creates it, and
    /// opens a new file.
    pub(crate) fn tree_new(
        &mut self,
        dir: &Path,
        folder: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dir = dir.to_path_buf();
        let (title, message) = if folder {
            ("New Folder", "Enter a name for the new folder:")
        } else {
            ("New File", "Enter a name for the new file:")
        };
        self.text_prompt(
            title,
            message,
            "",
            "Create",
            window,
            cx,
            move |this, name, window, cx| {
                let Some(name) = checked_name(&name) else {
                    return;
                };
                let path = dir.join(name);
                if path.exists() {
                    return this.toast(
                        Toast::warning(format!("{name} already exists.")),
                        window,
                        cx,
                    );
                }
                let made = if folder {
                    std::fs::create_dir(&path)
                } else {
                    std::fs::write(&path, "")
                };
                match made {
                    Ok(()) => {
                        this.expand_to(&dir, cx);
                        this.reload_tree(cx);
                        if !folder {
                            this.open_file(&path, window, cx);
                        }
                    }
                    Err(error) => this.toast(
                        Toast::warning(format!("Couldn't create {name}: {error}")),
                        window,
                        cx,
                    ),
                }
            },
        );
    }

    /// Show `dir` expanded in the tree (a new item made in it).
    fn expand_to(&mut self, dir: &Path, cx: &mut Context<Self>) {
        let show_hidden = crate::settings_store::SettingsStore::get(cx).sidebar_show_hidden;
        let workspace = self.workspace_mut();
        if workspace.expanded_dirs.insert(dir.to_path_buf()) {
            let entries = crate::workspace::list_dir(dir, show_hidden);
            workspace.children.insert(dir.to_path_buf(), entries);
            workspace.save_expanded();
        }
    }

    /// Open to the Side: the file in a new pane beside the focused one.
    pub(crate) fn open_beside(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let Some(surface) = self.make_editor(path, window, cx) else {
            return self.open_file(path, window, cx);
        };
        if let crate::workspace::Surface::Editor(editor) = &surface {
            let index = self.active;
            self.start_lsp(index, editor, window, cx);
        }
        self.split_focused(surface, SplitAxis::Horizontal, window, cx);
    }

    /// Show History of a file or folder.
    fn path_history(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let dir = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        let Some(root) = impulse_git::branches::repo_root(&dir.to_string_lossy())
            .map(|r| r.trim_end_matches('/').to_string())
        else {
            return self.set_notice("Not in a git repository.".into(), cx);
        };
        let relative = path
            .strip_prefix(&root)
            .ok()
            .map(|r| r.to_string_lossy().into_owned())
            .filter(|r| !r.is_empty());
        let surface = self.make_history(root, relative, None, window, cx);
        let active = self.active;
        self.add_tab(active, surface, window, cx);
    }

    /// Rename…: a new name in the same folder.
    fn tree_rename(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let current = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let from = path.to_path_buf();
        let title = format!("Rename {current}");
        self.text_prompt(
            &title,
            "",
            &current.clone(),
            "Rename",
            window,
            cx,
            move |this, name, window, cx| {
                let Some(name) = checked_name(&name).filter(|n| *n != current) else {
                    return;
                };
                let to = from.with_file_name(name);
                if to.exists() {
                    return this.toast(
                        Toast::warning(format!("{name} already exists.")),
                        window,
                        cx,
                    );
                }
                match std::fs::rename(&from, &to) {
                    Ok(()) => this.reload_tree(cx),
                    Err(error) => this.toast(
                        Toast::warning(format!("Couldn't rename: {error}")),
                        window,
                        cx,
                    ),
                }
            },
        );
    }

    /// Move to Trash, with Undo where the Trash can be read back (the
    /// freedesktop Trash on Linux).
    fn tree_trash(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if let Err(error) = trash::delete(path) {
            return self.toast(
                Toast::warning(format!("Couldn't move to the Trash: {error}")),
                window,
                cx,
            );
        }
        self.reload_tree(cx);
        let message = format!("Moved {name} to the Trash");
        if !cfg!(any(target_os = "linux", target_os = "freebsd")) {
            return self.toast(Toast::success(message), window, cx);
        }
        let original = path.to_path_buf();
        self.toast_with_action(
            message,
            None,
            "Undo",
            window,
            cx,
            move |this, window, cx| match restore_from_trash(&original) {
                Ok(()) => this.reload_tree(cx),
                Err(error) => this.toast(
                    Toast::warning(format!("Couldn't put it back: {error}")),
                    window,
                    cx,
                ),
            },
        );
    }
}

/// Put back the most recently trashed item that was at `path`.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn restore_from_trash(path: &Path) -> Result<(), String> {
    let items = trash::os_limited::list().map_err(|e| e.to_string())?;
    let item = items
        .into_iter()
        .filter(|item| item.original_path() == path)
        .max_by_key(|item| item.time_deleted)
        .ok_or_else(|| "it's no longer in the Trash".to_string())?;
    trash::os_limited::restore_all([item]).map_err(|e| e.to_string())
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
fn restore_from_trash(_: &Path) -> Result<(), String> {
    Err("the Trash can't be read here".into())
}

/// Reveal in Finder (the file selected), or open the folder holding it in
/// the file manager on Linux.
pub(crate) fn reveal(path: &Path, cx: &mut gpui_kit::App) {
    if cfg!(target_os = "macos") {
        let _ = std::process::Command::new("/usr/bin/open")
            .arg("-R")
            .arg(path)
            .spawn();
    } else if path.is_dir() {
        cx.open_with_system(path);
    } else if let Some(parent) = path.parent() {
        cx.open_with_system(parent);
    }
}

#[cfg(test)]
mod tests {
    use super::checked_name;

    #[test]
    fn names_stay_in_their_folder() {
        assert_eq!(checked_name(" notes.md "), Some("notes.md"));
        for bad in ["", "  ", "a/b", "..", "../x", ".", "a\0b"] {
            assert_eq!(checked_name(bad), None, "{bad:?}");
        }
    }
}

/// A tree row being dragged.
pub(crate) struct DraggedPath(pub PathBuf);

/// What follows the pointer while a row is dragged: its name.
pub(crate) struct DragPreview(pub String);

impl gpui_kit::Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl gpui_kit::IntoElement {
        use gpui_kit::component::ActiveTheme as _;
        use gpui_kit::{div, prelude::*, px};
        let theme = cx.theme();
        div()
            .px(px(8.))
            .py(px(3.))
            .rounded(px(5.))
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .text_size(px(12.))
            .text_color(theme.popover_foreground)
            .child(self.0.clone())
    }
}

/// `path` is `root` or inside it (`app-old` beside `app` isn't).
fn is_inside(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}

impl Workbench {
    /// Items dropped on a tree row or the tree: into the row's folder (a
    /// file's own), moved when they're in the project, copied when they
    /// come from elsewhere, asking before replacing an item of the same
    /// name (which goes to the Trash).
    pub(crate) fn tree_drop(
        &mut self,
        sources: Vec<PathBuf>,
        target: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dir = if target.is_dir() {
            target.to_path_buf()
        } else {
            target.parent().map(Path::to_path_buf).unwrap_or_default()
        };
        let root = self.tree_root(cx);
        for source in sources {
            let Some(name) = source.file_name().map(|n| n.to_os_string()) else {
                continue;
            };
            let destination = dir.join(&name);
            // Not onto itself, nor into itself.
            if destination == source || destination.starts_with(&source) {
                continue;
            }
            let moving = is_inside(&source, &root);
            let shown = name.to_string_lossy().into_owned();
            if destination.exists() {
                let message = if moving {
                    "Replace it? The existing one goes to the Trash and the original is moved."
                } else {
                    "Replace it with the one you're copying? The existing one goes to the Trash."
                };
                self.git_confirm(
                    format!("An item named “{shown}” already exists"),
                    message,
                    "Replace",
                    true,
                    window,
                    cx,
                    move |this, window, cx| {
                        if let Err(error) = trash::delete(&destination) {
                            return this.toast(
                                Toast::warning(format!("Couldn't replace {shown}: {error}")),
                                window,
                                cx,
                            );
                        }
                        this.place(&source, &destination, moving, window, cx);
                    },
                );
                continue;
            }
            self.place(&source, &destination, moving, window, cx);
        }
    }

    fn place(
        &mut self,
        source: &Path,
        destination: &Path,
        moving: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let placed = if moving {
            std::fs::rename(source, destination)
        } else {
            impulse_kit::project::folder_clone::copy_item(source, destination)
        };
        match placed {
            Ok(()) => self.reload_tree(cx),
            Err(error) => self.toast(
                Toast::warning(format!(
                    "{}: {error}",
                    if moving { "Move Failed" } else { "Copy Failed" }
                )),
                window,
                cx,
            ),
        }
    }
}
