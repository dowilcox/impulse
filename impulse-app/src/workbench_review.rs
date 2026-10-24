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
        cx.subscribe_in(
            &review,
            window,
            |workbench, _, event, window, cx| match event {
                ReviewEvent::OpenFile(path, line) => {
                    workbench.open_file(&PathBuf::from(path), window, cx);
                    if let (Some(line), Some(editor)) = (line, workbench.active_editor().cloned()) {
                        let position = lsp_types::Position::new(line.saturating_sub(1), 0);
                        editor.update(cx, |editor, cx| editor.go_to(position, window, cx));
                    }
                }
            },
        )
        .detach();
        Surface::Review(review)
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
        let surface = self.make_history(root, path, window, cx);
        let active = self.active;
        self.add_tab(active, surface, window, cx);
    }

    pub(crate) fn make_history(
        &mut self,
        root: String,
        path: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Surface {
        let (theme, palette) = (self.theme.clone(), self.palette.clone());
        let history = cx.new(|cx| HistoryView::new(root, path, &theme, &palette, cx));
        cx.observe(&history, |_, _, cx| cx.notify()).detach();
        let review = history.read(cx).review().clone();
        cx.subscribe_in(
            &review,
            window,
            |workbench, _, event, window, cx| match event {
                ReviewEvent::OpenFile(path, line) => {
                    workbench.open_file(&PathBuf::from(path), window, cx);
                    if let (Some(line), Some(editor)) = (line, workbench.active_editor().cloned()) {
                        let position = lsp_types::Position::new(line.saturating_sub(1), 0);
                        editor.update(cx, |editor, cx| editor.go_to(position, window, cx));
                    }
                }
            },
        )
        .detach();
        Surface::History(history)
    }
}
