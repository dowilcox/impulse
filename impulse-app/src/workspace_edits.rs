//! Workspace edits from language servers (rename, code actions) across
//! files, as the Swift app's `LSPWorkspaceEdits` applies them: an open
//! editor takes its part as one undo step, other files are rewritten on disk
//! (UTF-8, byte-order mark kept, through symlinks), and files are created,
//! moved or trashed as asked. An edit that changes files outside the
//! boundary, or creates, moves or deletes files, applies only once the user
//! says yes; one computed against a document that has changed since, or that
//! would move or remove a file open in a tab, is refused.

use std::path::{Path, PathBuf};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::WindowExt as _;
use gpui_kit::{div, prelude::*, px, Context, Window};
use impulse_kit::text_file;
use impulse_lsp::edits::apply_text_edits;
use impulse_lsp::workspace_edit::{
    repository_root, uri_path, user_edit_boundary, Confirmation, Operation, WorkspaceEdit,
};

use crate::workbench::Workbench;

/// A workspace edit a server sent for something the user did in `origin`.
#[derive(Clone)]
pub struct PendingEdit {
    pub edit: WorkspaceEdit,
    /// Positions are characters, not UTF-16 units.
    pub utf32: bool,
    /// The file the user was in, and the server's project root.
    pub origin: PathBuf,
    pub server_root: PathBuf,
    /// What it's for ("Rename to count"), for the dialog.
    pub title: String,
}

fn name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

impl Workbench {
    /// Apply `pending`, asking first when it reaches outside the boundary or
    /// creates, moves or deletes files.
    pub fn apply_workspace_edit(
        &mut self,
        pending: PendingEdit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(refusal) = self.edit_refusal(&pending.edit, cx) {
            self.set_notice(refusal, cx);
            return;
        }
        let origin = pending.origin.to_string_lossy().into_owned();
        let trusted = self.trust.trusting_folder(&origin).map(PathBuf::from);
        let boundary = user_edit_boundary(
            trusted.as_deref(),
            pending.origin.parent().and_then(repository_root).as_deref(),
            Some(&pending.server_root),
        );
        let confirmations = pending.edit.confirmations(boundary.as_deref());
        if confirmations.is_empty() {
            self.apply_now(&pending, window, cx);
        } else {
            self.confirm_edit(pending, boundary, confirmations, window, cx);
        }
    }

    /// Why the edit can't apply now, if it can't.
    fn edit_refusal(&self, edit: &WorkspaceEdit, cx: &Context<Self>) -> Option<String> {
        for (uri, version) in &edit.versions {
            let path = uri_path(uri);
            if let Some(editor) = self.editor_for(&path, cx) {
                if editor
                    .read(cx)
                    .document_version(uri)
                    .is_some_and(|v| v != *version)
                {
                    return Some(format!(
                        "{} changed while the language server was working on it. Try again.",
                        name(&path)
                    ));
                }
            }
        }
        for operation in &edit.operations {
            let removed: Vec<PathBuf> = match operation {
                Operation::Edit { .. } | Operation::Create { .. } => Vec::new(),
                Operation::Rename { old_uri, .. } => vec![uri_path(old_uri)],
                Operation::Delete { uri, .. } => vec![uri_path(uri)],
            };
            for path in removed {
                if self.editor_for(&path, cx).is_some() {
                    return Some(format!(
                        "{} is open in a tab. Close it first: the change would move or remove it.",
                        name(&path)
                    ));
                }
            }
        }
        None
    }

    fn confirm_edit(
        &mut self,
        pending: PendingEdit,
        boundary: Option<PathBuf>,
        confirmations: Vec<Confirmation>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workbench = cx.entity().downgrade();
        let lines: Vec<String> = confirmations.iter().map(Confirmation::describe).collect();
        let reach = boundary
            .map(|b| format!("It reaches outside “{}”.", name(&b)))
            .unwrap_or_default();
        window.open_dialog(cx, move |dialog, _, _| {
            let (apply_workbench, pending) = (workbench.clone(), pending.clone());
            dialog
                .title(format!("{}?", pending.title))
                .w(px(520.))
                .close_button(false)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(format!(
                            "This change does more than edit the file you're in. {reach}"
                        ))
                        .children(lines.iter().map(|line| {
                            div()
                                .font_family("JetBrains Mono")
                                .text_size(px(12.))
                                .child(line.clone())
                        })),
                )
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            Button::new("edit-cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(Button::new("edit-apply").primary().label("Apply").on_click(
                            move |_, window, cx| {
                                window.close_dialog(cx);
                                let pending = pending.clone();
                                let _ = apply_workbench.update(cx, |workbench, cx| {
                                    // Things may have changed while asking.
                                    match workbench.edit_refusal(&pending.edit, cx) {
                                        Some(refusal) => workbench.set_notice(refusal, cx),
                                        None => workbench.apply_now(&pending, window, cx),
                                    }
                                });
                            },
                        )),
                )
        });
    }

    fn apply_now(&mut self, pending: &PendingEdit, window: &mut Window, cx: &mut Context<Self>) {
        let mut touched = 0;
        let mut failure = None;
        for operation in &pending.edit.operations {
            touched += 1;
            if let Err(error) = self.apply_operation(operation, pending.utf32, window, cx) {
                // Changes before it stay, as LSP's "abort" failure handling.
                failure = Some(error);
                break;
            }
        }
        if pending.edit.has_resource_operations() {
            self.reload_tree(cx);
        }
        let files = pending.edit.uris().len();
        self.set_notice(
            failure.unwrap_or_else(|| match files {
                0 | 1 if touched <= 1 => format!("{}: done.", pending.title),
                _ => format!("{}: changed {files} files.", pending.title),
            }),
            cx,
        );
    }

    fn apply_operation(
        &mut self,
        operation: &Operation,
        utf32: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        match operation {
            Operation::Edit { uri, edits } => {
                let path = uri_path(uri);
                if edits.is_empty() {
                    return Ok(());
                }
                if let Some(editor) = self.editor_for(&path, cx) {
                    return editor.update(cx, |editor, cx| {
                        editor.apply_server_edits(edits, utf32, window, cx)
                    });
                }
                let file = text_file::read(&path)
                    .ok_or_else(|| format!("Couldn't read {} as UTF-8 text.", name(&path)))?;
                let updated = apply_text_edits(&file.text, edits, utf32)
                    .ok_or_else(|| format!("The changes to {} overlap.", name(&path)))?;
                text_file::write(&updated, file.bom, &path)
                    .map_err(|e| format!("Couldn't write {}: {e}", name(&path)))
            }
            Operation::Create {
                uri,
                overwrite,
                ignore_if_exists,
            } => {
                let path = uri_path(uri);
                if path.exists() && (*ignore_if_exists || !*overwrite) {
                    return if *ignore_if_exists {
                        Ok(())
                    } else {
                        Err(format!("{} already exists.", name(&path)))
                    };
                }
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::write(&path, "")
                    .map_err(|e| format!("Couldn't create {}: {e}", name(&path)))
            }
            Operation::Rename {
                old_uri,
                new_uri,
                overwrite,
                ignore_if_exists,
            } => {
                let (from, to) = (uri_path(old_uri), uri_path(new_uri));
                if to.exists() {
                    if *ignore_if_exists {
                        return Ok(());
                    }
                    if !*overwrite {
                        return Err(format!("{} already exists.", name(&to)));
                    }
                    trash::delete(&to).map_err(|e| e.to_string())?;
                }
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::rename(&from, &to)
                    .map_err(|e| format!("Couldn't move {}: {e}", name(&from)))
            }
            Operation::Delete {
                uri,
                recursive,
                ignore_if_not_exists,
            } => {
                let path = uri_path(uri);
                if !path.exists() {
                    return if *ignore_if_not_exists {
                        Ok(())
                    } else {
                        Err(format!("{} doesn't exist.", name(&path)))
                    };
                }
                // A folder goes with its contents only when the server said so.
                let empty = std::fs::read_dir(&path)
                    .map(|mut d| d.next().is_none())
                    .unwrap_or(true);
                if path.is_dir() && !*recursive && !empty {
                    return Err(format!("{} isn't empty.", name(&path)));
                }
                trash::delete(&path).map_err(|e| format!("Couldn't remove {}: {e}", name(&path)))
            }
        }
    }

    /// The editor showing `path`, if one does.
    pub fn editor_for(
        &self,
        path: &Path,
        cx: &gpui_kit::App,
    ) -> Option<gpui_kit::Entity<crate::editor_tab::EditorTab>> {
        let (w, t, pane) = self.find_editor(path, cx)?;
        self.workspaces[w].tabs[t].panes[&pane].editor().cloned()
    }
}
