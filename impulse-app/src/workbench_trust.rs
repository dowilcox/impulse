//! Workspace trust and language servers, per workspace (the Swift
//! `MainWindowController+Trust` and `+LSP`): a folder's language servers
//! run once it's trusted, the first time a workspace is shown untrusted the
//! window asks, and editors in a trusted folder get the servers for their
//! language.

use std::path::Path;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{div, prelude::*, px, App, Context, Entity, Window};

use crate::editor_tab::{ApplyEdit, EditorHooks, EditorTab, ShowDocument};
use crate::lsp_hub::LspHub;
use crate::settings_store::SettingsStore;
use crate::workbench::Workbench;
use crate::workbench_chrome::abbreviate_home;
use crate::workspace::{Kind, Surface};

/// Not the home folder or the disk: trusting those trusts everything.
fn offers_parent(parent: &str) -> bool {
    let normalized = impulse_kit::trust::normalize(parent);
    let home = impulse_kit::trust::normalize(&crate::workspace::home().to_string_lossy());
    !["/", "/Users", "/Volumes", "/home", home.as_str()].contains(&normalized.as_str())
}

impl Workbench {
    fn is_trusted_folder(&self, index: usize, cx: &App) -> bool {
        let Some(workspace) = self.workspaces.get(index) else {
            return false;
        };
        // "Ask before trusting folders" off: every folder is trusted.
        !SettingsStore::get(cx).ask_to_trust_folders
            || self.trust.is_trusted(&workspace.root.to_string_lossy())
    }

    pub(crate) fn is_restricted(&self) -> bool {
        let workspace = self.workspace();
        workspace.kind == Kind::Folder
            && workspace.lsp.is_none()
            && !self.trust.is_trusted(&workspace.root.to_string_lossy())
    }

    /// The folder is trusted: run language servers for it, else ask once
    /// (the Swift app's `requestTrustIfNeeded`). Scratch has none.
    pub(crate) fn request_trust(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.workspaces.get(index) else {
            return;
        };
        if workspace.kind == Kind::Scratch {
            return;
        }
        if self.is_trusted_folder(index, cx) {
            self.enable_lsp(index, window, cx);
        } else if !workspace.declined {
            self.present_trust_prompt(index, window, cx);
        }
    }

    /// "Do you trust the files in …?", as the Swift app asks it.
    pub(crate) fn present_trust_prompt(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.workspaces.get(index) else {
            return;
        };
        let workspace_id = workspace.id;
        let folder = impulse_kit::trust::normalize(&workspace.root.to_string_lossy());
        let name = Path::new(&folder)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| folder.clone());
        let parent = Path::new(&folder)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let offer_parent = offers_parent(&parent);
        let trust_parent = Rc::new(std::cell::Cell::new(false));
        let workbench = cx.entity().downgrade();
        let shown_folder = abbreviate_home(&folder);
        let shown_parent = abbreviate_home(&parent);
        let stay: crate::tasks::Run = {
            let workbench = workbench.clone();
            Rc::new(move |window, cx| {
                window.close_dialog(cx);
                let _ = workbench.update(cx, |workbench, cx| {
                    if let Some(w) = workbench
                        .workspaces
                        .iter_mut()
                        .find(|w| w.id == workspace_id)
                    {
                        w.declined = true;
                    }
                    cx.notify();
                });
            })
        };
        let trust_it: crate::tasks::Run = {
            let trust_parent = trust_parent.clone();
            Rc::new(move |window, cx| {
                window.close_dialog(cx);
                let _ = workbench.update(cx, |workbench, cx| {
                    // The folder itself first: trusting the parent
                    // doesn't lift a restriction on a folder inside it.
                    workbench.trust.trust(&folder);
                    if trust_parent.get() {
                        workbench.trust.trust(&parent);
                    }
                    let index = workbench
                        .workspaces
                        .iter()
                        .position(|w| w.id == workspace_id);
                    if let Some(index) = index {
                        workbench.workspaces[index].declined = false;
                        workbench.enable_lsp(index, window, cx);
                    }
                });
            })
        };
        let title = format!("Do you trust the files in “{name}”?");
        const ABOUT: &str = "Impulse can run code from this folder on its own: language servers \
                             (some run a project's build scripts and tools), formatters and \
                             commands on save, and background git fetch. Until you trust it, \
                             they stay off. The terminal and editing work as usual.";
        #[cfg(debug_assertions)]
        crate::debug_snapshot::note_sheet(
            vec![title.clone(), ABOUT.into(), shown_folder.clone()],
            vec![
                ("Stay Restricted", stay.clone()),
                ("Trust Folder", trust_it.clone()),
            ],
        );
        window.open_dialog(cx, move |dialog, _, _| {
            let (stay, trust_it) = (stay.clone(), trust_it.clone());
            let checkbox_value = trust_parent.clone();
            dialog
                .title(title.clone())
                .w(px(480.))
                .keyboard(false)
                .overlay_closable(false)
                .close_button(false)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .child(ABOUT)
                        .child(
                            div()
                                .font_family("JetBrains Mono")
                                .text_size(px(12.))
                                .child(shown_folder.clone()),
                        )
                        .children(offer_parent.then(|| {
                            Checkbox::new("trust-parent")
                                .label(format!("Trust everything in “{shown_parent}”"))
                                .checked(checkbox_value.get())
                                .on_change(move |checked, window, _| {
                                    checkbox_value.set(*checked);
                                    window.refresh();
                                })
                        })),
                )
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            Button::new("stay-restricted")
                                .label("Stay Restricted")
                                .on_click(move |_, window, cx| stay(window, cx)),
                        )
                        .child(
                            Button::new("trust-folder")
                                .primary()
                                .label("Trust Folder")
                                .on_click(move |_, window, cx| trust_it(window, cx)),
                        ),
                )
        });
    }

    /// Language servers for workspace `index`, and for its open files.
    pub(crate) fn enable_lsp(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(debug_assertions)]
        if crate::debug_snapshot::config().is_some_and(|c| c.no_lsp) {
            return;
        }
        let workspace = &mut self.workspaces[index];
        if workspace.lsp.is_none() {
            let (hub, events) = LspHub::new(workspace.root.clone(), self.trust.clone());
            workspace.lsp = Some(hub);
            let workspace_id = workspace.id;
            workspace.lsp_events = Some(cx.spawn(async move |workbench, cx| {
                while let Ok(event) = events.recv().await {
                    let alive = workbench.update(cx, |workbench, cx| {
                        workbench.server_event(workspace_id, event, cx)
                    });
                    if alive.is_err() {
                        break;
                    }
                }
            }));
        }
        let editors: Vec<Entity<EditorTab>> = self.workspaces[index]
            .all_surfaces()
            .filter_map(|s| s.editor().cloned())
            .collect();
        for editor in editors {
            self.start_lsp(index, &editor, window, cx);
        }
        cx.notify();
    }

    /// Stop workspace `index`'s language servers (restricted again).
    pub(crate) fn disable_lsp(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(workspace) = self.workspaces.get_mut(index) {
            workspace.lsp = None;
            workspace.lsp_events = None;
        }
        cx.notify();
    }

    /// Hand an editor its language servers, if its file is in a trusted
    /// folder and its workspace runs them.
    pub(crate) fn start_lsp(
        &mut self,
        index: usize,
        editor: &Entity<EditorTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = editor.read(cx).path.clone();
        let index = if self.workspaces[index].kind == Kind::Folder {
            index
        } else {
            self.workspace_for(&path)
        };
        let Some(hub) = self.workspaces.get(index).and_then(|w| w.lsp.clone()) else {
            return;
        };
        if !self.trust.is_trusted(&path.to_string_lossy())
            && SettingsStore::get(cx).ask_to_trust_folders
        {
            return;
        }
        let hooks = EditorHooks {
            show_document: self.show_document_handler(cx),
            apply_edit: self.apply_edit_handler(cx),
        };
        editor.update(cx, |editor, cx| editor.start_lsp(&hub, hooks, window, cx));
    }

    /// The folder workspace whose root holds `path` (the deepest), else the
    /// active one.
    pub(crate) fn workspace_for(&self, path: &Path) -> usize {
        self.workspaces
            .iter()
            .enumerate()
            .filter(|(_, w)| w.kind == Kind::Folder && path.starts_with(&w.root))
            .max_by_key(|(_, w)| w.root.components().count())
            .map(|(i, _)| i)
            .unwrap_or(self.active)
    }

    fn apply_edit_handler(&self, cx: &mut Context<Self>) -> ApplyEdit {
        let workbench = cx.entity().downgrade();
        Rc::new(move |pending, window, cx| {
            let _ = workbench.update(cx, |workbench, cx| {
                workbench.apply_workspace_edit(pending, window, cx)
            });
        })
    }

    /// Go-to-definition into another file: open it in a tab at the target.
    /// Targets in the same file are left to the editor.
    fn show_document_handler(&self, cx: &mut Context<Self>) -> ShowDocument {
        let workbench = cx.entity().downgrade();
        Rc::new(move |params, window, cx| {
            let path =
                std::path::PathBuf::from(impulse_kit::uri::uri_to_file_path(params.uri.as_str()));
            let Some(workbench) = workbench.upgrade() else {
                return false;
            };
            let current = workbench
                .read(cx)
                .active_editor()
                .map(|e| e.read(cx).path.clone());
            if current.as_deref() == Some(path.as_path()) || !path.is_file() {
                return false;
            }
            let start = params.selection.map(|range| range.start);
            workbench.update(cx, |workbench, cx| {
                workbench.open_file(&path, window, cx);
                if let (Some(start), Some(editor)) = (start, workbench.active_editor().cloned()) {
                    editor.update(cx, |editor, cx| editor.go_to(start, window, cx));
                }
            });
            true
        })
    }

    /// What a language server of workspace `workspace_id` sent on its own.
    fn server_event(
        &mut self,
        workspace_id: u64,
        event: impulse_lsp::ServerEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            impulse_lsp::ServerEvent::Diagnostics(server, params) => {
                self.record_problems(&server, &params, cx);
                let Some(workspace) = self.workspaces.iter().find(|w| w.id == workspace_id) else {
                    return;
                };
                let editors: Vec<Entity<EditorTab>> = workspace
                    .all_surfaces()
                    .filter_map(Surface::editor)
                    .filter(|e| e.read(cx).is_document(&params.uri))
                    .cloned()
                    .collect();
                for editor in editors {
                    editor.update(cx, |editor, cx| {
                        editor.set_diagnostics(server.clone(), params.diagnostics.clone(), cx)
                    });
                }
            }
            impulse_lsp::ServerEvent::Message(message) => log::info!("language server: {message}"),
            impulse_lsp::ServerEvent::Exited => log::warn!("a language server exited"),
        }
    }
}
