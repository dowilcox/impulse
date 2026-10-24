//! The window's chrome, laid out as `WorkbenchView` lays it out: a 40pt
//! titlebar band with the workspace and tabs, the left dock (workspaces,
//! then files), the content pane with the terminal's context chips and
//! input bar, and the 24pt status bar. Sizes come from
//! `impulse_kit::chrome::metrics`, colors from `ChromePalette`.

use std::process::Command;

use gpui::{
    div, prelude::*, px, svg, AnyElement, Context, Entity, FocusHandle, FontWeight, Hsla,
    KeyDownEvent, SharedString, Window,
};
use impulse_kit::chrome::{metrics, ChromePalette, Rgba as KitRgba};

use crate::terminal_view::{rgba, TerminalView};
use crate::Args;

const UI_FONT: &str = "Inter";

pub struct Workbench {
    palette: ChromePalette,
    terminal: Entity<TerminalView>,
    focus: FocusHandle,
    input: String,
    workspace: String,
    branch: String,
    files: Vec<(String, bool)>,
}

fn git(cwd: &str, args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

impl Workbench {
    pub fn new(args: Args, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let theme = impulse_kit::theme::builtin_theme(&args.theme)
            .unwrap_or_else(|| impulse_kit::theme::builtin_theme("nord").unwrap());
        let palette = ChromePalette::new(&theme, false);
        let terminal = cx.new(|cx| {
            TerminalView::new(
                theme.clone(),
                palette.clone(),
                args.cwd.clone(),
                args.commands.clone(),
                window,
                cx,
            )
        });
        cx.observe(&terminal, |_, _, cx| cx.notify()).detach();

        let mut files: Vec<(String, bool)> = std::fs::read_dir(&args.cwd)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|e| {
                        (
                            e.file_name().to_string_lossy().into_owned(),
                            e.path().is_dir(),
                        )
                    })
                    .filter(|(name, _)| !name.starts_with('.'))
                    .collect()
            })
            .unwrap_or_default();
        files.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
        });

        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            palette,
            terminal,
            focus,
            input: String::new(),
            workspace: std::path::Path::new(&args.cwd)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            branch: git(&args.cwd, &["rev-parse", "--abbrev-ref", "HEAD"]),
            files,
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let terminal = self.terminal.read(cx);
        match keystroke.key.as_str() {
            "enter" => {
                terminal.write(format!("{}\r", self.input).as_bytes());
                self.input.clear();
            }
            "backspace" => {
                self.input.pop();
            }
            "c" if keystroke.modifiers.control => terminal.write(b"\x03"),
            _ => {
                if let Some(text) = keystroke
                    .key_char
                    .as_ref()
                    .filter(|_| !keystroke.modifiers.control)
                {
                    self.input.push_str(text);
                }
            }
        }
        cx.notify();
    }

    fn c(&self, color: KitRgba) -> Hsla {
        rgba(color).into()
    }

    fn icon(&self, name: &str, size: f32, color: KitRgba) -> impl IntoElement {
        svg()
            .path(format!("icons/{name}.svg"))
            .size(px(size))
            .flex_none()
            .text_color(self.c(color))
    }

    fn titlebar(&self) -> impl IntoElement {
        let p = &self.palette;
        // macOS draws its traffic lights in this inset; on Linux the window
        // controls are Impulse's own (client-side decorations).
        let leading = if cfg!(target_os = "macos") {
            metrics::TRAFFIC_LIGHT_INSET
        } else {
            12.0
        };
        let tab = |icon: &str, title: &str, active: bool| {
            let mut t = div()
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(metrics::TAB_HEIGHT))
                .px(px(12.))
                .rounded(px(metrics::RADIUS))
                .text_color(self.c(if active { p.text } else { p.text_secondary }))
                .child(self.icon(
                    icon,
                    metrics::ICON,
                    if active { p.text } else { p.text_secondary },
                ))
                .child(title.to_string());
            if active {
                t = t
                    .bg(self.c(p.raised))
                    .border_b_2()
                    .border_color(self.c(p.accent));
            }
            t
        };
        div()
            .flex()
            .items_center()
            .h(px(metrics::TITLEBAR_HEIGHT))
            .pl(px(leading))
            .pr(px(10.))
            .gap(px(10.))
            .bg(self.c(p.chrome))
            .border_b_1()
            .border_color(self.c(p.hairline))
            .child(self.icon("panel-left", metrics::ICON_LARGE, p.text_secondary))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.workspace.clone()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_color(self.c(p.text_secondary))
                    .child(self.icon("git-branch", metrics::ICON_SMALL, p.text_secondary))
                    .child(self.branch.clone()),
            )
            .child(div().w(px(8.)))
            .child(tab("terminal", "bash", true))
            .child(tab("file-code", "main.rs", false))
            .child(self.icon("plus", metrics::ICON, p.text_secondary))
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .w(px(300.))
                    .h(px(26.))
                    .px(px(8.))
                    .rounded(px(metrics::RADIUS))
                    .bg(self.c(p.overlay))
                    .border_1()
                    .border_color(self.c(p.hairline))
                    .text_color(self.c(p.text_tertiary))
                    .child(self.icon("search", metrics::ICON_SMALL, p.text_tertiary))
                    .child("Search or run a command")
                    .child(div().flex_1())
                    .child(if cfg!(target_os = "macos") {
                        "⇧⌘P"
                    } else {
                        "Ctrl+Shift+P"
                    }),
            )
    }

    fn sidebar(&self) -> impl IntoElement {
        let p = &self.palette;
        let row = |icon: &str, label: String, selected: bool, indent: f32| {
            let mut r = div()
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(metrics::ROW_HEIGHT + 4.))
                .pl(px(indent))
                .pr(px(8.))
                .mx(px(6.))
                .rounded(px(metrics::RADIUS))
                .child(self.icon(icon, metrics::ICON, p.text_secondary))
                .child(label);
            if selected {
                r = r.bg(self.c(p.selection)).font_weight(FontWeight::SEMIBOLD);
            }
            r
        };
        let mut files = div().flex().flex_col().py(px(4.));
        for (name, is_dir) in self.files.iter().take(24) {
            let mut r = div()
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(metrics::ROW_HEIGHT))
                .px(px(10.))
                .child(div().w(px(12.)).child(if *is_dir {
                    self.icon("chevron-right", metrics::ICON_SMALL, p.text_tertiary)
                        .into_any_element()
                } else {
                    div().into_any_element()
                }))
                .child(self.icon(
                    if *is_dir { "folder" } else { "file" },
                    metrics::ICON,
                    if *is_dir { p.info } else { p.text_secondary },
                ))
                .child(name.clone());
            if *is_dir {
                r = r.text_color(self.c(p.text));
            }
            files = files.child(r);
        }
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(metrics::LEFT_DOCK_DEFAULT_WIDTH))
            .h_full()
            .bg(self.c(p.panel))
            .border_r_1()
            .border_color(self.c(p.hairline))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .py(px(8.))
                    .gap(px(2.))
                    .border_b_1()
                    .border_color(self.c(p.hairline))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .px(px(14.))
                            .h(px(metrics::ROW_HEIGHT))
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(self.c(p.text_secondary))
                            .child(self.icon("folder-git-2", metrics::ICON_SMALL, p.text_secondary))
                            .child(self.workspace.clone()),
                    )
                    .child(row("folder-git-2", self.workspace.clone(), true, 18.))
                    .child(row("terminal", "Scratch".into(), false, 10.)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .h(px(metrics::PANE_HEADER_HEIGHT + 6.))
                    .px(px(8.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .px(px(8.))
                            .h(px(24.))
                            .rounded(px(metrics::RADIUS))
                            .bg(self.c(p.raised))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.icon("folder-tree", metrics::ICON, p.text))
                            .child("Files"),
                    )
                    .child(self.icon("git-branch", metrics::ICON, p.text_tertiary))
                    .child(self.icon("search", metrics::ICON, p.text_tertiary)),
            )
            .child(files)
    }

    fn chip(&self, icon: &str, text: String, color: KitRgba) -> AnyElement {
        let p = &self.palette;
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(24.))
            .px(px(8.))
            .rounded(px(metrics::RADIUS))
            .bg(self.c(p.raised))
            .font_family("JetBrains Mono")
            .text_size(px(12.))
            .text_color(self.c(color))
            .child(self.icon(icon, metrics::ICON_SMALL, p.text_secondary))
            .child(text)
            .into_any_element()
    }

    fn content(&self, cx: &Context<Self>) -> impl IntoElement {
        let p = &self.palette;
        let terminal = self.terminal.read(cx);
        let home = std::env::var("HOME").unwrap_or_default();
        let cwd = match terminal.cwd.strip_prefix(&home) {
            Some(rest) if !home.is_empty() => format!("~{rest}"),
            _ => terminal.cwd.clone(),
        };
        let mut chips = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(self.chip("terminal", "bash".into(), p.text))
            .child(self.chip("folder", cwd, p.text))
            .child(self.chip("git-branch", self.branch.clone(), p.text));
        if let Some(ms) = terminal.last_duration_ms {
            chips = chips.child(self.chip("history", format!("{ms}ms"), p.success));
        }
        let input: SharedString = if self.input.is_empty() {
            "Run a command…".into()
        } else {
            self.input.clone().into()
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(self.terminal.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .px(px(12.))
                    .py(px(10.))
                    .bg(self.c(p.content))
                    .child(chips)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .h(px(36.))
                            .px(px(12.))
                            .rounded(px(metrics::RADIUS_LARGE))
                            .border_1()
                            .border_color(self.c(p.focus_ring))
                            .font_family("JetBrains Mono")
                            .text_color(self.c(if self.input.is_empty() {
                                p.text_tertiary
                            } else {
                                p.text
                            }))
                            .child(self.icon("chevron-right", metrics::ICON, p.accent))
                            .child(input),
                    ),
            )
    }

    fn status_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let p = &self.palette;
        let item = |icon: &str, text: String| {
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(self.icon(icon, metrics::ICON_SMALL, p.text_secondary))
                .child(text)
        };
        div()
            .flex()
            .items_center()
            .gap(px(16.))
            .h(px(metrics::STATUS_BAR_HEIGHT))
            .px(px(12.))
            .bg(self.c(p.chrome))
            .border_t_1()
            .border_color(self.c(p.hairline))
            .text_size(px(12.))
            .text_color(self.c(p.text_secondary))
            .child(item("git-branch", self.branch.clone()))
            .child(item("folder", self.terminal.read(cx).cwd.clone()))
            .child(div().flex_1())
            .child(item("terminal", "bash".into()))
    }
}

impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Everything but the terminal's own area, from the layout above.
        let viewport = window.viewport_size();
        let width = viewport.width - px(metrics::LEFT_DOCK_DEFAULT_WIDTH + 1.);
        let height =
            viewport.height - px(metrics::TITLEBAR_HEIGHT + metrics::STATUS_BAR_HEIGHT + 2. + 98.);
        self.terminal.update(cx, |t, _| t.fit(width, height));

        let p = &self.palette;
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .flex()
            .flex_col()
            .size_full()
            .bg(self.c(p.window))
            .font_family(UI_FONT)
            .text_size(px(13.))
            .text_color(self.c(p.text))
            .child(self.titlebar())
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.sidebar())
                    .child(self.content(cx)),
            )
            .child(self.status_bar(cx))
    }
}
