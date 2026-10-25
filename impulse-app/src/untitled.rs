//! New File (an editor for a file not saved yet, saved with Save As) and
//! Install Language Servers, the Swift app's `addUntitledEditorTab`,
//! `showSaveAsDialog` and `lspInstall`.

use std::path::{Path, PathBuf};

use gpui_kit::{AppContext as _, Context, Entity, Window};

use crate::editor_tab::{EditorTab, EditorTabChanged};
use crate::tasks::{toast, ToastKind};
use crate::workbench::Workbench;
use crate::workbench_chrome::abbreviate_home;
use crate::workspace::Surface;

impl Workbench {
    /// New File: an empty editor whose Save asks where it goes, starting in
    /// the focused terminal's folder (or the workspace's).
    pub(crate) fn new_untitled(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self
            .active_terminal()
            .map(|t| PathBuf::from(&t.read(cx).cwd))
            .filter(|d| d.is_dir())
            .unwrap_or_else(|| self.workspace().default_directory());
        let editor = cx.new(|cx| EditorTab::untitled(&dir, window, cx));
        cx.subscribe(&editor, |_, _, _: &EditorTabChanged, cx| cx.notify())
            .detach();
        cx.observe(&editor, |_, _, cx| cx.notify()).detach();
        let index = self.active;
        self.add_tab(index, Surface::Editor(editor), window, cx);
    }

    /// Save an untitled editor: the system's save dialog, or where there's
    /// none (Linux without a desktop portal) a path typed in Impulse.
    pub(crate) fn save_untitled(
        &mut self,
        editor: Entity<EditorTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_untitled_then(editor, window, cx, None)
    }

    /// `save_untitled`, then `then` once the file is written (a close or
    /// quit carrying on).
    pub(crate) fn save_untitled_then(
        &mut self,
        editor: Entity<EditorTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: Option<crate::alert::AlertAction>,
    ) {
        let Some(dir) = editor.read(cx).untitled.clone() else {
            return;
        };
        let chosen = cx.prompt_for_new_path(&dir, Some("Untitled"));
        cx.spawn_in(window, async move |this, cx| {
            let answer = chosen.await;
            let _ = this.update_in(cx, |this, window, cx| match answer {
                Ok(Ok(Some(path))) => {
                    if this.save_untitled_to(&editor, &path, window, cx) {
                        if let Some(then) = then.clone() {
                            cx.defer_in(window, move |_, window, cx| then(window, cx));
                        }
                    }
                }
                Ok(Ok(None)) => {}
                _ => {
                    let initial = dir.join("Untitled").to_string_lossy().into_owned();
                    this.text_prompt(
                        "Save As",
                        "Where to save the file:",
                        &initial,
                        "Save",
                        window,
                        cx,
                        move |this, path, window, cx| {
                            let path = PathBuf::from(path.trim());
                            if !path.as_os_str().is_empty()
                                && this.save_untitled_to(&editor, &path, window, cx)
                            {
                                if let Some(then) = then.clone() {
                                    cx.defer_in(window, move |_, window, cx| then(window, cx));
                                }
                            }
                        },
                    );
                }
            });
        })
        .detach();
    }

    /// Write the untitled editor's text to `path` and show the file in its
    /// place, as an editor of that file (its language, its servers).
    /// True when the file was written.
    fn save_untitled_to(
        &mut self,
        editor: &Entity<EditorTab>,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let path = if path.is_relative() {
            editor
                .read(cx)
                .untitled
                .clone()
                .unwrap_or_default()
                .join(path)
        } else {
            path.to_path_buf()
        };
        let text = editor.read(cx).text(cx);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(error) = impulse_kit::text_file::write(&text, false, &path) {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            toast(
                window,
                cx,
                ToastKind::Warning,
                format!("Couldn't save {name}: {error}"),
                None,
                None,
            );
            return false;
        }
        let Some(Surface::Editor(saved)) = self.make_editor(&path, window, cx) else {
            return false;
        };
        // The pane that showed the untitled editor shows the file now.
        let found = self.workspaces.iter().enumerate().find_map(|(w, ws)| {
            ws.tabs.iter().enumerate().find_map(|(t, tab)| {
                tab.panes
                    .iter()
                    .find(|(_, s)| matches!(s, Surface::Editor(e) if e == editor))
                    .map(|(pane, _)| (w, t, *pane))
            })
        });
        if let Some((w, t, pane)) = found {
            self.workspaces[w].tabs[t]
                .panes
                .insert(pane, Surface::Editor(saved.clone()));
            self.start_lsp(w, &saved, window, cx);
        }
        self.reload_tree(cx);
        self.focus_active(window, cx);
        cx.notify();
        true
    }

    /// Install Language Servers: npm-installs the managed web servers.
    pub(crate) fn install_language_servers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        toast(
            window,
            cx,
            ToastKind::Info,
            "Installing language servers…".into(),
            None,
            None,
        );
        let task = cx.background_spawn(async { impulse_lsp::managed::install() });
        cx.spawn_in(window, async move |_, cx| {
            let result = task.await;
            let _ = cx.update(|window, cx| match result {
                Ok(path) => toast(
                    window,
                    cx,
                    ToastKind::Success,
                    format!(
                        "Language servers installed in {}",
                        abbreviate_home(&path.to_string_lossy())
                    ),
                    None,
                    None,
                ),
                Err(error) => toast(
                    window,
                    cx,
                    ToastKind::Warning,
                    format!("Couldn't install language servers: {error}"),
                    None,
                    None,
                ),
            });
        })
        .detach();
    }
}
