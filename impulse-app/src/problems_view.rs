//! The Problems tab and the status bar's counts (the Swift
//! `ProblemsSurface` and `WindowModel.problemsByPath`): every diagnostic
//! the window's language servers report, open files or not (servers like
//! rust-analyzer report the whole workspace), by file with files holding
//! errors first. Filter by severity or text, click a problem to open its
//! file there, copy them all as a prompt for an agent.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    div, prelude::*, px, AnyElement, App, ClipboardItem, Context, Entity, EventEmitter,
    FocusHandle, FontWeight, SharedString, Subscription, Window,
};
use impulse_kit::chrome::{metrics, ChromePalette, Rgba};
use impulse_kit::problems::{self, Problem, ProblemCounts, Severity};

use crate::workbench::Workbench;
use crate::workbench_chrome::{abbreviate_home, hsla, icon};
use crate::workspace::Surface;

const MONO_FONT: &str = "JetBrains Mono";

/// The window's diagnostics: by file, then by the server that sent them,
/// each server's last report replacing its previous one.
#[derive(Default)]
pub struct ProblemStore {
    by_path: HashMap<String, BTreeMap<String, Vec<Problem>>>,
    /// Kept with each change: the status bar reads them every frame.
    counts: ProblemCounts,
}

impl ProblemStore {
    /// One server's problems for `path`; false when nothing changed.
    pub fn set(&mut self, path: &str, server: &str, list: Vec<Problem>) -> bool {
        let servers = self.by_path.entry(path.to_string()).or_default();
        let changed = if list.is_empty() {
            servers.remove(server).is_some()
        } else if servers.get(server) != Some(&list) {
            servers.insert(server.to_string(), list);
            true
        } else {
            false
        };
        if servers.is_empty() {
            self.by_path.remove(path);
        }
        if changed {
            let mut counts = ProblemCounts::default();
            for problem in self.by_path.values().flat_map(|s| s.values().flatten()) {
                match problem.severity {
                    Severity::Error => counts.errors += 1,
                    Severity::Warning => counts.warnings += 1,
                    Severity::Info | Severity::Hint => counts.others += 1,
                }
            }
            self.counts = counts;
        }
        changed
    }

    pub fn all(&self) -> Vec<Problem> {
        self.by_path
            .values()
            .flat_map(|servers| servers.values().flatten().cloned())
            .collect()
    }

    pub fn counts(&self) -> ProblemCounts {
        self.counts
    }

    /// The files any server reported on.
    pub fn paths(&self) -> impl Iterator<Item = &String> {
        self.by_path.keys()
    }
}

/// A server's diagnostics for `path` as problems (1-based positions, as
/// the server counts columns). One without a severity reads as an error,
/// as LSP leaves that to the client.
pub fn problems_from_diagnostics(
    path: &str,
    diagnostics: &[lsp_types::Diagnostic],
) -> Vec<Problem> {
    use lsp_types::{DiagnosticSeverity as S, NumberOrString};
    diagnostics
        .iter()
        .map(|d| Problem {
            path: path.to_string(),
            line: d.range.start.line as usize + 1,
            column: d.range.start.character as usize + 1,
            severity: match d.severity {
                Some(S::WARNING) => Severity::Warning,
                Some(S::INFORMATION) => Severity::Info,
                Some(S::HINT) => Severity::Hint,
                _ => Severity::Error,
            },
            message: d.message.clone(),
            source: d.source.clone(),
            code: d.code.as_ref().map(|code| match code {
                NumberOrString::Number(n) => n.to_string(),
                NumberOrString::String(s) => s.clone(),
            }),
        })
        .collect()
}

/// `path` under `root` without it; elsewhere, with the home folder as `~`.
fn relative(path: &str, root: &str) -> String {
    if path == root {
        return String::new();
    }
    match path.strip_prefix(root).and_then(|r| r.strip_prefix('/')) {
        Some(rest) if !root.is_empty() => rest.to_string(),
        _ => abbreviate_home(path),
    }
}

/// At most two of a message's lines, as the Swift row's `lineLimit(2)`.
fn two_lines(message: &str) -> String {
    let lines: Vec<&str> = message.lines().collect();
    if lines.len() <= 2 {
        return message.to_string();
    }
    format!("{}\n{}…", lines[0], lines[1])
}

pub enum ProblemsEvent {
    Open(Problem),
}

pub struct ProblemsView {
    pub(crate) palette: ChromePalette,
    pub(crate) focus: FocusHandle,
    store: Entity<ProblemStore>,
    /// The workspace folder paths are shown relative to.
    root: String,
    severities: HashSet<Severity>,
    filter: Entity<InputState>,
    collapsed: HashSet<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ProblemsEvent> for ProblemsView {}

impl ProblemsView {
    pub fn new(
        store: Entity<ProblemStore>,
        root: String,
        palette: ChromePalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Filter")
                .clean_on_escape()
        });
        let subscriptions = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.subscribe(&filter, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        ProblemsView {
            palette,
            focus: cx.focus_handle(),
            store,
            root,
            severities: Severity::ALL.into_iter().collect(),
            filter,
            collapsed: HashSet::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    fn visible(&self, all: &[Problem], cx: &App) -> Vec<Problem> {
        problems::filter(all, &self.severities, &self.filter.read(cx).value())
    }

    /// Error, warning, and info (which takes hints along) on or off.
    fn toggle(&mut self, severity: Severity, cx: &mut Context<Self>) {
        let on = self.severities.contains(&severity);
        let mut set = vec![severity];
        if severity == Severity::Info {
            set.push(Severity::Hint);
        }
        for severity in set {
            if on {
                self.severities.remove(&severity);
            } else {
                self.severities.insert(severity);
            }
        }
        cx.notify();
    }

    fn severity_toggle(
        &self,
        severity: Severity,
        count: usize,
        name: &'static str,
        color: Rgba,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = &self.palette;
        let on = self.severities.contains(&severity);
        let help: SharedString =
            format!("{} {}s", if on { "Hide" } else { "Show" }, severity.label()).into();
        div()
            .id(severity.label())
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(6.))
            .h(px(22.))
            .rounded(px(4.))
            .cursor_pointer()
            .when(on, |d| d.bg(hsla(p.raised)))
            .on_click(cx.listener(move |view, _, _, cx| view.toggle(severity, cx)))
            .tooltip(move |window, cx| Tooltip::new(help.clone()).build(window, cx))
            .child(icon(name, 11., if on { color } else { p.text_tertiary }))
            .child(
                div()
                    .font_family(MONO_FONT)
                    .text_size(px(11.))
                    .text_color(hsla(if on { p.text } else { p.text_tertiary }))
                    .child(count.to_string()),
            )
    }

    fn header(
        &self,
        visible: &[Problem],
        counts: ProblemCounts,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = &self.palette;
        let prompt = problems::prompt(visible, Some(&self.root), 100);
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(44.))
            .px(px(14.))
            .bg(hsla(p.panel))
            .border_b_1()
            .border_color(hsla(p.hairline))
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(hsla(p.text))
                    .child("Problems"),
            )
            .child(self.severity_toggle(Severity::Error, counts.errors, "circle-x", p.danger, cx))
            .child(self.severity_toggle(
                Severity::Warning,
                counts.warnings,
                "triangle-alert",
                p.warning,
                cx,
            ))
            .child(self.severity_toggle(Severity::Info, counts.others, "info", p.info, cx))
            .child(div().flex_1())
            .child(
                // Swift's plain field with its icon in a 26-high `raised`
                // capsule, 160 for the text.
                crate::workbench_chrome::chrome_text_field(
                    &self.filter,
                    "list-filter",
                    p,
                    window,
                    cx,
                )
                .h(px(26.))
                .w(px(194.))
                .rounded(px(metrics::RADIUS))
                .bg(hsla(p.raised))
                .border_0(),
            )
            .child(
                Button::new("fix-with-agent")
                    .ghost()
                    .compact()
                    .icon(gpui_kit::component::Icon::empty().path("icons/bot.svg"))
                    .label("Fix with Agent")
                    .tooltip("Send to an agent or copy as a prompt")
                    .dropdown_menu(move |menu: PopupMenu, window, cx| {
                        let prompt = prompt.clone();
                        let empty = prompt.is_empty();
                        let sent = prompt.clone();
                        let menu = crate::agent_send::agent_menu_items(
                            menu,
                            "Send to",
                            !empty,
                            std::rc::Rc::new(move |_, _| Some(sent.clone())),
                            None,
                            window,
                            cx,
                        );
                        menu.item(
                            PopupMenuItem::new("Copy as Prompt")
                                .disabled(empty)
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(prompt.clone()))
                                }),
                        )
                    }),
            )
    }

    fn file_header(&self, path: &str, count: usize, cx: &mut Context<Self>) -> AnyElement {
        let p = &self.palette;
        let collapsed = self.collapsed.contains(path);
        let file = Path::new(path);
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let folder = relative(
            &file
                .parent()
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_default(),
            &self.root,
        );
        let key = path.to_string();
        div()
            .id(SharedString::from(format!("problems-file:{path}")))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .h(px(26.))
            .px(px(14.))
            .cursor_pointer()
            .on_click(cx.listener(move |view, _, _, cx| {
                if !view.collapsed.remove(&key) {
                    view.collapsed.insert(key.clone());
                }
                cx.notify();
            }))
            .child(icon(
                if collapsed {
                    "chevron-right"
                } else {
                    "chevron-down"
                },
                11.,
                p.text_tertiary,
            ))
            .child(icon("file", 12., p.text_secondary))
            .child(
                div()
                    .flex_none()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(hsla(p.text))
                    .child(name),
            )
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(11.))
                    .text_color(hsla(p.text_tertiary))
                    .child(folder),
            )
            .child(
                div()
                    .flex_none()
                    .font_family(MONO_FONT)
                    .text_size(px(10.))
                    .text_color(hsla(p.text_tertiary))
                    .child(count.to_string()),
            )
            .into_any_element()
    }

    fn problem_row(&self, id: usize, problem: &Problem, cx: &mut Context<Self>) -> AnyElement {
        let p = &self.palette;
        let (name, color) = match problem.severity {
            Severity::Error => ("circle-x", p.danger),
            Severity::Warning => ("triangle-alert", p.warning),
            Severity::Info | Severity::Hint => ("info", p.info),
        };
        let source = problem.source.as_ref().map(|source| {
            [Some(source.as_str()), problem.code.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ")
        });
        let open = problem.clone();
        div()
            .id(("problem", id))
            .flex()
            .flex_none()
            .items_start()
            .gap(px(8.))
            .pl(px(44.))
            .pr(px(14.))
            .py(px(4.))
            .cursor_pointer()
            .hover(|s| s.bg(hsla(p.hover)))
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(ProblemsEvent::Open(open.clone()))))
            .child(div().pt(px(2.)).child(icon(name, 12., color)))
            .child(
                div()
                    .min_w_0()
                    .text_size(px(12.))
                    .text_color(hsla(p.text))
                    .line_clamp(2)
                    .child(two_lines(&problem.message)),
            )
            .children(source.map(|source| {
                div()
                    .flex_none()
                    .max_w(px(220.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .pt(px(1.))
                    .text_size(px(11.))
                    .text_color(hsla(p.text_tertiary))
                    .child(source)
            }))
            .child(div().flex_1().min_w(px(8.)))
            .child(
                div()
                    .flex_none()
                    .pt(px(1.))
                    .font_family(MONO_FONT)
                    .text_size(px(10.5))
                    .text_color(hsla(p.text_tertiary))
                    .child(format!("{}:{}", problem.line, problem.column)),
            )
            .into_any_element()
    }

    fn empty(&self, nothing_reported: bool) -> impl IntoElement {
        let p = &self.palette;
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .child(icon("circle-check", 22., p.success))
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(hsla(p.text_secondary))
                    .child(if nothing_reported {
                        "No problems reported."
                    } else {
                        "Nothing matches the filter."
                    }),
            )
            .when(nothing_reported, |d| {
                d.child(
                    div()
                        .text_size(px(11.5))
                        .text_color(hsla(p.text_tertiary))
                        .child("Language servers report problems for the files they've checked."),
                )
            })
    }
}

impl Render for ProblemsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let all = self.store.read(cx).all();
        let counts = problems::counts(&all);
        let visible = self.visible(&all, cx);
        let body = if visible.is_empty() {
            self.empty(all.is_empty()).into_any_element()
        } else {
            let mut list = div().flex().flex_col().py(px(6.));
            let mut id = 0;
            for (path, group) in problems::grouped(&visible) {
                list = list.child(self.file_header(&path, group.len(), cx));
                if self.collapsed.contains(&path) {
                    continue;
                }
                for problem in &group {
                    list = list.child(self.problem_row(id, problem, cx));
                    id += 1;
                }
            }
            div()
                .id("problems-list")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(list)
                .into_any_element()
        };
        div()
            .id("problems")
            .track_focus(&self.focus)
            .key_context("Problems")
            .flex()
            .flex_col()
            .size_full()
            .bg(hsla(self.palette.content))
            .child(self.header(&visible, counts, window, cx))
            .child(body)
    }
}

impl Workbench {
    /// A language server's diagnostics for one file, kept for Problems and
    /// the status bar.
    pub(crate) fn record_problems(
        &mut self,
        server: &str,
        params: &lsp_types::PublishDiagnosticsParams,
        cx: &mut Context<Self>,
    ) {
        let path = impulse_kit::uri::uri_to_file_path(params.uri.as_str());
        let list = problems_from_diagnostics(&path, &params.diagnostics);
        let changed = self.problems.update(cx, |store, cx| {
            let changed = store.set(&path, server, list);
            if changed {
                cx.notify();
            }
            changed
        });
        if changed {
            cx.notify();
        }
    }

    /// The window's language-server diagnostics as a tab (one per
    /// workspace).
    pub(crate) fn show_problems(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(Surface::Problems(view)) =
            self.reveal_tool(|s| matches!(s, Surface::Problems(_)), window, cx)
        {
            view.update(cx, |view, cx| view.focus(window, cx));
            return;
        }
        let (store, palette) = (self.problems.clone(), self.palette.clone());
        let root = self.workspace().root.to_string_lossy().into_owned();
        let view = cx.new(|cx| ProblemsView::new(store, root, palette, window, cx));
        cx.subscribe_in(
            &view,
            window,
            |workbench, _, event, window, cx| match event {
                ProblemsEvent::Open(problem) => {
                    workbench.open_file(&PathBuf::from(&problem.path), window, cx);
                    if let Some(editor) = workbench.active_editor().cloned() {
                        let position = lsp_types::Position::new(
                            problem.line.saturating_sub(1) as u32,
                            problem.column.saturating_sub(1) as u32,
                        );
                        editor.update(cx, |editor, cx| editor.go_to(position, window, cx));
                    }
                }
            },
        )
        .detach();
        let index = self.active;
        self.add_tab(index, Surface::Problems(view), window, cx);
    }

    /// The status bar's error and warning counts, when there are problems:
    /// a click shows them.
    pub(crate) fn problems_status(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let counts = self.problems.read(cx).counts();
        if counts.total() == 0 {
            return None;
        }
        let p = &self.palette;
        let help: SharedString = format!(
            "Problems: {} errors, {} warnings",
            counts.errors, counts.warnings
        )
        .into();
        let errors = counts.errors > 0;
        Some(
            div()
                .id("status-problems")
                .flex()
                .items_center()
                .gap(px(5.))
                .cursor_pointer()
                .text_color(hsla(if errors { p.danger } else { p.text_secondary }))
                .on_click(
                    cx.listener(|workbench, _, window, cx| workbench.show_problems(window, cx)),
                )
                .tooltip(move |window, cx| Tooltip::new(help.clone()).build(window, cx))
                .child(icon(
                    if errors { "circle-x" } else { "triangle-alert" },
                    metrics::ICON_SMALL,
                    if errors { p.danger } else { p.text_secondary },
                ))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .font_family(MONO_FONT)
                        .child(counts.errors.to_string())
                        .child({
                            let color = if counts.warnings > 0 {
                                p.warning
                            } else {
                                p.text_tertiary
                            };
                            div()
                                .flex()
                                .items_center()
                                .gap(px(3.))
                                .text_color(hsla(color))
                                .child(icon("triangle-alert", 10., color))
                                .child(counts.warnings.to_string())
                        }),
                )
                .into_any_element(),
        )
    }

    /// The files whose diagnostics are current: open with a server, or
    /// reported on.
    pub(crate) fn checked_files(&self, cx: &App) -> Vec<String> {
        let mut files: HashSet<String> = self.problems.read(cx).paths().cloned().collect();
        files.extend(
            self.workspaces
                .iter()
                .flat_map(|w| w.all_surfaces())
                .filter_map(Surface::editor)
                .filter(|e| !e.read(cx).server_names().is_empty())
                .map(|e| e.read(cx).path.to_string_lossy().into_owned()),
        );
        files.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range};

    fn diagnostic(line: u32, severity: Option<DiagnosticSeverity>, message: &str) -> Diagnostic {
        Diagnostic {
            range: Range::new(Position::new(line, 4), Position::new(line, 9)),
            severity,
            message: message.into(),
            ..Default::default()
        }
    }

    #[test]
    fn diagnostics_become_one_based_problems() {
        let mut with_code = diagnostic(0, Some(DiagnosticSeverity::WARNING), "unused");
        with_code.source = Some("rustc".into());
        with_code.code = Some(NumberOrString::Number(42));
        let list = problems_from_diagnostics(
            "/r/a.rs",
            &[
                with_code,
                diagnostic(9, None, "no severity"),
                diagnostic(2, Some(DiagnosticSeverity::HINT), "hint"),
            ],
        );
        assert_eq!((list[0].line, list[0].column), (1, 5));
        assert_eq!(list[0].severity, Severity::Warning);
        assert_eq!(list[0].code.as_deref(), Some("42"));
        assert_eq!(list[0].source.as_deref(), Some("rustc"));
        assert_eq!(list[1].severity, Severity::Error);
        assert_eq!(list[2].severity, Severity::Hint);
        assert!(list.iter().all(|p| p.path == "/r/a.rs"));
    }

    #[test]
    fn each_server_replaces_its_own_report() {
        let mut store = ProblemStore::default();
        let a = problems_from_diagnostics("/r/a.rs", &[diagnostic(0, None, "x")]);
        let b = problems_from_diagnostics(
            "/r/a.rs",
            &[diagnostic(1, Some(DiagnosticSeverity::WARNING), "y")],
        );
        assert!(store.set("/r/a.rs", "rust-analyzer", a.clone()));
        assert!(!store.set("/r/a.rs", "rust-analyzer", a));
        assert!(store.set("/r/a.rs", "clippy", b));
        assert_eq!(
            store.counts(),
            ProblemCounts {
                errors: 1,
                warnings: 1,
                others: 0
            }
        );
        assert!(store.set("/r/a.rs", "rust-analyzer", Vec::new()));
        assert_eq!(store.all().len(), 1);
        assert!(store.set("/r/a.rs", "clippy", Vec::new()));
        assert_eq!(store.paths().count(), 0);
        assert!(!store.set("/r/b.rs", "clippy", Vec::new()));
    }

    #[test]
    fn messages_keep_two_lines() {
        assert_eq!(two_lines("a"), "a");
        assert_eq!(two_lines("a\nb"), "a\nb");
        assert_eq!(two_lines("a\nb\nc"), "a\nb…");
    }

    #[test]
    fn folders_show_relative_to_the_workspace() {
        assert_eq!(relative("/r/src", "/r"), "src");
        assert_eq!(relative("/r", "/r"), "");
        assert_eq!(relative("/rx/src", "/r"), "/rx/src");
    }
}
