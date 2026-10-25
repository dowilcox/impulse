//! The sidebar's Search panel as the window uses it: opening a result at
//! its line, and Replace All (the Swift `ProjectReplace` flow): confirm,
//! rewrite every file with a match except those with unsaved edits, show
//! open editors the new text, and offer Undo while the files still hold
//! what was written.

use std::collections::BTreeSet;
use std::path::PathBuf;

use gpui_kit::{Context, Entity, Window};
use impulse_kit::text_file;

use crate::search_panel::{SearchPanel, SearchPanelEvent};
use crate::workbench::Workbench;
use crate::workspace::Surface;

/// A file Replace All rewrote: what it held before, and what it wrote.
#[derive(Clone)]
struct Rewritten {
    path: PathBuf,
    before: String,
    after: String,
    bom: bool,
}

impl Workbench {
    pub(crate) fn search_event(
        &mut self,
        panel: Entity<SearchPanel>,
        event: &SearchPanelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SearchPanelEvent::Close => {
                self.panel = crate::workbench::SidebarPanel::Files;
                cx.notify();
            }
            SearchPanelEvent::Open(path, line) => {
                self.open_file(path, window, cx);
                if let (Some(line), Some(editor)) = (line, self.active_editor().cloned()) {
                    let position = gpui_kit::component::input::Position {
                        line: line.saturating_sub(1),
                        character: 0,
                    };
                    editor.update(cx, |editor, cx| {
                        editor.go_to(position, window, cx);
                        editor.focus(window, cx);
                    });
                }
            }
            SearchPanelEvent::ReplaceAll {
                query,
                replacement,
                case_sensitive,
            } => {
                let files: BTreeSet<PathBuf> = panel
                    .read(cx)
                    .results
                    .iter()
                    .filter(|r| r.match_type == "content")
                    .map(|r| PathBuf::from(&r.path))
                    .collect();
                if files.is_empty() {
                    return;
                }
                let count = files.len();
                let title = if count == 1 {
                    "Replace in 1 file?".to_string()
                } else {
                    format!("Replace in {count} files?")
                };
                let message =
                    format!("Every \u{201c}{query}\u{201d} becomes \u{201c}{replacement}\u{201d}.");
                let (query, replacement, case_sensitive) =
                    (query.clone(), replacement.clone(), *case_sensitive);
                self.git_confirm(
                    title,
                    message,
                    "Replace All",
                    false,
                    window,
                    cx,
                    move |this, window, cx| {
                        this.replace_in_files(
                            &files,
                            &query,
                            &replacement,
                            case_sensitive,
                            &panel,
                            window,
                            cx,
                        )
                    },
                );
            }
        }
    }

    /// Paths with an editor holding unsaved edits, in any window.
    fn dirty_paths(&self, cx: &gpui_kit::App) -> BTreeSet<PathBuf> {
        crate::editor_tab::unsaved_paths(cx).into_iter().collect()
    }

    /// Show every open editor of `paths` what's on disk now.
    fn reload_editors(&self, paths: &[PathBuf], window: &mut Window, cx: &mut Context<Self>) {
        let editors: Vec<_> = self
            .workspaces
            .iter()
            .flat_map(|w| w.all_surfaces())
            .filter_map(Surface::editor)
            .filter(|e| paths.contains(&e.read(cx).path))
            .cloned()
            .collect();
        for editor in editors {
            editor.update(cx, |editor, cx| editor.reload_from_disk(window, cx));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn replace_in_files(
        &mut self,
        files: &BTreeSet<PathBuf>,
        query: &str,
        replacement: &str,
        case_sensitive: bool,
        panel: &Entity<SearchPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dirty = self.dirty_paths(cx);
        let (files, query, replacement) =
            (files.clone(), query.to_string(), replacement.to_string());
        let panel = panel.clone();
        // The files are read and written off the main thread.
        let work = cx.background_executor().spawn(async move {
            let (mut rewritten, mut matches, mut skipped, mut failed) = (Vec::new(), 0, 0, 0);
            for path in &files {
                if dirty.contains(path) {
                    skipped += 1;
                    continue;
                }
                let Some(file) = text_file::read(path) else {
                    failed += 1;
                    continue;
                };
                let (after, count) = impulse_kit::project_replace::replace(
                    &file.text,
                    &query,
                    &replacement,
                    case_sensitive,
                );
                if count == 0 {
                    continue;
                }
                if text_file::write(&after, file.bom, path).is_err() {
                    failed += 1;
                    continue;
                }
                matches += count;
                rewritten.push(Rewritten {
                    path: path.clone(),
                    before: file.text,
                    after,
                    bom: file.bom,
                });
            }
            (rewritten, matches, skipped, failed)
        });
        cx.spawn_in(window, async move |this, cx| {
            let (rewritten, matches, skipped, failed) = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.replaced(rewritten, matches, skipped, failed, &panel, window, cx)
            });
        })
        .detach();
    }

    /// Replace All is done: show the new text and say what happened.
    #[allow(clippy::too_many_arguments)]
    fn replaced(
        &mut self,
        rewritten: Vec<Rewritten>,
        matches: usize,
        skipped: usize,
        failed: usize,
        panel: &Entity<SearchPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths: Vec<PathBuf> = rewritten.iter().map(|r| r.path.clone()).collect();
        self.reload_editors(&paths, window, cx);
        panel.update(cx, |panel, cx| {
            panel.schedule(std::time::Duration::ZERO, cx)
        });
        let mut message = format!(
            "Replaced {} in {}",
            plural(matches, "match", "matches"),
            plural(rewritten.len(), "file", "files")
        );
        let mut notes = Vec::new();
        if skipped > 0 {
            notes.push(format!(
                "{} with unsaved changes left alone",
                plural(skipped, "file", "files")
            ));
        }
        if failed > 0 {
            notes.push(format!(
                "{} couldn't be written",
                plural(failed, "file", "files")
            ));
        }
        if !notes.is_empty() {
            message = format!("{message} ({})", notes.join(", "));
        }
        if rewritten.is_empty() {
            self.set_notice(message, cx);
            return;
        }
        let panel = panel.clone();
        self.toast_with_action(
            message,
            None,
            "Undo",
            window,
            cx,
            move |this, window, cx| this.undo_replace(&rewritten, &panel, window, cx),
        );
    }

    /// Put back what Replace All changed, in the files that still hold
    /// exactly what it wrote.
    fn undo_replace(
        &mut self,
        rewritten: &[Rewritten],
        panel: &Entity<SearchPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dirty = self.dirty_paths(cx);
        let (mut restored, mut changed) = (Vec::new(), 0);
        for file in rewritten {
            let unchanged = !dirty.contains(&file.path)
                && text_file::read(&file.path).is_some_and(|now| now.text == file.after);
            if unchanged && text_file::write(&file.before, file.bom, &file.path).is_ok() {
                restored.push(file.path.clone());
            } else {
                changed += 1;
            }
        }
        self.reload_editors(&restored, window, cx);
        panel.update(cx, |panel, cx| {
            panel.schedule(std::time::Duration::ZERO, cx)
        });
        let message = if changed == 0 {
            "Replace undone".to_string()
        } else {
            format!(
                "Replace undone, except in {} changed since",
                plural(changed, "file", "files")
            )
        };
        self.set_notice(message, cx);
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}
