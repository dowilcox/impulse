//! Workspace trust and language servers, per workspace (the Swift
//! `MainWindowController+Trust` and `+LSP`): a folder's language servers
//! run once it's trusted, the first time a workspace is shown untrusted the
//! window asks, and editors in a trusted folder get the servers for their
//! language.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::{prelude::*, App, Context, Entity, Window};

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
        let light = self.palette.is_light;
        let mut alert =
            crate::alert::Alert::new(title).message(format!("{ABOUT}\n\n{shown_folder}"));
        if offer_parent {
            alert = alert.accessory(None, move |_, _| {
                let checked = trust_parent.clone();
                crate::alert::checkbox(
                    "trust-parent",
                    format!("Trust everything in \u{201c}{shown_parent}\u{201d}"),
                    checked.get(),
                    light,
                    move |window, _| {
                        checked.set(!checked.get());
                        window.refresh();
                    },
                )
                .into_any_element()
            });
        }
        alert
            .button("Trust Folder", move |window, cx| trust_it(window, cx))
            .button("Stay Restricted", move |window, cx| stay(window, cx))
            .show(window, cx);
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
            workspace.lsp_events = Some(cx.spawn_in(window, async move |workbench, cx| {
                while let Ok(event) = events.recv().await {
                    let alive = workbench.update_in(cx, |workbench, window, cx| {
                        workbench.server_event(workspace_id, event, window, cx)
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
        if !self.trust.is_trusted(&path.to_string_lossy())
            && SettingsStore::get(cx).ask_to_trust_folders
        {
            let language = editor.read(cx).language.clone();
            if crate::lsp_hub::LspHub::has_servers(&path, &language) {
                self.note_restricted_file(&path, window, cx);
            }
            return;
        }
        let Some(hub) = self.workspaces.get(index).and_then(|w| w.lsp.clone()) else {
            return;
        };
        let hooks = EditorHooks {
            show_document: self.show_document_handler(cx),
            apply_edit: self.apply_edit_handler(cx),
        };
        editor.update(cx, |editor, cx| editor.start_lsp(&hub, hooks, window, cx));
    }

    /// Ask whether to trust `folder`, one that isn't a workspace (a lone
    /// file's repository or folder); trusting it starts the servers of the
    /// files open in it.
    fn ask_to_trust(&mut self, folder: String, window: &mut Window, cx: &mut Context<Self>) {
        let name = Path::new(&folder)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| folder.clone());
        let workbench = cx.entity().downgrade();
        crate::tasks::confirm(
            window,
            cx,
            format!("Do you trust the files in “{name}”?"),
            "Impulse can run code from this folder on its own: language servers, \
             formatters and commands on save, and background git fetch. Until you \
             trust it, they stay off."
                .into(),
            "Trust Folder",
            "Stay Restricted",
            false,
            move |yes, window, cx| {
                if !yes {
                    return;
                }
                let _ = workbench.update(cx, |w, cx| {
                    w.trust.trust(&folder);
                    w.start_lsp_under(Path::new(&folder), window, cx);
                });
            },
        )
    }

    /// The folder Trust and Restrict act on (Swift's
    /// `trustFolderForActiveContext`): the active folder workspace, else
    /// the active file's repository, else its folder.
    fn trust_folder_for_active_context(&self, cx: &App) -> Option<String> {
        let workspace = self.workspace();
        if workspace.kind == Kind::Folder {
            return Some(workspace.root.to_string_lossy().into_owned());
        }
        let path = self.active_editor().map(|e| e.read(cx).path.clone())?;
        let dir = path.parent()?.to_string_lossy().into_owned();
        Some(
            impulse_git::repo_root(&dir)
                .map(|r| r.trim_end_matches('/').to_string())
                .unwrap_or(dir),
        )
    }

    /// Trust This Folder…
    pub(crate) fn trust_active_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(folder) = self.trust_folder_for_active_context(cx) else {
            return crate::toasts::show(
                window,
                cx,
                crate::toasts::Toast::info("Open a folder or a file to trust it."),
            );
        };
        if self.trust.is_enabled() && self.trust.is_trusted(&folder) {
            let name = folder_name(&folder);
            return crate::toasts::show(
                window,
                cx,
                crate::toasts::Toast::info(format!("“{name}” is already trusted.")),
            );
        }
        if self.workspace().kind == Kind::Folder {
            let index = self.active;
            self.workspaces[index].declined = false;
            self.present_trust_prompt(index, window, cx);
        } else {
            self.ask_to_trust(folder, window, cx);
        }
    }

    /// Restrict This Folder: no longer trusted, and its trusted project
    /// files forgotten so their commands ask again. A folder trusted as
    /// part of one above it says so, offering to restrict that one.
    pub(crate) fn restrict_active_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(folder) = self.trust_folder_for_active_context(cx) else {
            return;
        };
        let name = folder_name(&folder);
        forget_project_trust(&folder);
        if let Some(above) = self.trust.revoke(&folder) {
            let above_name = folder_name(&above);
            let workbench = cx.entity().downgrade();
            let restrict: crate::tasks::Run = Rc::new(move |_, cx| {
                let above = above.clone();
                let _ = workbench.update(cx, |w, cx| {
                    w.trust.revoke(&above);
                    forget_project_trust(&above);
                    w.trust_changed(cx);
                });
            });
            return crate::toasts::show(
                window,
                cx,
                crate::toasts::Toast::info(format!(
                    "“{name}” is trusted as part of “{above_name}”."
                ))
                .action(format!("Restrict {above_name}"), restrict)
                .lifetime(Some(std::time::Duration::from_secs(10))),
            );
        }
        let normalized = impulse_kit::trust::normalize(&folder);
        for workspace in &mut self.workspaces {
            if impulse_kit::trust::normalize(&workspace.root.to_string_lossy()) == normalized {
                workspace.declined = true;
            }
        }
        self.trust_changed(cx);
        crate::toasts::show(
            window,
            cx,
            crate::toasts::Toast::info(format!(
                "“{name}” is restricted: language servers, formatters and background fetch are off."
            )),
        );
    }

    /// Forget Trusted Folders: every folder, and every project file.
    pub(crate) fn forget_trusted_folders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.trust.revoke_all();
        let mut projects = crate::tasks::project_trust();
        projects.revoke_all();
        crate::tasks::save_project_trust(&projects);
        self.trust_changed(cx);
        crate::toasts::show(
            window,
            cx,
            crate::toasts::Toast::info(
                "Every folder is restricted until you trust it again, and project files ask before running commands.",
            ),
        );
    }

    /// Trust was taken away: language servers stop where it's gone, in
    /// every window (they share one trust list).
    fn trust_changed(&mut self, cx: &mut Context<Self>) {
        self.stop_untrusted_servers(cx);
        let me = cx.entity_id();
        for (other, _) in crate::session::workbenches(cx) {
            if other.entity_id() != me {
                other.update(cx, |w, cx| w.stop_untrusted_servers(cx));
            }
        }
    }

    fn stop_untrusted_servers(&mut self, cx: &mut Context<Self>) {
        for index in 0..self.workspaces.len() {
            let root = self.workspaces[index].root.to_string_lossy().into_owned();
            if self.workspaces[index].lsp.is_some() && !self.trust.is_trusted(&root) {
                self.disable_lsp(index, cx);
            }
        }
        cx.notify();
    }

    /// Language servers stayed off for a file outside the trusted folders:
    /// say so once per folder, offering to trust it (not for a file in a
    /// folder workspace, whose own question covers it).
    fn note_restricted_file(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .workspaces
            .iter()
            .any(|w| w.kind == Kind::Folder && path.starts_with(&w.root))
        {
            return;
        }
        let dir = path.parent().unwrap_or(path);
        let folder = impulse_git::repo_root(&dir.to_string_lossy())
            .map(|r| r.trim_end_matches('/').to_string())
            .unwrap_or_else(|| dir.to_string_lossy().into_owned());
        let folder = impulse_kit::trust::normalize(&folder);
        if !self.restricted_noted.insert(folder.clone()) {
            return;
        }
        let name = Path::new(&folder)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| folder.clone());
        let message = format!("Language servers are off in “{name}”: it isn't a trusted folder.");
        let workbench = cx.entity().downgrade();
        let trust: crate::tasks::Run = Rc::new(move |window, cx| {
            let folder = folder.clone();
            let _ = workbench.update(cx, |w, cx| w.ask_to_trust(folder, window, cx));
        });
        crate::tasks::toast(
            window,
            cx,
            crate::tasks::ToastKind::Info,
            message,
            None,
            Some(("Trust…", trust)),
        );
    }

    /// Start language servers for the open editors under `folder` (it was
    /// just trusted).
    fn start_lsp_under(&mut self, folder: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let editors: Vec<_> = self
            .workspaces
            .iter()
            .enumerate()
            .flat_map(|(i, w)| w.tabs.iter().map(move |t| (i, t)))
            .flat_map(|(i, t)| {
                t.panes
                    .values()
                    .filter_map(Surface::editor)
                    .map(move |e| (i, e.clone()))
            })
            .filter(|(_, e)| e.read(cx).path.starts_with(folder))
            .collect();
        for (index, editor) in editors {
            self.start_lsp(index, &editor, window, cx);
        }
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

    /// A language-server problem, said once (the registry keeps retrying).
    fn tell_lsp_once(&mut self, message: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.told_lsp.insert(message.clone()) {
            return;
        }
        crate::toasts::show(
            window,
            cx,
            crate::toasts::Toast::warning(message)
                .lifetime(Some(std::time::Duration::from_secs(12))),
        );
    }

    /// Notice lsp.json edits (the global file and the projects' of open
    /// files), checked every two seconds, as the Swift registry re-reads
    /// it when its stamp changes.
    pub(crate) fn start_lsp_config_watch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |workbench, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(2))
                .await;
            let alive = workbench.update_in(cx, |w, window, cx| {
                let folders: Vec<PathBuf> = w
                    .workspaces
                    .iter()
                    .flat_map(|ws| ws.all_surfaces())
                    .filter_map(Surface::editor)
                    .filter_map(|e| e.read(cx).path.parent().map(Path::to_path_buf))
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect();
                let stamp = crate::lsp_hub::config_stamp(&folders);
                // The first look only records where things stand.
                if w.lsp_config_stamp.as_ref().is_some_and(|s| *s != stamp) {
                    w.lsp_config_changed(window, cx);
                }
                w.lsp_config_stamp = Some(stamp);
            });
            if alive.is_err() {
                break;
            }
        })
        .detach();
    }

    /// lsp.json changed: servers whose settings changed stop, and every
    /// open editor gets the servers now configured for it.
    fn lsp_config_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The servers are the app's: the first window to notice restarts
        // them, and the others' editors follow when they exit.
        let stopped = LspHub::reconfigure(cx);
        for index in 0..self.workspaces.len() {
            if self.workspaces[index].lsp.is_none() {
                continue;
            }
            self.lsp_progress
                .retain(|(key, _)| !stopped.iter().any(|id| key.starts_with(&format!("{id}:"))));
            let editors: Vec<Entity<EditorTab>> = self.workspaces[index]
                .all_surfaces()
                .filter_map(Surface::editor)
                .cloned()
                .collect();
            for editor in editors {
                editor.update(cx, |editor, cx| {
                    for id in &stopped {
                        if editor.serves(id) {
                            editor.detach_server(id, cx);
                        }
                    }
                });
                self.start_lsp(index, &editor, window, cx);
            }
        }
        cx.notify();
    }

    /// Editors of workspace `workspace_id` still on a server that was told
    /// to stop drop it and get the servers configured now.
    fn reattach_retired(&mut self, workspace_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.workspaces.iter().position(|w| w.id == workspace_id) else {
            return;
        };
        if self.workspaces[index].lsp.is_none() {
            return;
        }
        let editors: Vec<Entity<EditorTab>> = self.workspaces[index]
            .all_surfaces()
            .filter_map(Surface::editor)
            .cloned()
            .collect();
        for editor in editors {
            if editor.update(cx, |editor, cx| editor.detach_retired(cx)) {
                self.start_lsp(index, &editor, window, cx);
            }
        }
    }

    /// A crashed server's restart delay is over: start it again for the
    /// editors of workspace `workspace_id` it was serving.
    fn restart_server(
        &mut self,
        workspace_id: u64,
        server: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.workspaces.iter().position(|w| w.id == workspace_id) else {
            return;
        };
        let editors: Vec<Entity<EditorTab>> = self.workspaces[index]
            .all_surfaces()
            .filter_map(Surface::editor)
            .filter(|e| e.read(cx).serves(server))
            .cloned()
            .collect();
        for editor in editors {
            editor.update(cx, |editor, cx| editor.detach_server(server, cx));
            self.start_lsp(index, &editor, window, cx);
        }
    }

    /// What a language server of workspace `workspace_id` sent on its own.
    fn server_event(
        &mut self,
        workspace_id: u64,
        event: impulse_lsp::ServerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            impulse_lsp::ServerEvent::ApplyEdit {
                edit,
                label,
                root,
                utf32,
            } => {
                // The file the user is in, else the server's root, decides
                // what the edit may change without asking.
                let origin = self
                    .active_editor()
                    .map(|e| e.read(cx).path.clone())
                    .unwrap_or_else(|| root.clone());
                let pending = crate::workspace_edits::PendingEdit {
                    edit: impulse_lsp::workspace_edit::WorkspaceEdit::from(&edit),
                    utf32,
                    origin,
                    server_root: root,
                    title: label.unwrap_or_else(|| "Language server edit".into()),
                };
                self.apply_workspace_edit(pending, window, cx);
            }
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
            // 4 is a log message: not worth interrupting for.
            impulse_lsp::ServerEvent::ShowMessage { server, kind, text } if kind <= 3 => {
                let (toast_kind, seconds) = match kind {
                    1 => (crate::toasts::Kind::Error, 12),
                    2 => (crate::toasts::Kind::Warning, 6),
                    _ => (crate::toasts::Kind::Info, 6),
                };
                crate::toasts::show(
                    window,
                    cx,
                    crate::toasts::Toast::new(toast_kind, format!("{server}: {text}"))
                        .lifetime(Some(std::time::Duration::from_secs(seconds))),
                );
            }
            impulse_lsp::ServerEvent::ShowMessage { .. } => {}
            impulse_lsp::ServerEvent::Unavailable { message, .. } => {
                self.tell_lsp_once(message, window, cx)
            }
            impulse_lsp::ServerEvent::Exited {
                server,
                root,
                expected,
            } => {
                if expected {
                    // Stopped on purpose (lsp.json changed, maybe in another
                    // window): editors still on it move to its successor.
                    if !QUITTING.load(std::sync::atomic::Ordering::SeqCst) {
                        self.reattach_retired(workspace_id, window, cx);
                    }
                    return;
                }
                let Some(hub) = self
                    .workspaces
                    .iter()
                    .find(|w| w.id == workspace_id)
                    .and_then(|w| w.lsp.clone())
                else {
                    return;
                };
                let delay = hub.server_exited(&server, &root);
                // Its work in progress went with it.
                let prefix = format!("{server}:");
                self.lsp_progress
                    .retain(|(key, _)| !key.starts_with(&prefix));
                cx.notify();
                log::warn!("language server {server} exited; restarting in {delay:?}");
                self.tell_lsp_once(
                    format!(
                        "The {server} language server stopped unexpectedly. Impulse restarts it."
                    ),
                    window,
                    cx,
                );
                cx.spawn_in(window, async move |workbench, cx| {
                    cx.background_executor()
                        .timer(delay + std::time::Duration::from_millis(100))
                        .await;
                    let _ = workbench.update_in(cx, |workbench, window, cx| {
                        workbench.restart_server(workspace_id, &server, window, cx)
                    });
                })
                .detach();
            }
            impulse_lsp::ServerEvent::Progress {
                server,
                token,
                kind,
                title,
                message,
                percentage,
            } => {
                let key = format!("{server}:{token}");
                if kind == "end" {
                    self.lsp_progress.retain(|(k, _)| *k != key);
                } else {
                    let index = match self.lsp_progress.iter().position(|(k, _)| *k == key) {
                        Some(index) => index,
                        None => {
                            self.lsp_progress.push((
                                key,
                                LspProgress {
                                    title: server.clone(),
                                    server,
                                    message: None,
                                    fraction: None,
                                },
                            ));
                            self.lsp_progress.len() - 1
                        }
                    };
                    let status = &mut self.lsp_progress[index].1;
                    if let Some(title) = title {
                        status.title = title;
                    }
                    if message.is_some() {
                        status.message = message;
                    }
                    if let Some(percentage) = percentage {
                        status.fraction = Some(percentage as f32 / 100.);
                    }
                }
                cx.notify();
            }
        }
    }
}

/// A language server's work in progress, for the status bar (Swift's
/// `LspProgressStatus`).
#[derive(Clone)]
pub(crate) struct LspProgress {
    pub server: String,
    pub title: String,
    pub message: Option<String>,
    pub fraction: Option<f32>,
}

/// Set at quit, when servers stopping mustn't start again.
static QUITTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// At quit: ask every language server to shut down at once, and stop any
/// still running two seconds later.
pub(crate) fn shutdown_servers(cx: &mut gpui_kit::App) -> impl std::future::Future<Output = ()> {
    use futures::FutureExt as _;
    QUITTING.store(true, std::sync::atomic::Ordering::SeqCst);
    let clients = LspHub::running();
    let timer = cx
        .background_executor()
        .timer(std::time::Duration::from_secs(2));
    async move {
        if clients.is_empty() {
            return;
        }
        let all = futures::future::join_all(clients.iter().map(|c| c.shutdown()));
        futures::select! {
            _ = all.fuse() => {}
            _ = timer.fuse() => {}
        }
        for client in &clients {
            client.kill();
        }
    }
}

/// A folder's last component, as the trust messages name it.
fn folder_name(folder: &str) -> String {
    Path::new(folder)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| folder.to_string())
}

/// Forget the trusted project files of `folder`: those of repositories
/// inside it and of the repository it's in (trust is kept per main
/// repository, so a task's folder names its repository's), as Swift's
/// `forgetProjectTrust`.
fn forget_project_trust(folder: &str) {
    let mut projects = crate::tasks::project_trust();
    projects.revoke_within(folder);
    projects.revoke(&crate::tasks::repository_trust_key(folder));
    crate::tasks::save_project_trust(&projects);
}

/// The run's trust store, for work off the main thread that has no window
/// to ask (agent checkpoints, overlap checks).
static SHARED_TRUST: std::sync::OnceLock<std::sync::Arc<impulse_kit::trust::WorkspaceTrust>> =
    std::sync::OnceLock::new();

pub(crate) fn share_trust(trust: std::sync::Arc<impulse_kit::trust::WorkspaceTrust>) {
    let _ = SHARED_TRUST.set(trust);
}

/// The trust list every window shares.
pub(crate) fn shared_trust() -> Option<std::sync::Arc<impulse_kit::trust::WorkspaceTrust>> {
    SHARED_TRUST.get().cloned()
}

/// Whether `path` is in a trusted folder (not before the store is shared).
pub(crate) fn is_trusted(path: &str) -> bool {
    SHARED_TRUST
        .get()
        .is_some_and(|trust| trust.is_trusted(path))
}
