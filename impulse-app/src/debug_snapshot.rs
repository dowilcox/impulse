//! Headless snapshots for the smoke checks and the docs screenshots (the
//! Swift `DebugSnapshot` and `MainWindowController+Debug`), in debug builds
//! only: `impulse-app --impulse-snapshot <dir>` opens the window from
//! `--impulse-snapshot-session` (or the default one), runs the actions one
//! after another, writes every window as `<dir>/window-<n>.png` with
//! `snapshot.log` beside them, and quits, without restoring or saving the
//! session or settings (`--impulse-snapshot-persist` saves them as usual).
//!
//!     impulse-app --impulse-snapshot /tmp/shots [--impulse-snapshot-delay 4]
//!         [--impulse-snapshot-cwd ~/Code/impulse]
//!         [--impulse-snapshot-actions sidebar,palette]
//!         [--impulse-snapshot-actions-file actions.json]
//!         [--impulse-snapshot-session saved-session.json]
//!         [--impulse-snapshot-size 1280x800] [--impulse-snapshot-no-lsp]
//!         [--impulse-snapshot-persist]
//!
//! Actions and `until=` conditions are the Swift app's, as far as the Rust
//! app has the features; one it can't do is reported as a failure in the
//! dump. Window images: on Linux (X11, Xvfb included) ImageMagick's
//! `import -window` reads each window from the X server; on macOS
//! `screencapture -l` takes it by window number, the windows kept
//! transparent and letting clicks through until then.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use gpui_kit::component::WindowExt as _;
use gpui_kit::{App, AsyncApp, Context, Entity, Window};
use serde_json::{json, Map, Value};

use crate::workbench::Workbench;
use crate::workspace::{Kind, Surface};

/// The snapshot's arguments.
pub struct Config {
    pub output: PathBuf,
    pub delay: f64,
    /// Where relative paths in actions start (else the first workspace).
    pub cwd: Option<String>,
    pub actions: Vec<String>,
    pub session: Option<PathBuf>,
    pub size: (f32, f32),
    pub no_lsp: bool,
    pub persists: bool,
}

static CONFIG: OnceLock<Option<Config>> = OnceLock::new();

/// The snapshot run, when the app was started for one.
pub fn config() -> Option<&'static Config> {
    CONFIG.get().and_then(Option::as_ref)
}

pub fn is_active() -> bool {
    config().is_some()
}

fn expand(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => crate::workspace::home()
            .join(rest)
            .to_string_lossy()
            .into_owned(),
        None => path.to_string(),
    }
}

/// Read the `--impulse-snapshot*` arguments, before anything else starts:
/// a run that doesn't persist turns persistence off for the whole app.
pub fn configure() {
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| {
        let index = args.iter().position(|a| a == flag)?;
        args.get(index + 1).cloned()
    };
    let config = value("--impulse-snapshot").map(|dir| {
        let mut actions: Vec<String> = value("--impulse-snapshot-actions")
            .map(|raw| raw.split(',').map(|a| a.trim().to_string()).collect())
            .unwrap_or_default();
        if let Some(list) = value("--impulse-snapshot-actions-file")
            .and_then(|path| std::fs::read(expand(&path)).ok())
            .and_then(|data| serde_json::from_slice::<Vec<String>>(&data).ok())
        {
            actions = list;
        }
        let size = value("--impulse-snapshot-size")
            .and_then(|raw| {
                let (w, h) = raw.split_once('x')?;
                Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?))
            })
            .filter(|(w, h)| *w >= 400. && *h >= 300.)
            .unwrap_or((1440., 900.));
        Config {
            output: PathBuf::from(expand(&dir)),
            delay: value("--impulse-snapshot-delay")
                .and_then(|d| d.parse().ok())
                .unwrap_or(4.),
            cwd: value("--impulse-snapshot-cwd").map(|c| expand(&c)),
            actions,
            session: value("--impulse-snapshot-session").map(|p| PathBuf::from(expand(&p))),
            size,
            no_lsp: args.iter().any(|a| a == "--impulse-snapshot-no-lsp"),
            persists: args.iter().any(|a| a == "--impulse-snapshot-persist"),
        }
    });
    if config.as_ref().is_some_and(|c| !c.persists) {
        std::env::set_var("IMPULSE_NO_PERSISTENCE", "1");
    }
    let _ = CONFIG.set(config);
}

/// The window to open: the session file's, read only.
pub fn session(cx: &App) -> Option<impulse_kit::session::SessionState> {
    let path = config()?.session.as_ref()?;
    let home = crate::workspace::home().to_string_lossy().into_owned();
    let mut state = impulse_kit::session::SessionState::load(path, &home)?;
    if crate::settings_store::SettingsStore::get(cx).restore_scrollback {
        let dir = impulse_kit::session::scrollback_dir(cfg!(debug_assertions));
        for window in &mut state.windows {
            if let Some(workspaces) = window.workspaces.as_mut() {
                impulse_kit::session::load_scrollback(workspaces, &dir);
            }
        }
    }
    (!state.windows.is_empty()).then_some(state)
}

// MARK: What the dumps report

/// Something to run with the window: a toast's action, a dialog's button.
pub(crate) type Press = Rc<dyn Fn(&mut Window, &mut App)>;

struct ToastRecord {
    kind: String,
    message: String,
    detail: Option<String>,
    action: Option<(String, Press)>,
}

/// A dialog's texts and buttons, as it said when it opened.
struct Sheet {
    texts: Vec<String>,
    buttons: Vec<(String, Press)>,
}

#[derive(Default)]
struct State {
    toasts: Vec<ToastRecord>,
    results: Map<String, Value>,
    failures: Vec<String>,
    sheet: Option<Sheet>,
    task_sheet: Option<gpui_kit::WeakEntity<crate::task_sheet::TaskSheet>>,
    agent_sheet: Option<gpui_kit::WeakEntity<crate::agent_integration::AgentIntegration>>,
    /// The action running now (for the log).
    current: String,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    /// Where each window's open dialog, palette or popover painted in its
    /// last frame (`overlay_probe`, cleared by `frame_start`).
    static OVERLAY: RefCell<HashMap<gpui_kit::WindowId, gpui_kit::Bounds<gpui_kit::Pixels>>> =
        RefCell::new(HashMap::new());
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

/// A toast (or a status bar notice) was shown.
pub(crate) fn note_toast(
    kind: &str,
    message: &str,
    detail: Option<&str>,
    action: Option<(&str, Press)>,
) {
    if !is_active() {
        return;
    }
    with_state(|s| {
        s.toasts.push(ToastRecord {
            kind: kind.into(),
            message: message.into(),
            detail: detail.map(Into::into),
            action: action.map(|(label, press)| (label.to_string(), press)),
        })
    });
}

/// A dialog opened, with these texts and buttons.
pub(crate) fn note_sheet(texts: Vec<String>, buttons: Vec<(&str, Press)>) {
    if !is_active() {
        return;
    }
    let buttons = buttons
        .into_iter()
        .map(|(label, press)| (label.to_string(), press))
        .collect();
    with_state(|s| s.sheet = Some(Sheet { texts, buttons }));
}

pub(crate) fn note_task_sheet(sheet: &Entity<crate::task_sheet::TaskSheet>) {
    if is_active() {
        with_state(|s| s.task_sheet = Some(sheet.downgrade()));
    }
}

pub(crate) fn note_agent_sheet(sheet: &Entity<crate::agent_integration::AgentIntegration>) {
    if is_active() {
        with_state(|s| s.agent_sheet = Some(sheet.downgrade()));
    }
}

/// Something to report under `label` in the dumps' results.
pub(crate) fn note_result(label: &str, value: String) {
    if is_active() {
        report(label, Value::String(value));
    }
}

fn fail(message: String) {
    with_state(|s| {
        log::warn!("snapshot, during “{}”: {message}", s.current);
        s.failures.push(message)
    });
}

fn report(label: &str, value: Value) {
    with_state(|s| s.results.insert(label.to_string(), value));
}

// MARK: The run

/// The first window's workbench, with its window.
fn with_workbench<R>(
    cx: &mut App,
    f: impl FnOnce(&mut Workbench, &mut Window, &mut Context<Workbench>) -> R,
) -> Option<R> {
    let (workbench, handle) = crate::session::workbenches(cx).into_iter().next()?;
    handle
        .update(cx, |_, window, cx| {
            workbench.update(cx, |workbench, cx| f(workbench, window, cx))
        })
        .ok()
}

/// Called once the windows are open: size them, run the actions, capture,
/// quit. Each action starts 0.6 s after the one before it finished, so its
/// animations settle; the capture follows the last one by delay/2.
pub fn run(cx: &mut App) {
    let Some(config) = config() else {
        return;
    };
    let _ = std::fs::create_dir_all(&config.output);
    with_workbench(cx, |_, window, cx| {
        // GPUI under Xvfb paints its first frame once the window changes.
        window.resize(gpui_kit::size(
            gpui_kit::px(config.size.0),
            gpui_kit::px(config.size.1),
        ));
        hide(window);
        cx.notify();
    });
    cx.spawn(async move |cx| {
        sleep(cx, config.delay / 2.).await;
        for action in &config.actions {
            with_state(|s| s.current = action.chars().take(60).collect());
            perform(action, cx).await;
            sleep(cx, 0.6).await;
        }
        sleep(cx, config.delay / 2.).await;
        cx.update(capture);
        cx.update(|cx| cx.quit());
    })
    .detach();
}

async fn sleep(cx: &mut AsyncApp, seconds: f64) {
    cx.background_executor()
        .timer(Duration::from_secs_f64(seconds.max(0.)))
        .await;
}

/// Actions that take time; the rest are `perform_now`.
///
///     sleep=<seconds>
///     until=[<seconds>:]<condition>   wait until it holds (30 s by default)
///     task=<title>                    New Task with that title, created
///     agent-sheet-apply               Agent Integration's Install/Remove
async fn perform(action: &str, cx: &mut AsyncApp) {
    if let Some(seconds) = action.strip_prefix("sleep=") {
        return sleep(cx, seconds.parse().unwrap_or(0.)).await;
    }
    if let Some(spec) = action.strip_prefix("until=") {
        let (timeout, condition) = match spec.split_once(':') {
            Some((t, rest)) if t.parse::<f64>().is_ok() => (t.parse().unwrap_or(30.), rest),
            _ => (30., spec),
        };
        let deadline = Instant::now() + Duration::from_secs_f64(timeout);
        loop {
            let holds = cx.update(|cx| holds(condition, cx));
            if holds {
                return;
            }
            if Instant::now() > deadline {
                return fail(format!("until {condition}: timed out"));
            }
            sleep(cx, 0.25).await;
        }
    }
    // `lsp=label:kind:line:character[:new name]` asks the selected editor's
    // server (EditorTab::debug_lsp) and reports the answer; `lsp-wait=` asks
    // until there's one (the server is still loading the project).
    for (prefix, wait) in [("lsp=", false), ("lsp-wait=", true)] {
        let Some(spec) = action.strip_prefix(prefix) else {
            continue;
        };
        let parts: Vec<&str> = spec.splitn(5, ':').collect();
        let (label, kind) = (
            parts.first().copied().unwrap_or(""),
            parts.get(1).copied().unwrap_or(""),
        );
        let number = |i: usize| {
            parts
                .get(i)
                .and_then(|n| n.parse::<u32>().ok())
                .unwrap_or(0)
        };
        let (line, character, new_name) =
            (number(2), number(3), parts.get(4).map(|n| n.to_string()));
        let deadline = Instant::now() + Duration::from_secs(if wait { 120 } else { 20 });
        loop {
            let task = cx.update(|cx| {
                with_workbench(cx, |workbench, window, cx| {
                    let editor = workbench.active_editor().cloned()?;
                    editor.update(cx, |e, cx| {
                        e.debug_lsp(kind, line, character, new_name.clone(), window, cx)
                    })
                })
                .flatten()
            });
            let answer = match task {
                Some(task) => task.await,
                None => Value::Null,
            };
            if !wait || !answer.is_null() || Instant::now() > deadline {
                if wait && answer.is_null() {
                    fail(format!("lsp-wait={label}: no answer"));
                }
                report(label, answer);
                return;
            }
            sleep(cx, 1.0).await;
        }
    }
    if let Some(title) = action.strip_prefix("task=") {
        let title = title.to_string();
        cx.update(|cx| present_task_sheet(&title, None, None, cx));
        // Created once the sheet has looked up the branches and copies.
        let path = cx.update(|cx| {
            let sheet = with_state(|s| s.task_sheet.clone()).and_then(|s| s.upgrade())?;
            Some(PathBuf::from(sheet.read(cx).path(cx)))
        });
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut created = false;
        while !created && Instant::now() < deadline {
            created = cx.update(task_sheet_create);
            if !created {
                sleep(cx, 0.25).await;
            }
        }
        if !created {
            return fail(format!(
                "task={title}: the New Task sheet couldn't create it"
            ));
        }
        // What the Swift app's `task=` starts the task with, once its
        // terminal is at a prompt.
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            let started = cx.update(|cx| {
                with_workbench(cx, |workbench, _, cx| {
                    let terminal = workbench
                        .workspaces
                        .iter()
                        .find(|w| Some(&w.root) == path.as_ref())
                        .and_then(|w| w.active_tab())
                        .and_then(|t| t.focused_surface().terminal().cloned())
                        .filter(|t| t.read(cx).tracks_prompts && !t.read(cx).command_running);
                    terminal.map(|t| t.update(cx, |t, _| t.run_command("echo task ready")))
                })
                .flatten()
                .is_some()
            });
            if started {
                return;
            }
            sleep(cx, 0.25).await;
        }
        return fail(format!(
            "task={title}: the task's terminal never reached a prompt"
        ));
    }
    if let Some(spec) = action.strip_prefix("task-sheet=") {
        let mut parts = spec.split('|');
        let title = parts.next().unwrap_or_default().to_string();
        let command = parts.next().map(str::to_string);
        let base = parts.next().map(str::to_string);
        return cx.update(|cx| present_task_sheet(&title, command.as_deref(), base.as_deref(), cx));
    }
    if action == "task-sheet-create" {
        if !cx.update(task_sheet_create) {
            fail("task-sheet-create: no New Task sheet that can create is up".into());
        }
        return;
    }
    if action == "agent-sheet-apply" {
        let Some(sheet) = with_state(|s| s.agent_sheet.clone()).and_then(|s| s.upgrade()) else {
            return fail("agent-sheet-apply: no Agent Integration sheet is up".into());
        };
        with_state(|s| s.results.remove("agent-sheet"));
        cx.update(|cx| {
            with_window(cx, |window, cx| {
                sheet.update(cx, |sheet, cx| sheet.debug_apply(window, cx))
            })
        });
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            if with_state(|s| s.results.contains_key("agent-sheet")) {
                return;
            }
            sleep(cx, 0.25).await;
        }
        return fail("agent-sheet-apply: no outcome".into());
    }
    cx.update(|cx| perform_now(action, cx));
}

/// A path from an action: absolute, under ~ (the demo home), or relative to
/// the starting folder.
fn debug_path(path: &str, workbench: &Workbench) -> PathBuf {
    let expanded = expand(path);
    if expanded.starts_with('/') {
        return PathBuf::from(expanded);
    }
    config()
        .and_then(|c| c.cwd.clone())
        .map(PathBuf::from)
        .unwrap_or_else(|| first_root(workbench))
        .join(expanded)
}

/// The first folder workspace's root (the Swift app's file tree root).
fn first_root(workbench: &Workbench) -> PathBuf {
    workbench
        .workspaces
        .iter()
        .find(|w| w.kind == Kind::Folder)
        .map(|w| w.root.clone())
        .unwrap_or_else(|| workbench.workspace().root.clone())
}

/// Every tab of every workspace, in order: (workspace, tab).
fn all_tabs(workbench: &Workbench) -> Vec<(usize, usize)> {
    workbench
        .workspaces
        .iter()
        .enumerate()
        .flat_map(|(w, ws)| (0..ws.tabs.len()).map(move |t| (w, t)))
        .collect()
}

fn select_tab(
    workbench: &mut Workbench,
    (w, t): (usize, usize),
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    workbench.activate_workspace(w, window, cx);
    workbench.activate_tab(t, window, cx);
}

/// The commands the Swift app's named actions run.
fn command_for(action: &str) -> Option<&'static str> {
    Some(match action {
        "review" => "review_changes",
        "changes" => "show_changes",
        "search" => "project_search",
        "history" => "git_history",
        "review-agent" => "review_agent_turn",
        "manage-branches" => "manage_branches",
        "keybindings" => "open_keybindings",
        "settings" => "open_settings",
        "quick-terminal" => "quick_terminal",
        "preview-beside" => "preview_beside",
        "diff-view" => "diff_view",
        "close-pane" => "close_tab",
        "reopen" => "reopen_tab",
        "newtab" | "new-terminal" => "new_tab",
        "close-workspace" => "close_workspace",
        "finish-task" => "finish_task",
        "trust-prompt" => "trust_folder",
        "problems" => "show_problems",
        _ => return None,
    })
}

fn perform_now(action: &str, cx: &mut App) {
    let (name, argument) = match action.split_once('=') {
        Some((name, argument)) => (name, Some(argument)),
        None => (action, None),
    };
    let unknown = || fail(format!("unknown action “{action}”"));
    // A click goes to the window, outside the workbench's update (its
    // handlers update the workbench themselves).
    if let ("click", Some(at)) = (name, argument) {
        let point = at
            .split_once(':')
            .and_then(|(x, y)| Some((x.parse::<f32>().ok()?, y.parse::<f32>().ok()?)));
        let Some((x, y)) = point else {
            return unknown();
        };
        if let Some((_, handle)) = crate::session::workbenches(cx).into_iter().next() {
            let _ = handle.update(cx, |_, window, cx| click(window, x, y, cx));
        }
        return;
    }
    let done = with_workbench(cx, |workbench, window, cx| {
        if let Some(command) = command_for(action) {
            workbench.run_command(command, window, cx);
            return None;
        }
        // Review's own actions, on every Review open.
        if let Some(review_action) = action.strip_prefix("review-") {
            let reviews: Vec<_> = workbench
                .workspaces
                .iter()
                .flat_map(|w| &w.tabs)
                .flat_map(|t| t.surfaces())
                .filter_map(|(_, s)| match s {
                    Surface::Review(r) => Some(r.clone()),
                    _ => None,
                })
                .collect();
            if reviews.is_empty() {
                fail(format!("{action}: no Review is open"));
            }
            for review in reviews {
                if let Err(error) =
                    review.update(cx, |r, cx| r.debug_action(review_action, window, cx))
                {
                    fail(error);
                }
            }
            return None;
        }
        match (name, argument) {
            ("wait", None) => {}
            ("sidebar" | "no-sidebar", None) => {
                crate::settings_store::SettingsStore::update(cx, |s| {
                    s.sidebar_visible = name == "sidebar"
                })
            }
            ("palette", None) => workbench.show_palette(">", window, cx),
            ("quickopen", None) => workbench.show_palette("", window, cx),
            ("quickopen-query", None) => workbench.show_palette("wbv", window, cx),
            ("branches", None) => workbench.show_palette("b:", window, cx),
            ("palette", Some(prefix)) => workbench.show_palette(prefix, window, cx),
            ("command", Some(id)) => workbench.run_command(id, window, cx),
            ("pane", Some(id)) => workbench.run_command(id, window, cx),
            ("open", Some(path)) => {
                let path = debug_path(path, workbench);
                workbench.open_file(&path, window, cx)
            }
            ("workspace", Some(folder)) => {
                let folder = debug_path(folder, workbench);
                workbench.open_workspace(&folder, window, cx)
            }
            ("select-workspace", Some(wanted)) => {
                match workbench.workspaces.iter().position(|w| w.name() == wanted) {
                    Some(index) => workbench.activate_workspace(index, window, cx),
                    None => fail(format!("select-workspace: no workspace “{wanted}”")),
                }
            }
            ("select-tab", Some(index)) => {
                let tabs = all_tabs(workbench);
                if let Some(at) = index.parse::<usize>().ok().and_then(|i| tabs.get(i)) {
                    select_tab(workbench, *at, window, cx);
                }
            }
            ("select-terminal", None) => {
                let found = all_tabs(workbench).into_iter().rev().find(|(w, t)| {
                    workbench.workspaces[*w].tabs[*t]
                        .surfaces()
                        .iter()
                        .any(|(_, s)| s.terminal().is_some())
                });
                if let Some(at) = found {
                    select_tab(workbench, at, window, cx);
                }
            }
            ("select-editor", Some(suffix)) => {
                let found = all_tabs(workbench).into_iter().find(|(w, t)| {
                    workbench.workspaces[*w].tabs[*t]
                        .surfaces()
                        .iter()
                        .any(|(_, s)| {
                            s.editor().is_some_and(|e| {
                                e.read(cx).path.to_string_lossy().ends_with(suffix)
                            })
                        })
                });
                if let Some(at) = found {
                    select_tab(workbench, at, window, cx);
                }
            }
            ("tree-expand", Some(relative)) => {
                // A folder in the file tree, by root-relative path.
                let show_hidden = crate::settings_store::SettingsStore::get(cx).sidebar_show_hidden;
                let workspace = workbench.workspace_mut();
                let path = workspace.root.join(relative);
                if !workspace.expanded_dirs.contains(&path) {
                    let entries = crate::workspace::list_dir(&path, show_hidden);
                    workspace.children.insert(path.clone(), entries);
                    workspace.expanded_dirs.insert(path);
                    // Remembered, as a click would (Swift: expandDirectory).
                    workspace.save_expanded();
                }
                cx.notify();
            }
            ("settings", Some(query)) => workbench.open_settings(Some(query), window, cx),
            ("preview", Some(path)) => {
                // What a single click in the file tree does.
                let path = workbench.workspace().root.join(path);
                workbench.open_file_as(&path, true, window, cx)
            }
            ("workspace-edit", Some(other)) => {
                // An edit to the open file plus one on disk, as a rename makes.
                let Some(editor) = workbench.active_editor().cloned() else {
                    fail(format!("{action}: no editor is selected"));
                    return None;
                };
                let own = editor.read(cx).path.clone();
                let root = workbench.workspace().root.clone();
                let other = root.join(other);
                let edit = |path: &std::path::Path, text: &str| {
                    Some(impulse_lsp::workspace_edit::Operation::Edit {
                        uri: impulse_lsp::path_uri(path)?,
                        edits: vec![lsp_types::TextEdit {
                            range: lsp_types::Range::default(),
                            new_text: text.to_string(),
                        }],
                    })
                };
                let operations = [edit(&own, "// edited\n"), edit(&other, "edited ")]
                    .into_iter()
                    .flatten()
                    .collect();
                workbench.apply_workspace_edit(
                    crate::workspace_edits::PendingEdit {
                        edit: impulse_lsp::workspace_edit::WorkspaceEdit {
                            operations,
                            versions: Default::default(),
                        },
                        utf32: false,
                        origin: own,
                        server_root: root,
                        title: "Rename".into(),
                    },
                    window,
                    cx,
                );
            }
            ("search", Some(text)) => {
                workbench.run_command("project_search", window, cx);
                let search = workbench.search.clone();
                search.update(cx, |s, cx| s.search_for(text, window, cx));
            }
            ("gallery", options) => {
                // gallery, gallery=dark, gallery=light+contrast …
                let options = options.unwrap_or("");
                let light = if options.contains("light") {
                    Some(true)
                } else if options.contains("dark") {
                    Some(false)
                } else {
                    None
                };
                crate::component_gallery::show(light, options.contains("contrast"), cx);
            }
            ("history-all", None) => {
                // Every History open shows all branches.
                let histories: Vec<_> = workbench
                    .workspaces
                    .iter()
                    .flat_map(|w| w.all_surfaces())
                    .filter_map(|s| match s {
                        Surface::History(h) => Some(h.clone()),
                        _ => None,
                    })
                    .collect();
                for history in histories {
                    history.update(cx, |h, cx| h.set_scope(impulse_git::log::LogScope::All, cx));
                }
            }
            ("history-filter", Some(text)) => {
                workbench.run_command("git_history", window, cx);
                if let Some(Surface::History(history)) = workbench.focused_surface().cloned() {
                    history.update(cx, |h, cx| h.set_filter(text, window, cx));
                }
            }
            ("history", Some(sha)) => {
                workbench.run_command("git_history", window, cx);
                if let Some(Surface::History(history)) = workbench.focused_surface().cloned() {
                    history.update(cx, |h, cx| h.reveal(sha.to_string(), cx));
                }
            }
            ("expand-workspaces", None) => {
                for workspace in &mut workbench.workspaces {
                    workspace.expanded = true;
                }
                cx.notify();
            }
            ("run", Some(command)) => match workbench.active_terminal().cloned() {
                Some(terminal) => terminal.update(cx, |t, _| t.run_command(command)),
                None => fail(format!("run: no terminal is selected ({command})")),
            },
            ("run-quoted", Some(spec)) => {
                let (command, path) = spec.split_once('|').unwrap_or((spec, ""));
                match workbench.active_terminal().cloned() {
                    Some(terminal) => terminal.update(cx, |t, _| {
                        let quoted = impulse_kit::shell::quoting::quoted(path, t.shell);
                        t.run_command(&format!("{command} {quoted}"))
                    }),
                    None => fail(format!("run-quoted: no terminal is selected ({command})")),
                }
            }
            ("draft", Some(text)) => {
                if let Some(terminal) = workbench.active_terminal().cloned() {
                    let text = text.replace("\\n", "\n");
                    terminal.update(cx, |t, cx| {
                        t.editor.update(cx, |e, cx| e.set_text(&text, cx));
                        // As typing it would: the ghost suggestion follows.
                        t.bar_text_changed(cx);
                    });
                }
            }
            ("problems", Some(file)) => {
                // A JSON list of {path (root-relative), line, column,
                // severity, message, source, code?}.
                use impulse_kit::problems::{Problem, Severity};
                let list: Vec<Value> = std::fs::read_to_string(file)
                    .ok()
                    .and_then(|t| serde_json::from_str(&t).ok())
                    .unwrap_or_default();
                let root = first_root(workbench);
                let mut by_path: std::collections::BTreeMap<String, Vec<Problem>> =
                    Default::default();
                for item in list {
                    let (Some(relative), Some(message)) =
                        (item["path"].as_str(), item["message"].as_str())
                    else {
                        continue;
                    };
                    let path = root.join(relative).to_string_lossy().into_owned();
                    let severity = match item["severity"].as_str() {
                        Some("error") => Severity::Error,
                        Some("warning") => Severity::Warning,
                        Some("hint") => Severity::Hint,
                        _ => Severity::Info,
                    };
                    by_path.entry(path.clone()).or_default().push(Problem {
                        path,
                        line: item["line"].as_u64().unwrap_or(1) as usize,
                        column: item["column"].as_u64().unwrap_or(1) as usize,
                        severity,
                        message: message.to_string(),
                        source: item["source"].as_str().map(str::to_string),
                        code: item["code"].as_str().map(str::to_string),
                    });
                }
                let store = workbench.problems.clone();
                store.update(cx, |store, cx| {
                    for (path, list) in by_path {
                        store.set(&path, "snapshot", list);
                    }
                    cx.notify();
                });
                workbench.run_command("show_problems", window, cx);
            }
            ("focus-pane", Some(n)) => {
                // The nth pane of the active tab, in layout order (the
                // session's pane ids).
                let pane = n.parse::<usize>().ok().and_then(|n| {
                    let tab = workbench.workspace().active_tab()?;
                    tab.layout.leaves().get(n).copied()
                });
                match pane {
                    Some(pane) => {
                        if let Some(tab) = workbench.workspace_mut().active_tab_mut() {
                            tab.focused = pane;
                            tab.zoomed = None;
                        }
                        workbench.focus_active(window, cx);
                        cx.notify();
                    }
                    None => fail(format!("focus-pane: no pane {n}")),
                }
            }
            ("select-blocks", Some(n)) => {
                if let Some(terminal) = workbench.active_terminal().cloned() {
                    let count = n.parse().unwrap_or(1);
                    terminal.update(cx, |t, cx| t.select_last_blocks(count, window, cx));
                }
            }
            ("find-word", None) => {
                if let Some(terminal) = workbench.active_terminal().cloned() {
                    terminal.update(cx, |t, cx| t.find_whole_word(window, cx));
                }
            }
            ("settings-category", Some(name)) => {
                workbench.open_settings(None, window, cx);
                let category = impulse_kit::settings::Category::ALL
                    .into_iter()
                    .find(|c| c.title() == name);
                match (category, workbench.focused_surface().cloned()) {
                    (Some(category), Some(Surface::Settings(view))) => {
                        view.update(cx, |v, cx| v.select_category(category, window, cx))
                    }
                    _ => fail(format!("settings-category: no category “{name}”")),
                }
            }
            ("replace", Some(spec)) => {
                let (query, replacement) = spec.split_once(':').unwrap_or((spec, ""));
                workbench.run_command("project_search", window, cx);
                let search = workbench.search.clone();
                search.update(cx, |s, cx| s.replace_for(query, replacement, window, cx));
            }
            ("git-peek", Some(line)) => {
                let editor = workbench.active_editor().cloned();
                let peeked = match (editor, line.parse::<usize>()) {
                    (Some(editor), Ok(line)) => {
                        editor.update(cx, |e, cx| e.peek_change_at(line, cx))
                    }
                    _ => false,
                };
                if !peeked {
                    fail(format!("git-peek: no change on line {line}"));
                }
            }
            ("beside", Some(path)) => {
                let path = debug_path(path, workbench);
                workbench.open_beside(&path, window, cx)
            }
            ("tag-sheet", None) => workbench.run_command("git_create_tag", window, cx),
            ("tag" | "tag-push", Some(spec)) => {
                // Tag HEAD without the prompt (name[:message]).
                let (tag, message) = match spec.split_once(':') {
                    Some((tag, message)) => (tag, Some(message.to_string())),
                    None => (spec, None),
                };
                match workbench.git.active.clone() {
                    Some(repo) => workbench.git_create_tag(
                        &repo,
                        tag.to_string(),
                        "HEAD".into(),
                        message,
                        name == "tag-push",
                        false,
                        window,
                        cx,
                    ),
                    None => fail(format!("{name}: not in a git repository")),
                }
            }
            ("project-trust", None) => {
                let root = workbench.workspace().root.to_string_lossy().into_owned();
                workbench.trust_project_config(&root, window, cx, Box::new(|_, _, _, _| {}));
            }
            ("project-setup", None) => workbench.open_project_setup(None, None, window, cx),
            ("move-sheet", None) => workbench.present_new_task(None, true, window, cx),
            ("archive-merged", None) => {
                let index = workbench.active;
                workbench.present_archive_merged_tasks(index, window, cx)
            }
            ("composer", Some(text)) => {
                // The agent composer, opened with a draft.
                workbench.run_command("agent_composer", window, cx);
                if let Some(terminal) = workbench.active_terminal().cloned() {
                    let composer = terminal.read(cx).agent.composer.clone();
                    match composer {
                        Some(composer) => {
                            composer.update(cx, |c, cx| c.set_text(text.to_string(), cx))
                        }
                        None => fail(format!("composer: no agent composer opened ({text})")),
                    }
                }
            }
            ("find", Some(text)) => {
                if let Some(terminal) = workbench.active_terminal().cloned() {
                    terminal.update(cx, |t, cx| t.find_text(text, window, cx));
                }
            }
            ("complete", None) => {
                // Tab in the input bar: the completion menu.
                if let Some(terminal) = workbench.active_terminal().cloned() {
                    terminal.update(cx, |t, cx| t.request_completions(cx));
                }
            }
            ("block", Some(id)) => {
                if let Some(terminal) = workbench.active_terminal().cloned() {
                    terminal.update(cx, |t, cx| t.perform_block_command(id, window, cx));
                }
            }
            ("save", None) => match workbench.active_editor().cloned() {
                Some(editor) => {
                    let name = editor
                        .read(cx)
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "untitled".into());
                    // The workbench's save: a file changed on disk asks.
                    workbench.save_editor(editor.clone(), window, cx);
                    let saved = !editor.read(cx).dirty;
                    report(&format!("save:{name}"), Value::Bool(saved));
                }
                None => fail("save: no editor is selected".into()),
            },
            ("write-file", Some(spec)) => {
                let (path, text) = spec.split_once('|').unwrap_or((spec, ""));
                let path = debug_path(path, workbench);
                // In place, as `>` does (the same inode).
                if let Err(error) = std::fs::write(&path, text) {
                    fail(format!("write-file {}: {error}", path.display()));
                }
            }
            ("read-file", Some(spec)) => {
                let (label, path) = spec.split_once('|').unwrap_or((spec, ""));
                let path = debug_path(path, workbench);
                let text = std::fs::read_to_string(path).map_or(Value::Null, Value::String);
                report(label, text);
            }
            ("theme", Some(theme)) => {
                let theme = theme.to_string();
                crate::settings_store::SettingsStore::update(cx, |s| s.color_scheme = theme)
            }
            ("setting-on", Some(key)) => set_setting(key, Value::Bool(true), cx),
            ("setting", Some(spec)) => {
                let (key, value) = spec.split_once('=').unwrap_or((spec, ""));
                set_setting(key, Value::String(value.into()), cx)
            }
            ("trust-on", None) => {
                workbench.trust.set_enabled(true);
                crate::settings_store::SettingsStore::update(cx, |s| s.ask_to_trust_folders = true);
                cx.notify();
            }
            ("trust-project-settings", None) => {
                let root = workbench.workspace().root.to_string_lossy().into_owned();
                if let Some(loaded) = crate::tasks::load_project_config(&root) {
                    let mut trust = crate::tasks::project_trust();
                    for source in loaded.sources() {
                        trust.trust(
                            &crate::tasks::trust_key(&source.path, &loaded, &root),
                            &source.digest,
                        );
                    }
                    crate::tasks::save_project_trust(&trust);
                }
            }
            ("finish-run", landing) => workbench.debug_finish(landing, window, cx),
            ("agent-sheet", Some(spec)) => {
                let mut parts = spec.split(':');
                let part = parts.next().unwrap_or_default();
                let agent = parts.next().unwrap_or_default().to_string();
                let project = parts.next() == Some("project");
                let part = if part == "mcp" {
                    crate::agent_integration::Part::Mcp
                } else {
                    crate::agent_integration::Part::Hooks
                };
                with_state(|s| s.agent_sheet = None);
                workbench.present_agent_integration(part, window, cx);
                let sheet = with_state(|s| s.agent_sheet.clone()).and_then(|s| s.upgrade());
                match sheet {
                    Some(sheet) => sheet.update(cx, |sheet, cx| {
                        sheet.debug_choose(None, Some(&agent), project, cx)
                    }),
                    None => fail(format!("agent-sheet={spec}: the sheet didn't open")),
                }
            }
            ("agent-sheet-part" | "agent-sheet-agent", Some(value)) => {
                match with_state(|s| s.agent_sheet.clone()).and_then(|s| s.upgrade()) {
                    Some(sheet) => sheet.update(cx, |sheet, cx| {
                        let (part, agent) = if name == "agent-sheet-part" {
                            (Some(value), None)
                        } else {
                            (None, Some(value))
                        };
                        sheet.debug_choose(part, agent, false, cx)
                    }),
                    None => fail(format!("{action}: no Agent Integration sheet is up")),
                }
            }
            ("agent-sheet-close", None) => window.close_dialog(cx),
            ("task-sheet-cancel", None) => window.close_dialog(cx),
            ("sheet-button", Some(title)) => {
                let press = with_state(|s| {
                    s.sheet.as_ref().and_then(|sheet| {
                        sheet
                            .buttons
                            .iter()
                            .find(|(label, _)| label == title)
                            .map(|(_, press)| press.clone())
                    })
                });
                match press.filter(|_| window.has_active_dialog(cx)) {
                    Some(press) => {
                        with_state(|s| s.sheet = None);
                        return Some(press);
                    }
                    None => fail(format!("sheet-button: no “{title}” button")),
                }
            }
            ("toast-action", Some(text)) => {
                let press = with_state(|s| {
                    s.toasts
                        .iter()
                        .rev()
                        .find(|t| t.message.contains(text) && t.action.is_some())
                        .and_then(|t| t.action.as_ref().map(|(_, press)| press.clone()))
                });
                match press {
                    Some(press) => return Some(press),
                    None => fail(format!(
                        "toast-action: no toast with “{text}” and an action"
                    )),
                }
            }
            ("save-session", None) => {
                return Some(Rc::new(|_: &mut Window, cx: &mut App| {
                    crate::session::save(cx)
                }));
            }
            ("dump", Some(name)) => dump(name, workbench, window, cx),
            ("js", Some(spec)) => {
                let label = spec.split_once(':').map_or(spec, |(label, _)| label);
                report(label, Value::Null);
                fail(format!(
                    "js={label}: the Rust app's editor has no web page to run it in"
                ));
            }
            // The Rust app's stand-in for `js=`: `edit=label:script`,
            // EditorTab::debug_edit's commands on the selected editor.
            ("edit", Some(spec)) => {
                let (label, script) = spec.split_once(':').unwrap_or((spec, ""));
                match workbench.active_editor().cloned() {
                    Some(editor) => {
                        let text = editor.update(cx, |e, cx| e.debug_edit(script, window, cx));
                        report(label, Value::String(text));
                    }
                    None => fail(format!("edit={label}: no editor is selected")),
                }
            }
            ("key", Some(key)) => {
                if let Ok(keystroke) = gpui_kit::Keystroke::parse(key) {
                    window.dispatch_keystroke(keystroke, cx);
                }
            }
            _ => unknown(),
        }
        None
    });
    match done {
        // Toasts' and dialogs' buttons update the workbench themselves.
        Some(Some(press)) => {
            with_window(cx, |window, cx| press(window, cx));
        }
        Some(None) => {}
        None => fail(format!("{action}: no window")),
    }
}

fn with_window(cx: &mut App, f: impl FnOnce(&mut Window, &mut App)) {
    if let Some((_, handle)) = crate::session::workbenches(cx).into_iter().next() {
        let _ = handle.update(cx, |_, window, cx| f(window, cx));
    }
}

/// A left click at x:y points from the window's top-left.
fn click(window: &mut Window, x: f32, y: f32, cx: &mut App) {
    use gpui_kit::{
        point, px, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput,
    };
    let position = point(px(x), px(y));
    window.dispatch_event(
        PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
            position,
            modifiers: Modifiers::default(),
            click_count: 1,
        }),
        cx,
    );
}

/// A setting by its `settings.json` key, in memory only (snapshot runs
/// save no settings unless they persist).
fn set_setting(key: &str, value: Value, cx: &mut App) {
    let mut changed = false;
    crate::settings_store::SettingsStore::update(cx, |s| {
        let current = s.get(key);
        // Numbers and booleans from text, as the setting's type wants.
        let value = match (&current, &value) {
            (Some(Value::Number(_)), Value::String(text)) => {
                serde_json::from_str(text).unwrap_or(value.clone())
            }
            (Some(Value::Bool(_)), Value::String(text)) => Value::Bool(text == "true"),
            _ => value.clone(),
        };
        changed = s.set(key, &value);
    });
    if !changed {
        fail(format!(
            "setting {key}: no such setting, or the value doesn't fit"
        ));
    }
}

fn present_task_sheet(title: &str, command: Option<&str>, base: Option<&str>, cx: &mut App) {
    with_state(|s| s.task_sheet = None);
    with_workbench(cx, |workbench, window, cx| {
        workbench.present_new_task(None, false, window, cx)
    });
    let sheet = with_state(|s| s.task_sheet.clone()).and_then(|s| s.upgrade());
    let Some(sheet) = sheet else {
        return fail(format!("task-sheet={title}: no New Task sheet opened"));
    };
    let (title, command, base) = (
        title.to_string(),
        command.map(str::to_string),
        base.map(str::to_string),
    );
    with_workbench(cx, |_, window, cx| {
        sheet.update(cx, |sheet, cx| {
            sheet.debug_fill(&title, command.as_deref(), base.as_deref(), window, cx)
        })
    });
}

/// Press the New Task sheet's Create once it can; false while it can't.
fn task_sheet_create(cx: &mut App) -> bool {
    let Some(sheet) = with_state(|s| s.task_sheet.clone()).and_then(|s| s.upgrade()) else {
        return false;
    };
    if !sheet.read(cx).can_create(cx) {
        return false;
    }
    with_workbench(cx, |workbench, window, cx| {
        workbench.create_task(sheet.clone(), window, cx)
    });
    true
}

// MARK: Conditions

/// Conditions for `until=`:
///
///     prompt              the selected terminal's shell is at a prompt
///     blocks:<n>          the selected terminal has n finished command blocks
///     editor              an editor is selected (its file is loaded)
///     file:<path>, no-file:<path>
///     sheet, no-sheet     a dialog is (not) showing
///     agent:<state>       the selected terminal's agent is in that state
///     toast:<text>        a toast with that text was shown
///     result:<label>      something was reported under label
///     tabs:<n>            at least n tabs
///     panes:<n>           at least n panes in the selected tab
///     tool:<title>        a tool tab whose title contains title is open
///     text:<text>         the selected editor holds text (`\n` escaped)
///     has:<text>          the selected editor's text contains text
fn holds(condition: &str, cx: &mut App) -> bool {
    let (kind, argument) = condition.split_once(':').unwrap_or((condition, ""));
    let count = argument.parse::<usize>().unwrap_or(1);
    let answer = with_workbench(cx, |workbench, window, cx| {
        let terminal = workbench.active_terminal().cloned();
        match kind {
            "prompt" => terminal.is_some_and(|t| {
                let t = t.read(cx);
                t.tracks_prompts && !t.command_running
            }),
            "blocks" => terminal.is_some_and(|t| {
                let blocks = t.read(cx).backend.command_blocks();
                blocks.iter().filter(|b| b.ended_at_ms.is_some()).count() >= count
            }),
            "editor" => workbench.active_editor().is_some(),
            "js" => {
                fail(format!(
                    "until js:{argument}: the Rust app's editor has no web page"
                ));
                true
            }
            "text" => workbench
                .active_editor()
                .is_some_and(|e| e.read(cx).text(cx) == argument.replace("\\n", "\n")),
            "has" => workbench
                .active_editor()
                .is_some_and(|e| e.read(cx).text(cx).contains(argument)),
            "file" => debug_path(argument, workbench).exists(),
            "no-file" => !debug_path(argument, workbench).exists(),
            "sheet" => window.has_active_dialog(cx),
            "no-sheet" => !window.has_active_dialog(cx),
            "agent" => {
                terminal.is_some_and(|t| t.read(cx).agent.state().map(state_name) == Some(argument))
            }
            "toast" => with_state(|s| {
                s.toasts.iter().any(|t| {
                    t.message.contains(argument)
                        || t.detail.as_deref().is_some_and(|d| d.contains(argument))
                })
            }),
            "result" => with_state(|s| s.results.contains_key(argument)),
            "tabs" => all_tabs(workbench).len() >= count,
            "panes" => workbench
                .workspace()
                .active_tab()
                .is_some_and(|t| t.surfaces().len() >= count),
            "tool" => workbench
                .workspaces
                .iter()
                .flat_map(|w| w.all_surfaces())
                .any(|s| is_tool(s) && s.name(cx).contains(argument)),
            _ => {
                fail(format!("until: unknown condition “{condition}”"));
                true
            }
        }
    });
    answer.unwrap_or(false)
}

fn is_tool(surface: &Surface) -> bool {
    matches!(
        surface,
        Surface::Settings(_)
            | Surface::Keybindings(_)
            | Surface::TaskFinish(_)
            | Surface::Problems(_)
    )
}

/// The Swift `AgentState` raw values.
fn state_name(state: impulse_kit::agents::state_machine::AgentState) -> &'static str {
    use impulse_kit::agents::state_machine::AgentState::*;
    match state {
        Idle => "idle",
        Working => "working",
        NeedsInput => "needsInput",
        Done => "done",
        Exited => "exited",
    }
}

// MARK: Dumps

/// Writes `<name>.json`: workspaces, tabs and what each pane holds
/// (terminals with their blocks and output, editors with their text),
/// toasts, sheets, results and failures, in the Swift app's format.
fn dump(name: &str, workbench: &mut Workbench, window: &mut Window, cx: &mut Context<Workbench>) {
    let Some(config) = config() else {
        return;
    };
    let mut tabs = Vec::new();
    for (w, workspace) in workbench.workspaces.iter().enumerate() {
        for (t, tab) in workspace.tabs.iter().enumerate() {
            let order = tab.layout.leaves();
            let surfaces: Vec<Value> = order
                .iter()
                .filter_map(|id| tab.panes.get(id))
                .map(|s| describe(s, cx))
                .collect();
            tabs.push(json!({
                "workspace": workspace.name(),
                "selected": w == workbench.active && t == workspace.active,
                "title": tab.focused_surface().title(cx),
                "surfaces": surfaces,
            }));
        }
    }
    let workspaces: Vec<Value> = workbench
        .workspaces
        .iter()
        .enumerate()
        .map(|(index, w)| {
            json!({
                "name": w.name(),
                "root": w.root.to_string_lossy(),
                "kind": match w.kind { Kind::Folder => "folder", Kind::Scratch => "scratch" },
                "task": w.task.is_task,
                "active": index == workbench.active,
                "trusted": workbench.trust.is_trusted(&w.root.to_string_lossy()),
            })
        })
        .collect();
    let dialog = window.has_active_dialog(cx);
    let (toasts, sheets, results, failures, task_sheet, agent_sheet) = with_state(|s| {
        if !dialog {
            s.sheet = None;
        }
        let toasts: Vec<Value> = s
            .toasts
            .iter()
            .map(|t| {
                json!({
                    "kind": t.kind, "message": t.message, "detail": t.detail,
                    "action": t.action.as_ref().map(|(label, _)| label.clone()),
                })
            })
            .collect();
        let sheets: Vec<Value> = if dialog {
            vec![match &s.sheet {
                Some(sheet) => json!({
                    "texts": sheet.texts,
                    "buttons": sheet.buttons.iter().map(|(l, _)| l.clone()).collect::<Vec<_>>(),
                }),
                None => json!({"texts": [], "buttons": []}),
            }]
        } else {
            Vec::new()
        };
        (
            toasts,
            sheets,
            Value::Object(s.results.clone()),
            s.failures.clone(),
            s.task_sheet.clone(),
            s.agent_sheet.clone(),
        )
    });
    let task_sheet = task_sheet
        .and_then(|s| s.upgrade())
        .filter(|_| dialog)
        .map_or(Value::Null, |s| s.read(cx).debug_state(cx));
    let agent_sheet = agent_sheet
        .and_then(|s| s.upgrade())
        .filter(|_| dialog)
        .map_or(Value::Null, |s| s.read(cx).debug_state());
    let state = json!({
        "workspaces": workspaces,
        "trustEnabled": workbench.trust.is_enabled(),
        "tabs": tabs,
        "toasts": toasts,
        "sheets": sheets,
        "agentSheet": agent_sheet,
        "taskSheet": task_sheet,
        "results": results,
        "failures": failures,
    });
    let text = serde_json::to_string_pretty(&state).unwrap_or_default();
    if let Err(error) = std::fs::write(config.output.join(format!("{name}.json")), text) {
        log::error!("snapshot: couldn't write {name}.json: {error}");
    }
}

fn describe(surface: &Surface, cx: &App) -> Value {
    match surface {
        Surface::Terminal(t) => {
            let t = t.read(cx);
            let blocks: Vec<Value> = t
                .backend
                .command_blocks()
                .into_iter()
                .map(|b| {
                    json!({
                        "command": b.command, "cwd": b.cwd, "exit": b.exit_code,
                        "finished": b.ended_at_ms.is_some(),
                    })
                })
                .collect();
            let tracker = &t.agent;
            json!({
                "kind": "terminal",
                "shell": t.shell.name(),
                "cwd": t.cwd,
                "running": t.command_running,
                "agent": tracker.agent.as_ref().map(|a| a.id.clone()),
                "agentState": tracker.state().map(state_name),
                "resume": tracker.session.as_ref().and_then(|(agent, session)| {
                    impulse_kit::agents::known_agents::resume_command(agent, session)
                }),
                "blocks": blocks,
                "transcript": t.backend.transcript(400, false),
            })
        }
        Surface::Editor(e) => {
            let e = e.read(cx);
            // One buffer: Impulse's copy is the editor's own text, with the
            // file's line ending as Monaco's getValue gives it.
            let text = e.disk_text(cx);
            json!({
                "kind": "editor", "path": e.path.to_string_lossy(), "modified": e.dirty,
                "language": e.language, "largeFile": false, "swift": text, "monaco": text,
            })
        }
        Surface::Image(i) => json!({"kind": "image", "path": i.read(cx).path.to_string_lossy()}),
        Surface::Review(r) => json!({"kind": "review", "root": r.read(cx).root}),
        Surface::History(h) => json!({"kind": "history", "root": h.read(cx).root}),
        other => json!({"kind": "tool", "title": other.name(cx)}),
    }
}

// MARK: Window images

/// The windows' native handles, for hiding and capturing them.
fn native(window: &Window) -> Option<raw_window_handle::RawWindowHandle> {
    raw_window_handle::HasWindowHandle::window_handle(window)
        .ok()
        .map(|h| h.as_raw())
}

/// Nothing shows on screen and no click lands in the window (macOS: fully
/// transparent until the capture). An X server for snapshots is Xvfb, where
/// nobody sees the window.
fn hide(window: &Window) {
    #[cfg(target_os = "macos")]
    if let Some(raw_window_handle::RawWindowHandle::AppKit(handle)) = native(window) {
        mac::set_visible(handle.ns_view.as_ptr(), false);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}

/// Records where it's painted: put in a dialog's body (`overlays`) or the
/// palette, it tells `capture` where the overlay is for the docs shots'
/// crop (Swift's are windows of their own; GPUI Kit's are drawn inside).
pub(crate) fn overlay_probe() -> impl gpui_kit::IntoElement {
    use gpui_kit::Styled as _;
    gpui_kit::canvas(
        |bounds, window, _| {
            let id = window.window_handle().window_id();
            OVERLAY.with(|o| o.borrow_mut().insert(id, bounds));
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// Painted first in a window's frame: forgets the overlay its last frame
/// drew, so only one painted in this frame (after it) is reported.
pub(crate) fn frame_start() -> impl gpui_kit::IntoElement {
    use gpui_kit::Styled as _;
    gpui_kit::canvas(
        |_, window, _| {
            let id = window.window_handle().window_id();
            OVERLAY.with(|o| o.borrow_mut().remove(&id));
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_0()
}

/// Writes `window-<n>.png` for every open window and `snapshot.log`.
fn capture(cx: &mut App) {
    let Some(config) = config() else {
        return;
    };
    let mut written = Vec::new();
    let mut frames = String::new();
    let mut notes = String::new();
    let main: Vec<_> = crate::session::workbenches(cx)
        .iter()
        .map(|(_, handle)| handle.window_id())
        .collect();
    for (index, handle) in cx.windows().into_iter().enumerate() {
        let name = format!("window-{index}.png");
        let path = config.output.join(&name);
        let outcome = handle.update(cx, |_, window, cx| {
            window.refresh();
            let bounds = window.bounds();
            let screen = window
                .display(cx)
                .map(|d| d.bounds().size.height)
                .unwrap_or(bounds.size.height);
            let (x, y) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
            let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            // The scripts read screen coordinates y-up, as AppKit has them.
            let y_up = f32::from(screen) - y - h;
            let class = if main.contains(&handle.window_id()) {
                "MainWindow"
            } else {
                "Window"
            };
            let mut frame = format!(
                "frame window-{index} x={x} y={y_up} w={w} h={h} level=0 sheet=false class={class}\n"
            );
            // An open dialog, palette or popover, inside its frame, from the
            // window's top left.
            if let Some(o) = OVERLAY.with(|o| o.borrow().get(&handle.window_id()).copied()) {
                frame.push_str(&format!(
                    "overlay window-{index} x={} y={} w={} h={}\n",
                    f32::from(o.origin.x),
                    f32::from(o.origin.y),
                    f32::from(o.size.width),
                    f32::from(o.size.height)
                ));
            }
            (frame, write_image(window, &path))
        });
        match outcome {
            Ok((frame, Ok(()))) => {
                frames.push_str(&frame);
                written.push(name);
            }
            Ok((frame, Err(error))) => {
                frames.push_str(&frame);
                notes.push_str(&format!("window-{index}: {error}\n"));
            }
            Err(error) => notes.push_str(&format!("window-{index}: {error}\n")),
        }
    }
    let report = format!("captured: {}\n{notes}{frames}", written.join(", "));
    let _ = std::fs::write(config.output.join("snapshot.log"), report);
}

fn write_image(window: &Window, path: &Path) -> Result<(), String> {
    let run = |program: &str, args: &[String]| -> Result<(), String> {
        let output = std::process::Command::new(program)
            .args(args)
            .output()
            .map_err(|e| format!("{program}: {e}"))?;
        if output.status.success() && path.exists() {
            Ok(())
        } else {
            Err(format!(
                "{program} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    };
    let path_arg = path.to_string_lossy().into_owned();
    match native(window) {
        Some(raw_window_handle::RawWindowHandle::Xcb(handle)) => run(
            "import",
            &[
                "-window".into(),
                handle.window.get().to_string(),
                format!("png:{path_arg}"),
            ],
        ),
        Some(raw_window_handle::RawWindowHandle::Xlib(handle)) => run(
            "import",
            &[
                "-window".into(),
                handle.window.to_string(),
                format!("png:{path_arg}"),
            ],
        ),
        #[cfg(target_os = "macos")]
        Some(raw_window_handle::RawWindowHandle::AppKit(handle)) => {
            let view = handle.ns_view.as_ptr();
            let number = mac::window_number(view).ok_or("no window number")?;
            mac::set_visible(view, true);
            let result = run(
                "/usr/sbin/screencapture",
                &["-x".into(), "-o".into(), format!("-l{number}"), path_arg],
            );
            mac::set_visible(view, false);
            result
        }
        Some(_) => Err("window images need X11 or macOS (Wayland can't be read)".into()),
        None => Err("no native window".into()),
    }
}

#[cfg(target_os = "macos")]
// objc's `msg_send!` asks for a `cargo-clippy` feature it doesn't declare.
#[allow(unexpected_cfgs)]
mod mac {
    use objc::runtime::{Object, NO, YES};
    use objc::{msg_send, sel, sel_impl};

    fn window_of(view: *mut std::ffi::c_void) -> *mut Object {
        let view = view as *mut Object;
        unsafe { msg_send![view, window] }
    }

    /// The `CGWindowID` screencapture takes.
    pub fn window_number(view: *mut std::ffi::c_void) -> Option<isize> {
        let window = window_of(view);
        if window.is_null() {
            return None;
        }
        let number: isize = unsafe { msg_send![window, windowNumber] };
        Some(number)
    }

    /// Opaque (for the capture) or transparent and letting clicks through.
    pub fn set_visible(view: *mut std::ffi::c_void, visible: bool) {
        let window = window_of(view);
        if window.is_null() {
            return;
        }
        let alpha: f64 = if visible { 1.0 } else { 0.0 };
        unsafe {
            let _: () = msg_send![window, setAlphaValue: alpha];
            let _: () = msg_send![window, setIgnoresMouseEvents: if visible { NO } else { YES }];
        }
    }
}
