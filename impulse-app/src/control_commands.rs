//! What `impulse` subcommands do in a window (the Swift `ControlCommands`).
//! The calling pane, found by its token, is the anchor: files open in its
//! window, splits split it. The token only names a pane, so commands that
//! start processes, write files or run git (split, tab, checkpoint,
//! creating a file to edit) also need the caller to be the pane's shell or a
//! program started from it.

use std::path::PathBuf;

use gpui_kit::{App, AppContext as _, Context, Entity, Window};
use impulse_kit::layout_tree::SplitAxis;
use impulse_protocol::control::ControlResponse;

use crate::control_server::{descends, Incoming};
use crate::terminal_view::TerminalView;
use crate::workbench::Workbench;
use crate::workspace::Surface;

/// Hand a request to the window holding its pane (else the first window).
pub fn dispatch(incoming: Incoming, cx: &mut App) {
    let windows = crate::session::workbenches(cx);
    let token = incoming.request.token.clone().unwrap_or_default();
    let target = windows
        .iter()
        .find(|(w, _)| !token.is_empty() && w.read(cx).terminal_with_token(&token, cx).is_some())
        .or(windows.first())
        .cloned();
    let Some((workbench, handle)) = target else {
        let _ = incoming
            .reply
            .try_send(ControlResponse::failure("Impulse isn't ready yet."));
        return;
    };
    let _ = handle.update(cx, |_, window, cx| {
        workbench.update(cx, |workbench, cx| {
            workbench.handle_control(incoming, window, cx)
        })
    });
}

fn inside_terminal() -> ControlResponse {
    ControlResponse::failure("Run this inside an Impulse terminal.")
}

impl Workbench {
    /// The terminal with a pane token, if it's in this window.
    pub(crate) fn terminal_with_token(
        &self,
        token: &str,
        cx: &App,
    ) -> Option<Entity<TerminalView>> {
        self.workspaces
            .iter()
            .flat_map(|w| w.all_surfaces())
            .filter_map(Surface::terminal)
            .find(|t| t.read(cx).token == token)
            .cloned()
            .or_else(|| crate::quick_terminal::terminal_with_token(token, cx))
    }

    pub(crate) fn handle_control(
        &mut self,
        incoming: Incoming,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Incoming {
            request,
            caller,
            reply,
        } = incoming;
        let answer = {
            let reply = reply.clone();
            move |response: ControlResponse| {
                let _ = reply.try_send(response);
            }
        };
        let terminal = request
            .token
            .as_deref()
            .and_then(|token| self.terminal_with_token(token, cx));
        let from_shell = terminal
            .as_ref()
            .is_some_and(|t| descends(caller, t.read(cx).shell_pid()));
        // Where things run: the caller's folder only when it's the pane's
        // own shell (or started from it), else the pane's.
        let pane_cwd = terminal.as_ref().map(|t| t.read(cx).cwd.clone());
        let cwd = if from_shell {
            request.cwd.clone().or(pane_cwd)
        } else {
            pane_cwd
        };
        let args = &request.arguments;
        let needs_shell = matches!(request.command.as_str(), "split" | "tab" | "checkpoint");
        if needs_shell && !from_shell {
            if terminal.is_none() {
                return answer(inside_terminal());
            }
            return answer(ControlResponse::failure(format!(
                "impulse {} only works from the terminal's own shell, not inside tmux or screen.",
                request.command
            )));
        }
        // Hooks, statuses, notifications, turns, blocks and Review
        // (`agent_hooks`).
        let agent_answer = std::rc::Rc::new(answer.clone());
        if self.handle_agent_control(
            &request,
            terminal.as_ref(),
            from_shell,
            agent_answer,
            window,
            cx,
        ) {
            return;
        }
        match request.command.as_str() {
            "open" => {
                let Some(path) = args.get("path").cloned() else {
                    return answer(ControlResponse::failure("No file given."));
                };
                // Here, /dev/fd/5 would be one of Impulse's own files.
                if path.starts_with("/dev/") {
                    return answer(ControlResponse::failure(format!(
                        "{path} isn't a regular file."
                    )));
                }
                let path = PathBuf::from(&path);
                if !path.exists() {
                    if !request.wait {
                        return answer(ControlResponse::failure(format!(
                            "No such file: {}",
                            path.display()
                        )));
                    }
                    if !from_shell {
                        return answer(ControlResponse::failure(format!(
                            "No such file: {} (only the terminal's own shell can create one)",
                            path.display()
                        )));
                    }
                    if std::fs::write(&path, "").is_err() {
                        return answer(ControlResponse::failure(format!(
                            "Couldn't create {}.",
                            path.display()
                        )));
                    }
                }
                if path.is_dir() {
                    self.open_workspace(&path, window, cx);
                    return answer(ControlResponse::ok());
                }
                if !path.is_file() {
                    return answer(ControlResponse::failure(format!(
                        "{} isn't a regular file.",
                        path.display()
                    )));
                }
                if args.get("background").is_none() {
                    window.activate_window();
                }
                self.open_file(&path, window, cx);
                if let (Some(line), Some(editor)) = (
                    args.get("line").and_then(|l| l.parse::<u32>().ok()),
                    self.active_editor().cloned(),
                ) {
                    let column = args
                        .get("column")
                        .and_then(|c| c.parse::<u32>().ok())
                        .unwrap_or(1);
                    let position =
                        lsp_types::Position::new(line.saturating_sub(1), column.saturating_sub(1));
                    editor.update(cx, |editor, cx| editor.go_to(position, window, cx));
                }
                if request.wait {
                    // `impulse edit`: answer when the file's tab closes.
                    match self.find_editor(&path, cx) {
                        Some((w, t, pane)) => {
                            let editor = self.workspaces[w].tabs[t].panes[&pane].clone();
                            self.pending_edits
                                .push((crate::workbench::surface_id(&editor), reply));
                        }
                        None => answer(ControlResponse::failure(format!(
                            "Impulse couldn't open {} in the editor.",
                            path.display()
                        ))),
                    }
                } else {
                    answer(ControlResponse::ok());
                }
            }
            "split" | "tab" => {
                let dir = cwd
                    .map(PathBuf::from)
                    .filter(|d| d.is_dir())
                    .unwrap_or_else(crate::workspace::home);
                let commands: Vec<String> = args.get("command").cloned().into_iter().collect();
                let surface = self.new_terminal(dir, commands, window, cx);
                if request.command == "tab" {
                    let active = self.active;
                    self.add_tab(active, surface, window, cx);
                } else {
                    if let Some(terminal) = terminal.as_ref() {
                        self.reveal_surface(&Surface::Terminal(terminal.clone()), cx);
                    }
                    let axis = if args.get("direction").map(String::as_str) == Some("down") {
                        SplitAxis::Vertical
                    } else {
                        SplitAxis::Horizontal
                    };
                    self.split_focused(surface, axis, window, cx);
                }
                answer(ControlResponse::ok());
            }
            "checkpoint" => {
                let Some(cwd) = cwd else {
                    return answer(inside_terminal());
                };
                let reason = args
                    .get("message")
                    .filter(|m| !m.is_empty())
                    .cloned()
                    .unwrap_or_else(|| "manual checkpoint".into());
                cx.background_spawn(async move {
                    let Some(root) = impulse_git::branches::repo_root(&cwd) else {
                        return answer(ControlResponse::failure("Not in a git repository."));
                    };
                    let prefix = format!("{}manual/", impulse_git::safety::CHECKPOINT_PREFIX);
                    match impulse_git::safety::create(&reason, &root, &prefix) {
                        Ok(snapshot) => {
                            answer(ControlResponse::ok_message(snapshot.ref_name));
                            crate::agent_checkpoints::prune(&root);
                        }
                        Err(error) => answer(ControlResponse::failure(error.message())),
                    }
                })
                .detach();
            }
            "unsaved" => {
                if terminal.is_none() {
                    return answer(inside_terminal());
                }
                let files = self
                    .workspace()
                    .all_surfaces()
                    .filter_map(Surface::editor)
                    .filter(|e| e.read(cx).dirty)
                    .map(|e| impulse_protocol::agent_tools::UnsavedFile {
                        path: e.read(cx).path.to_string_lossy().into_owned(),
                        changed_on_disk: false,
                    })
                    .collect();
                let list = impulse_protocol::agent_tools::UnsavedList::new(files);
                let json = args.contains_key("json");
                answer(ControlResponse::answer(&list, list.text(), json));
            }
            "review" | "comments" | "problems" => {
                self.handle_review_control(&request, terminal, cwd, from_shell, reply, window, cx)
            }
            "tasks" | "project" => answer(ControlResponse::failure(format!(
                "impulse {} isn't in the Rust app yet.",
                request.command
            ))),
            other => answer(ControlResponse::failure(format!(
                "impulse: unknown command '{other}'"
            ))),
        }
    }

    /// Show the tab holding `surface` and focus its pane.
    pub(crate) fn reveal_surface(&mut self, surface: &Surface, cx: &mut Context<Self>) {
        let id = crate::workbench::surface_id(surface);
        for (w, workspace) in self.workspaces.iter_mut().enumerate() {
            for (t, tab) in workspace.tabs.iter_mut().enumerate() {
                if let Some(pane) = tab.pane_of(|s| crate::workbench::surface_id(s) == id) {
                    tab.focused = pane;
                    workspace.active = t;
                    self.active = w;
                    cx.notify();
                    return;
                }
            }
        }
    }

    /// An editor surface closed: answer the `impulse edit` waiting on it.
    pub(crate) fn editor_closed(&mut self, surface: &Surface) {
        let id = crate::workbench::surface_id(surface);
        self.pending_edits.retain(|(editor, reply)| {
            if *editor == id {
                let _ = reply.try_send(ControlResponse::ok());
                false
            } else {
                true
            }
        });
    }
}
