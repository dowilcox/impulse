//! Asking before closing (the Swift `MainWindowController+TabClose` and the
//! window and quit checks): a pinned tab, each editor with unsaved changes
//! (Save, Don't Save, Cancel), then the commands and processes still
//! running in its terminals (`impulse_kit::close_risk`, when
//! `confirm_close_warnings` is on). Closing a pane, a tab, a window and
//! quitting all go through it.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::{App, Context, EntityId, Window};
use impulse_kit::close_risk::{CloseRiskAction, CloseRiskInput, RunningCommandRisk};

use crate::settings_store::SettingsStore;
use crate::terminal_view::TerminalView;
use crate::workbench::Workbench;
use crate::workspace::{PaneId, Surface};

/// What's being closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Pane { tab: u64, pane: PaneId },
    Tab(u64),
    Workspace(u64),
    Window,
}

/// The questions already answered for this close.
#[derive(Clone, Debug, Default)]
pub(crate) struct Answered {
    pinned: bool,
    /// Editors whose changes were saved or may go.
    editors: Vec<EntityId>,
    running: bool,
}

impl TerminalView {
    /// Processes running under the shell (none for an idle prompt).
    pub(crate) fn running_processes(&self) -> usize {
        let mut pending = vec![self.shell_pid()];
        let mut count = 0;
        while let Some(pid) = pending.pop() {
            if pid <= 0 {
                continue;
            }
            for child in crate::agent_process::children(pid) {
                count += 1;
                pending.push(child);
            }
        }
        count
    }

    /// The command block still running, for the close question.
    pub(crate) fn running_command_risk(&self) -> Option<RunningCommandRisk> {
        let block = self
            .backend
            .command_block_summaries()
            .into_iter()
            .rev()
            .find(|b| b.ended_at_ms.is_none() && b.exit_code.is_none())?;
        Some(RunningCommandRisk {
            command: block.command,
            cwd: block.cwd.or_else(|| Some(self.cwd.clone())),
            started_at_ms: block.started_at_ms,
        })
    }
}

impl Workbench {
    /// The surfaces `target` closes, and whether it's a pinned tab.
    fn target_surfaces(&self, target: Target) -> Option<(Vec<Surface>, bool)> {
        let find = |id: u64| {
            self.workspaces
                .iter()
                .flat_map(|w| w.tabs.iter())
                .find(|t| t.id == id)
        };
        match target {
            Target::Pane { tab, pane } => {
                let tab = find(tab)?;
                Some((vec![tab.panes.get(&pane)?.clone()], false))
            }
            Target::Tab(id) => {
                let tab = find(id)?;
                Some((tab.panes.values().cloned().collect(), tab.pinned))
            }
            Target::Workspace(id) => Some((
                self.workspaces
                    .iter()
                    .find(|w| w.id == id)?
                    .all_surfaces()
                    .cloned()
                    .collect(),
                false,
            )),
            Target::Window => Some((
                self.workspaces
                    .iter()
                    .flat_map(|w| w.tabs.iter())
                    .flat_map(|t| t.panes.values().cloned())
                    .collect(),
                false,
            )),
        }
    }

    /// Ask what closing `target` needs asked, one question at a time, then
    /// run `done` (the close itself). Cancelling any question stops it.
    pub(crate) fn confirm_close(
        &mut self,
        target: Target,
        action: CloseRiskAction,
        answered: Answered,
        window: &mut Window,
        cx: &mut Context<Self>,
        done: Then,
    ) {
        let Some((surfaces, pinned)) = self.target_surfaces(target) else {
            return;
        };
        let workbench = cx.entity().downgrade();
        let again = {
            let done = done.clone();
            move |answered: Answered, window: &mut Window, cx: &mut App| {
                let done = done.clone();
                let _ = workbench.update(cx, |w, cx| {
                    w.confirm_close(target, action, answered, window, cx, done)
                });
            }
        };
        if pinned && !answered.pinned {
            let again = again.clone();
            return crate::tasks::confirm(
                window,
                cx,
                "Pinned Tab".into(),
                "This tab is pinned. Close anyway?".into(),
                "Close",
                "Cancel",
                true,
                move |yes, window, cx| {
                    if yes {
                        again(
                            Answered {
                                pinned: true,
                                ..answered
                            },
                            window,
                            cx,
                        )
                    }
                },
            );
        }
        // Each editor with unsaved changes, in turn.
        let dirty = surfaces.iter().find_map(|s| match s {
            Surface::Editor(e)
                if e.read(cx).dirty && !answered.editors.contains(&e.entity_id()) =>
            {
                Some(e.clone())
            }
            _ => None,
        });
        if let Some(editor) = dirty {
            let name = editor.read(cx).title.clone();
            let workbench = cx.entity().downgrade();
            return choice_dialog(
                &name,
                target == Target::Window,
                window,
                cx,
                move |choice, window, cx| {
                    let mut answered = answered.clone();
                    match choice {
                        Choice::Save => {
                            if editor.read(cx).untitled.is_some() {
                                // Save As first; closing waits for another try.
                                let _ = workbench.update(cx, |w, cx| {
                                    w.save_untitled(editor.clone(), window, cx)
                                });
                                return;
                            }
                            if editor.update(cx, |e, cx| e.write(cx)).is_none() {
                                return;
                            }
                        }
                        Choice::DontSave => {}
                        Choice::Cancel => return,
                    }
                    answered.editors.push(editor.entity_id());
                    again(answered, window, cx);
                },
            );
        }
        // Other surfaces with unsaved changes (Project Setup) can only be
        // discarded here.
        let modified = surfaces.iter().find(|s| {
            !matches!(s, Surface::Editor(_))
                && s.is_dirty(cx)
                && !answered.editors.contains(&crate::workbench::surface_id(s))
        });
        if let Some(surface) = modified {
            let (id, name) = (crate::workbench::surface_id(surface), surface.name(cx));
            return crate::tasks::confirm(
                window,
                cx,
                format!("{name} has unsaved changes"),
                "Your changes will be lost if you close it.".into(),
                "Discard",
                "Cancel",
                true,
                move |yes, window, cx| {
                    if yes {
                        let mut answered = answered.clone();
                        answered.editors.push(id);
                        again(answered, window, cx)
                    }
                },
            );
        }
        if !answered.running {
            let terminals: Vec<_> = surfaces.iter().filter_map(Surface::terminal).collect();
            if let Some((title, text, ok)) = running_question(&terminals, action, cx) {
                return crate::tasks::confirm(
                    window,
                    cx,
                    title,
                    text,
                    ok,
                    "Cancel",
                    true,
                    move |yes, window, cx| {
                        if yes {
                            again(
                                Answered {
                                    running: true,
                                    ..answered
                                },
                                window,
                                cx,
                            )
                        }
                    },
                );
            }
        }
        done(self, window, cx);
    }
}

/// The question to ask about the work still running in `terminals`
/// (`impulse_kit::close_risk`): its title, text and the button that goes
/// ahead; `None` when there's nothing to ask or the setting is off.
fn running_question(
    terminals: &[&gpui_kit::Entity<TerminalView>],
    action: CloseRiskAction,
    cx: &App,
) -> Option<(String, String, &'static str)> {
    let settings = SettingsStore::get(cx);
    if !settings.confirm_close_warnings {
        return None;
    }
    let input = CloseRiskInput {
        action,
        unsaved_editor_count: 0,
        running_terminal_process_count: terminals
            .iter()
            .map(|t| t.read(cx).running_processes())
            .sum(),
        running_commands: terminals
            .iter()
            .filter_map(|t| t.read(cx).running_command_risk())
            .collect(),
        now_ms: now_ms(),
        long_command_threshold_seconds: settings.terminal_long_command_seconds.max(1) as u64,
    };
    let summary = input.summarize();
    if !summary.has_risk {
        return None;
    }
    let details = summary.detail_lines.join("\n");
    let text = [summary.informative_text.clone(), details]
        .into_iter()
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let ok = match action {
        CloseRiskAction::Quit => "Quit",
        CloseRiskAction::CloseWindow => "Close Window",
        CloseRiskAction::CloseTab => "Close Tab",
    };
    Some((summary.title, text, ok))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

type Windows = Vec<(gpui_kit::Entity<Workbench>, gpui_kit::AnyWindowHandle)>;

/// Quit, asking first as the Swift `applicationShouldTerminate` does: with
/// unsaved documents in any window, whether to review them (each one's
/// save question in turn), cancel or discard them; then, once, about the
/// work still running in every window's terminals.
pub(crate) fn quit(cx: &mut App) {
    // The action comes from inside a window's update: ask once it's done.
    cx.defer(|cx| {
        let windows = crate::session::workbenches(cx);
        let dirty: usize = windows
            .iter()
            .map(|(w, _)| {
                w.read(cx)
                    .workspaces
                    .iter()
                    .flat_map(|ws| ws.all_surfaces())
                    .filter(|s| s.is_dirty(cx))
                    .count()
            })
            .sum();
        if dirty == 0 {
            return confirm_quit(cx);
        }
        let Some(front) = front_window(&windows, cx) else {
            return cx.quit();
        };
        let title = if dirty == 1 {
            "You have unsaved changes in 1 document. Do you want to review this change before quitting?".to_string()
        } else {
            format!("You have unsaved changes in {dirty} documents. Do you want to review these changes before quitting?")
        };
        let _ = front.update(cx, move |_, window, cx| {
            ask(
                title,
                "If you don't review your documents, all your changes will be lost.".into(),
                [
                    ("Review Changes…", Choice::Save),
                    ("Cancel", Choice::Cancel),
                    ("Discard Changes", Choice::DontSave),
                ],
                window,
                cx,
                move |choice, _, cx| match choice {
                    Choice::Save => App::defer(cx, move |cx| review_for_quit(windows, cx)),
                    Choice::DontSave => App::defer(cx, confirm_quit),
                    Choice::Cancel => {}
                },
            )
        });
    });
}

/// The window in front, else the last one.
fn front_window(windows: &Windows, cx: &App) -> Option<gpui_kit::AnyWindowHandle> {
    let active = cx.active_window();
    windows
        .iter()
        .map(|(_, handle)| *handle)
        .find(|handle| Some(*handle) == active)
        .or_else(|| windows.last().map(|(_, handle)| *handle))
}

/// Each window's unsaved documents in turn, brought to the front; then the
/// question about running work. Cancelling any stops the quit.
fn review_for_quit(mut windows: Windows, cx: &mut App) {
    let Some((workbench, handle)) = windows.pop() else {
        return confirm_quit(cx);
    };
    let _ = handle.update(cx, move |_, window, cx| {
        window.activate_window();
        workbench.update(cx, |w, cx| {
            let rest = RefCell::new(Some(windows));
            w.confirm_close(
                Target::Window,
                CloseRiskAction::Quit,
                // Running work is asked about once, for every window.
                Answered {
                    running: true,
                    ..Default::default()
                },
                window,
                cx,
                Rc::new(move |_, _, cx| {
                    if let Some(rest) = rest.borrow_mut().take() {
                        App::defer(cx, move |cx| review_for_quit(rest, cx));
                    }
                }),
            )
        });
    });
}

/// How long quitting waits for commands on save (Swift's
/// `commandsOnSaveQuitTimeout`).
const COMMANDS_ON_SAVE_QUIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Quit once the commands on save have finished (quitting would kill them
/// mid-run), saying so in the front window while it waits, at most
/// `COMMANDS_ON_SAVE_QUIT_TIMEOUT`.
fn quit_after_commands_on_save(cx: &mut App) {
    use std::sync::atomic::Ordering;
    if crate::save_pipeline::COMMANDS_RUNNING.load(Ordering::SeqCst) == 0 {
        return cx.quit();
    }
    let windows = crate::session::workbenches(cx);
    if let Some(front) = front_window(&windows, cx) {
        let _ = front.update(cx, |_, window, cx| {
            crate::toasts::show(
                window,
                cx,
                crate::toasts::Toast::info("Waiting for commands on save to finish…")
                    .lifetime(None),
            )
        });
    }
    cx.spawn(async move |cx| {
        let started = std::time::Instant::now();
        while crate::save_pipeline::COMMANDS_RUNNING.load(Ordering::SeqCst) > 0
            && started.elapsed() < COMMANDS_ON_SAVE_QUIT_TIMEOUT
        {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(100))
                .await;
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}

/// Ask once about the work still running in every window, then quit.
fn confirm_quit(cx: &mut App) {
    let windows = crate::session::workbenches(cx);
    let terminals: Vec<_> = windows
        .iter()
        .flat_map(|(w, _)| {
            w.read(cx)
                .workspaces
                .iter()
                .flat_map(|ws| ws.all_surfaces())
                .filter_map(Surface::terminal)
                .cloned()
                .collect::<Vec<_>>()
        })
        .collect();
    let question = running_question(
        &terminals.iter().collect::<Vec<_>>(),
        CloseRiskAction::Quit,
        cx,
    );
    let (Some((title, text, ok)), Some(front)) = (question, front_window(&windows, cx)) else {
        return quit_after_commands_on_save(cx);
    };
    let _ = front.update(cx, move |_, window, cx| {
        crate::tasks::confirm(window, cx, title, text, ok, "Cancel", true, |yes, _, cx| {
            if yes {
                quit_after_commands_on_save(cx)
            }
        })
    });
}

#[derive(Clone, Copy)]
enum Choice {
    Save,
    DontSave,
    Cancel,
}

type Chosen = Rc<RefCell<Option<Box<dyn FnOnce(Choice, &mut Window, &mut App)>>>>;

/// What to do with an editor's unsaved changes. Closing the window or
/// quitting asks as a document's close sheet does (Save, Cancel, Don't
/// Save); closing a pane, tab or workspace asks as Swift's `confirmClosing`
/// does (Save & Close, Don't Save, Cancel).
fn choice_dialog(
    name: &str,
    window_close: bool,
    window: &mut Window,
    cx: &mut App,
    answer: impl FnOnce(Choice, &mut Window, &mut App) + 'static,
) {
    let (title, message, buttons) = if window_close {
        (
            format!("Do you want to save the changes made to {name}?"),
            "Your changes will be lost if you don't save them.".to_string(),
            [
                ("Save", Choice::Save),
                ("Cancel", Choice::Cancel),
                ("Don't Save", Choice::DontSave),
            ],
        )
    } else {
        (
            "Unsaved Changes".to_string(),
            format!("\"{name}\" has unsaved changes. Close anyway?"),
            [
                ("Save & Close", Choice::Save),
                ("Don't Save", Choice::DontSave),
                ("Cancel", Choice::Cancel),
            ],
        )
    };
    ask(title, message, buttons, window, cx, answer)
}

/// A question with three answers, the first the default.
fn ask(
    title: String,
    message: String,
    buttons: [(&'static str, Choice); 3],
    window: &mut Window,
    cx: &mut App,
    answer: impl FnOnce(Choice, &mut Window, &mut App) + 'static,
) {
    let chosen: Chosen = Rc::new(RefCell::new(Some(Box::new(answer))));
    #[cfg(debug_assertions)]
    {
        let press = |choice: Choice| -> crate::debug_snapshot::Press {
            let chosen = chosen.clone();
            Rc::new(move |window, cx| {
                window.close_dialog(cx);
                if let Some(chosen) = chosen.borrow_mut().take() {
                    chosen(choice, window, cx)
                }
            })
        };
        crate::debug_snapshot::note_sheet(
            vec![title.clone(), message.clone()],
            buttons
                .iter()
                .map(|&(label, choice)| (label, press(choice)))
                .collect(),
        );
    }
    let reply = |choice: Choice| {
        let chosen = chosen.clone();
        move |window: &mut Window, cx: &mut App| {
            window.close_dialog(cx);
            if let Some(chosen) = chosen.borrow_mut().take() {
                chosen(choice, window, cx)
            }
        }
    };
    let mut alert = crate::alert::Alert::new(title).message(message);
    for (label, choice) in buttons {
        alert = alert.button(label, reply(choice));
    }
    alert.show(window, cx);
}

impl Workbench {
    /// Close this window, asking first.
    pub(crate) fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_close(
            Target::Window,
            CloseRiskAction::CloseWindow,
            Answered::default(),
            window,
            cx,
            Rc::new(|w, window, cx| {
                w.close_confirmed = true;
                // The last window closing quits, as the Swift app's
                // `applicationShouldTerminateAfterLastWindowClosed` does,
                // with the session saved while the window is still there to
                // save (closing was already asked about).
                let last = crate::session::workbenches(cx).len() <= 1;
                if !last {
                    return window.remove_window();
                }
                // Saving reads every workbench, this one included: once
                // its update is over.
                window.defer(cx, |window, cx| {
                    crate::session::save(cx);
                    window.remove_window();
                    cx.quit();
                });
            }),
        );
    }

    /// The window's close button (or the window manager) asks first too.
    pub(crate) fn guard_window_close(&self, window: &mut Window, cx: &mut Context<Self>) {
        let workbench = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            workbench
                .update(cx, |w, cx| {
                    if w.close_confirmed {
                        return true;
                    }
                    let entity = cx.entity();
                    window.defer(cx, move |window, cx| {
                        entity.update(cx, |w, cx| w.close_window(window, cx))
                    });
                    false
                })
                .unwrap_or(true)
        });
    }
}

/// What runs once closing goes ahead.
type Then = Rc<dyn Fn(&mut Workbench, &mut Window, &mut Context<Workbench>)>;
