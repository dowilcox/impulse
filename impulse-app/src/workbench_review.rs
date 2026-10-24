//! Opening Review tabs (the Swift `TabManager.addReviewTab`): one per
//! repository in a workspace, reused when it's already open.

use std::path::PathBuf;

use gpui_kit::{AppContext as _, Context, Window};
use impulse_git::types::DiffScope;

use crate::history_view::HistoryView;
use crate::review_view::{ReviewEvent, ReviewView};
use crate::workbench::Workbench;
use crate::workspace::Surface;

impl Workbench {
    /// A Review of `scope` (by default what `default_scope` picks) for the
    /// repository the focused surface is in.
    pub(crate) fn open_review(
        &mut self,
        scope: Option<DiffScope>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dir = self
            .focused_surface()
            .and_then(|s| s.directory(cx))
            .unwrap_or_else(|| self.workspace().default_directory());
        let Some(root) = impulse_git::branches::repo_root(&dir.to_string_lossy())
            .map(|r| r.trim_end_matches('/').to_string())
        else {
            self.set_notice("Not in a git repository.".into(), cx);
            return;
        };
        self.open_review_of(root, scope, window, cx);
    }

    /// A Review of the repository at `root` in the active workspace,
    /// reusing the one open there.
    pub(crate) fn open_review_of(
        &mut self,
        root: String,
        scope: Option<DiffScope>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Already open in this workspace: show it.
        let existing = self
            .workspace()
            .tabs
            .iter()
            .enumerate()
            .find_map(|(t, tab)| {
                tab.surfaces().into_iter().find_map(|(pane, s)| match s {
                    Surface::Review(r) if r.read(cx).root == root => Some((t, pane, r.clone())),
                    _ => None,
                })
            });
        if let Some((t, pane, review)) = existing {
            self.workspace_mut().active = t;
            self.workspace_mut().tabs[t].focused = pane;
            if let Some(scope) = scope {
                review.update(cx, |r, cx| {
                    r.scope = scope;
                    r.refresh(cx)
                });
            }
            self.focus_active(window, cx);
            cx.notify();
            return;
        }
        let surface = self.make_review(root, scope, window, cx);
        let active = self.active;
        self.add_tab(active, surface, window, cx);
    }

    /// The file at `path` in an editor showing its Diff View.
    pub(crate) fn open_diff_editor(
        &mut self,
        path: &std::path::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_file(path, window, cx);
        if let Some(editor) = self.active_editor().cloned() {
            let result = editor.update(cx, |editor, cx| {
                if editor.diff.is_some() {
                    Ok(())
                } else {
                    editor.toggle_diff_view(window, cx)
                }
            });
            if let Err(message) = result {
                self.set_notice(message, cx);
            }
        }
    }

    /// Review of `scope`, scrolled to the file at `path`.
    pub(crate) fn open_review_at(
        &mut self,
        scope: DiffScope,
        path: &std::path::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_review(Some(scope), window, cx);
        if let Some(Surface::Review(review)) = self.focused_surface().cloned() {
            review.update(cx, |review, cx| review.reveal_path(path, cx));
        }
    }

    /// A Review surface wired to the workbench, not yet in a tab.
    pub(crate) fn make_review(
        &mut self,
        root: String,
        scope: Option<DiffScope>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Surface {
        let scope = scope.unwrap_or_else(|| crate::review_view::default_scope(&root));
        let (theme, palette) = (self.theme.clone(), self.palette.clone());
        let review = cx.new(|cx| ReviewView::new(root, scope, &theme, &palette, cx));
        cx.observe(&review, |_, _, cx| cx.notify()).detach();
        cx.subscribe_in(&review, window, |workbench, _, event, window, cx| {
            workbench.review_event(event, window, cx)
        })
        .detach();
        Surface::Review(review)
    }

    /// A commit SHA picked in a terminal: History of its repository, with
    /// the commit selected.
    pub(crate) fn open_commit(
        &mut self,
        sha: String,
        cwd: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dir = if cwd.is_empty() {
            self.workspace()
                .default_directory()
                .to_string_lossy()
                .into_owned()
        } else {
            cwd.to_string()
        };
        let Some(root) =
            impulse_git::branches::repo_root(&dir).map(|r| r.trim_end_matches('/').to_string())
        else {
            return crate::toasts::show(
                window,
                cx,
                crate::toasts::Toast::info(format!("{sha} isn't in a git repository here.")),
            );
        };
        let surface = self.make_history(root, None, None, window, cx);
        if let Surface::History(history) = &surface {
            history.update(cx, |history, cx| history.reveal(sha, cx));
        }
        let active = self.active;
        self.add_tab(active, surface, window, cx);
    }

    /// Show Git History, or the history of the focused file.
    pub(crate) fn open_history(
        &mut self,
        of_file: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let file = self.active_editor().map(|e| e.read(cx).path.clone());
        if of_file && file.is_none() {
            self.set_notice("Open a file to see its history.".into(), cx);
            return;
        }
        let dir = self
            .focused_surface()
            .and_then(|s| s.directory(cx))
            .unwrap_or_else(|| self.workspace().default_directory());
        let Some(root) = impulse_git::branches::repo_root(&dir.to_string_lossy())
            .map(|r| r.trim_end_matches('/').to_string())
        else {
            self.set_notice("Not in a git repository.".into(), cx);
            return;
        };
        let path = file.filter(|_| of_file).and_then(|f| {
            f.strip_prefix(&root)
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        });
        let surface = self.make_history(root, path, None, window, cx);
        let active = self.active;
        self.add_tab(active, surface, window, cx);
    }

    pub(crate) fn make_history(
        &mut self,
        root: String,
        path: Option<String>,
        branch: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Surface {
        let (theme, palette) = (self.theme.clone(), self.palette.clone());
        let scope = match branch {
            Some(branch) => impulse_git::log::LogScope::Branch(branch),
            None => impulse_git::log::LogScope::Head,
        };
        let history =
            cx.new(|cx| HistoryView::new(root, path, scope, &theme, &palette, window, cx));
        cx.observe(&history, |_, _, cx| cx.notify()).detach();
        cx.subscribe_in(&history, window, |workbench, history, event, window, cx| {
            let root = history.read(cx).root.clone();
            workbench.history_action(root, event, window, cx)
        })
        .detach();
        let review = history.read(cx).review().clone();
        cx.subscribe_in(&review, window, |workbench, _, event, window, cx| {
            workbench.review_event(event, window, cx)
        })
        .detach();
        Surface::History(history)
    }

    /// What a Review (or History's) asks of the workbench.
    fn review_event(&mut self, event: &ReviewEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            ReviewEvent::OpenFile(path, line) => {
                self.open_file(&PathBuf::from(path), window, cx);
                if let (Some(line), Some(editor)) = (line, self.active_editor().cloned()) {
                    let position = lsp_types::Position::new(line.saturating_sub(1), 0);
                    editor.update(cx, |editor, cx| editor.go_to(position, window, cx));
                }
            }
            ReviewEvent::OpenDiff(path) => self.open_diff_editor(path, window, cx),
            ReviewEvent::ShowChanges => self.run_command("show_changes", window, cx),
        }
    }
}
