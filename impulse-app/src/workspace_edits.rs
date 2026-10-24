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

/// How to reverse one part of an applied edit.
enum UndoStep {
    /// An open editor's buffer.
    Editor {
        path: PathBuf,
        before: String,
        after: String,
    },
    /// A file rewritten on disk.
    File {
        path: PathBuf,
        before: String,
        after: String,
        bom: bool,
    },
    /// A file created (emptied, if it was there).
    Created {
        path: PathBuf,
        previous: Option<Vec<u8>>,
    },
    Moved {
        from: PathBuf,
        to: PathBuf,
    },
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
        // As the Swift app's alert words it.
        let place = boundary
            .map(|b| format!("“{}”", name(&b)))
            .unwrap_or_else(|| "the project".into());
        crate::overlays::open_dialog(window, cx, move |dialog, _, _| {
            let (apply_workbench, pending) = (workbench.clone(), pending.clone());
            dialog
                .title(format!("Apply “{}”?", pending.title))
                .w(px(520.))
                .close_button(false)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(format!("It does more than edit files in {place}:"))
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
        let mut failure = None;
        let mut steps = Vec::new();
        for operation in &pending.edit.operations {
            match self.apply_operation(operation, pending.utf32, window, cx) {
                Ok(step) => steps.extend(step),
                Err(error) => {
                    // Changes before it stay, as LSP's "abort" failure handling.
                    failure = Some(error);
                    break;
                }
            }
        }
        if pending.edit.has_resource_operations() {
            self.reload_tree(cx);
        }
        let files = pending.edit.uris().len();
        if let Some(failure) = failure {
            return self.set_notice(failure, cx);
        }
        // One file undoes with Cmd/Ctrl+Z like any edit; more get an Undo
        // for the whole edit, as the Swift app's toast has.
        if files <= 1 {
            return self.set_notice(format!("{}: done.", pending.title), cx);
        }
        self.toast_with_action(
            format!("{}: changed {files} files", pending.title),
            None,
            "Undo",
            window,
            cx,
            move |this, window, cx| {
                for step in steps.iter().rev() {
                    this.undo_step(step, window, cx);
                }
                if steps
                    .iter()
                    .any(|s| !matches!(s, UndoStep::Editor { .. } | UndoStep::File { .. }))
                {
                    this.reload_tree(cx);
                }
            },
        );
    }

    /// Reverse one part of an applied edit, if nothing changed it since.
    fn undo_step(&mut self, step: &UndoStep, window: &mut Window, cx: &mut Context<Self>) {
        match step {
            UndoStep::Editor {
                path,
                before,
                after,
            } => {
                if let Some(editor) = self.editor_for(path, cx) {
                    editor.update(cx, |editor, cx| {
                        editor.restore_text(after, before, window, cx)
                    });
                }
            }
            UndoStep::File {
                path,
                before,
                after,
                bom,
            } => {
                if text_file::read(path).is_some_and(|now| &now.text == after) {
                    let _ = text_file::write(before, *bom, path);
                    if let Some(editor) = self.editor_for(path, cx) {
                        editor.update(cx, |editor, cx| editor.reload_from_disk(window, cx));
                    }
                }
            }
            UndoStep::Created { path, previous } => match previous {
                Some(bytes) => {
                    let _ = std::fs::write(path, bytes);
                }
                None => {
                    if std::fs::metadata(path).is_ok_and(|m| m.len() == 0) {
                        let _ = std::fs::remove_file(path);
                    }
                }
            },
            UndoStep::Moved { from, to } => {
                if to.exists() && !from.exists() {
                    let _ = std::fs::rename(to, from);
                }
            }
        }
    }

    fn apply_operation(
        &mut self,
        operation: &Operation,
        utf32: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Option<UndoStep>, String> {
        match operation {
            Operation::Edit { uri, edits } => {
                let path = uri_path(uri);
                if edits.is_empty() {
                    return Ok(None);
                }
                if let Some(editor) = self.editor_for(&path, cx) {
                    return editor.update(cx, |editor, cx| {
                        let before = editor.text(cx);
                        editor.apply_server_edits(edits, utf32, window, cx)?;
                        let after = editor.text(cx);
                        Ok(Some(UndoStep::Editor {
                            path: path.clone(),
                            before,
                            after,
                        }))
                    });
                }
                let file = text_file::read(&path)
                    .ok_or_else(|| format!("Couldn't read {} as UTF-8 text.", name(&path)))?;
                let updated = apply_text_edits(&file.text, edits, utf32)
                    .ok_or_else(|| format!("The changes to {} overlap.", name(&path)))?;
                text_file::write(&updated, file.bom, &path)
                    .map_err(|e| format!("Couldn't write {}: {e}", name(&path)))?;
                Ok(Some(UndoStep::File {
                    path,
                    before: file.text,
                    after: updated,
                    bom: file.bom,
                }))
            }
            Operation::Create {
                uri,
                overwrite,
                ignore_if_exists,
            } => {
                let path = uri_path(uri);
                if path.exists() && (*ignore_if_exists || !*overwrite) {
                    return if *ignore_if_exists {
                        Ok(None)
                    } else {
                        Err(format!("{} already exists.", name(&path)))
                    };
                }
                let previous = std::fs::read(&path).ok();
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::write(&path, "")
                    .map_err(|e| format!("Couldn't create {}: {e}", name(&path)))?;
                Ok(Some(UndoStep::Created { path, previous }))
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
                        return Ok(None);
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
                    .map_err(|e| format!("Couldn't move {}: {e}", name(&from)))?;
                Ok(Some(UndoStep::Moved { from, to }))
            }
            Operation::Delete {
                uri,
                recursive,
                ignore_if_not_exists,
            } => {
                let path = uri_path(uri);
                if !path.exists() {
                    return if *ignore_if_not_exists {
                        Ok(None)
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
                // In the Trash, where it can be put back by hand: Undo
                // doesn't reach into the Trash.
                trash::delete(&path)
                    .map_err(|e| format!("Couldn't remove {}: {e}", name(&path)))?;
                Ok(None)
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
