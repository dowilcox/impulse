//! The window's chrome, laid out as `WorkbenchView` lays it out: a 40pt
//! titlebar band with the workspace, its branch and its tabs; the left dock
//! (workspaces, then the file tree); and the 24pt status bar. Sizes come
//! from `impulse_kit::chrome::metrics`, colors from `ChromePalette`.

use std::path::{Path, PathBuf};

use std::rc::Rc;

use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem};
use gpui_kit::{div, prelude::*, px, svg, AnyElement, Context, FontWeight, Hsla, Window};
use impulse_kit::chrome::{metrics, Rgba as KitRgba};

use crate::settings_store::SettingsStore;
use crate::workbench::{SidebarPanel, Workbench};
use crate::workspace::{Kind, Surface, Workspace};

pub(crate) fn hsla(color: KitRgba) -> Hsla {
    crate::terminal_view::rgba(color).into()
}

pub(crate) fn icon(name: &str, size: f32, color: KitRgba) -> impl IntoElement {
    svg()
        .path(format!("icons/{name}.svg"))
        .size(px(size))
        .flex_none()
        .text_color(hsla(color))
}

/// Swift's `ProgressRing`: a faint circle and an arc for `progress`, or a
/// 28% arc turned by `turn` (0 to 1) while it's unknown.
pub(crate) fn progress_ring(
    progress: Option<f32>,
    color: KitRgba,
    size: f32,
    line_width: f32,
    turn: f32,
) -> impl IntoElement {
    use std::f32::consts::PI;
    gpui_kit::canvas(
        |_, _, _| {},
        move |bounds: gpui_kit::Bounds<gpui_kit::Pixels>, _, window, _| {
            let width = px(line_width);
            let radius = bounds.size.width / 2. - width / 2.;
            let center = bounds.center();
            let arc = |from: f32, to: f32| {
                let mut path = gpui_kit::PathBuilder::stroke(width);
                let steps = 32;
                for step in 0..=steps {
                    let angle = from + (to - from) * step as f32 / steps as f32;
                    let at = gpui_kit::point(
                        center.x + radius * angle.cos(),
                        center.y + radius * angle.sin(),
                    );
                    if step == 0 {
                        path.move_to(at);
                    } else {
                        path.line_to(at);
                    }
                }
                path.build().ok()
            };
            if let Some(path) = arc(0., 2. * PI) {
                window.paint_path(path, hsla(color.opacity(0.25)));
            }
            let start = -PI / 2. + turn * 2. * PI;
            let sweep = progress.unwrap_or(0.28).clamp(0.02, 1.) * 2. * PI;
            if let Some(path) = arc(start, start + sweep) {
                window.paint_path(path, hsla(color));
            }
        },
    )
    .size(px(size))
    .flex_none()
}

/// Swift's `ChromeSegmented`: the frame holding its segments.
pub(crate) fn segmented(p: &impulse_kit::chrome::ChromePalette) -> gpui_kit::Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(2.))
        .p(px(2.))
        .rounded(px(metrics::RADIUS_SMALL + 2.))
        .bg(hsla(p.hover))
        .border_1()
        .border_color(hsla(p.hairline))
}

/// One of `segmented`'s options: semibold on a raised, lightly shadowed
/// pill when selected.
pub(crate) fn segment(
    id: impl Into<gpui_kit::ElementId>,
    label: impl Into<gpui_kit::SharedString>,
    selected: bool,
    p: &impulse_kit::chrome::ChromePalette,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .px(px(10.))
        .h(px(20.))
        .rounded(px(metrics::RADIUS_SMALL))
        .text_size(px(11.5))
        .font_weight(if selected {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::MEDIUM
        })
        .text_color(hsla(if selected { p.text } else { p.text_secondary }))
        .when(selected, |d| d.bg(hsla(p.raised)).shadow_xs())
        .cursor_pointer()
        .whitespace_nowrap()
        .child(label.into())
}

/// AppKit's segmented control (SwiftUI's `.segmented` picker): a 24-high
/// track, segments of equal width, the selected one raised, a hairline
/// between two that aren't.
#[allow(clippy::too_many_arguments)]
pub(crate) fn native_segmented(
    id: &'static str,
    labels: Vec<gpui_kit::SharedString>,
    selected: usize,
    segment_width: f32,
    light: bool,
    disabled: bool,
    on_select: impl Fn(usize, &mut Window, &mut gpui_kit::App) + 'static,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let ink = if light {
        KitRgba::BLACK
    } else {
        KitRgba::WHITE
    };
    let on_select = Rc::new(on_select);
    let count = labels.len();
    let mut track = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .h(px(24.))
        .p(px(1.))
        .rounded(px(7.))
        .bg(hsla(ink.opacity(if light { 0.06 } else { 0.075 })))
        .text_size(px(13.))
        .text_color(hsla(ink.opacity(if disabled { 0.35 } else { 0.88 })));
    for (index, label) in labels.into_iter().enumerate() {
        let on_select = on_select.clone();
        let chosen = index == selected;
        track = track.child(
            div()
                .id((id, index))
                .relative()
                .w(px(segment_width))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .when(chosen, |d| {
                    d.bg(if light {
                        gpui_kit::white()
                    } else {
                        hsla(KitRgba::WHITE.opacity(0.2))
                    })
                    .shadow_xs()
                })
                .when(!disabled, |d| {
                    d.on_click(move |_, window, cx| on_select(index, window, cx))
                })
                // A hairline before a segment, between two unselected ones.
                .when(
                    index > 0 && !chosen && index - 1 != selected && count > 1,
                    |d| {
                        d.child(
                            div()
                                .absolute()
                                .left_0()
                                .top(px(5.))
                                .bottom(px(5.))
                                .w(px(1.))
                                .bg(hsla(ink.opacity(0.15))),
                        )
                    },
                )
                .child(label),
        );
    }
    track
}

/// The macOS spinner (a small `ProgressView` at 0.8): eight rounded spokes
/// turning clockwise a spoke per `step`, the newest brightest. `color` is
/// the label color it's drawn in at low strength.
pub(crate) fn spinner(color: KitRgba, size: f32, step: usize) -> impl IntoElement {
    use std::f32::consts::PI;
    const SPOKES: usize = 8;
    gpui_kit::canvas(
        |_, _, _| {},
        move |bounds: gpui_kit::Bounds<gpui_kit::Pixels>, _, window, _| {
            let center = bounds.center();
            let scale = bounds.size.width / 13.5;
            let (inner, outer, half) = (scale * 2.5, scale * 6.25, scale * 0.95);
            let head = step % SPOKES;
            for spoke in 0..SPOKES {
                let angle = -PI / 2. + spoke as f32 * 2. * PI / SPOKES as f32;
                let (dx, dy) = (angle.cos(), angle.sin());
                // A capsule from `inner` to `outer` along the spoke.
                let mut path = gpui_kit::PathBuilder::fill();
                let ends = [(outer - half, angle), (inner + half, angle + PI)];
                for (index, (distance, facing)) in ends.into_iter().enumerate() {
                    let middle =
                        gpui_kit::point(center.x + distance * dx, center.y + distance * dy);
                    for i in 0..=6 {
                        let a = facing - PI / 2. + PI * i as f32 / 6.;
                        let at =
                            gpui_kit::point(middle.x + half * a.cos(), middle.y + half * a.sin());
                        if index == 0 && i == 0 {
                            path.move_to(at);
                        } else {
                            path.line_to(at);
                        }
                    }
                }
                path.close();
                let age = (head + SPOKES - spoke) % SPOKES;
                let strength = 0.35 * (1. - age as f32 / SPOKES as f32 * 0.75);
                if let Ok(path) = path.build() {
                    window.paint_path(path, hsla(color.opacity(strength)));
                }
            }
        },
    )
    .size(px(size))
    .flex_none()
}

/// Text set with Swift's `.tracking(0.6)` (its uppercase section headers):
/// GPUI has no letter spacing, so each character is laid out on its own,
/// 0.6 apart.
pub(crate) fn tracked(text: &str) -> gpui_kit::Div {
    div().flex().gap(px(0.6)).children(
        text.chars()
            .map(|c| if c == ' ' { '\u{a0}' } else { c }.to_string()),
    )
}

/// A sidebar row's highlight, inset 6 from the row's edges (Swift's
/// `rowBackground`): the selection, else the hover while `group` is hovered.
/// The row is `relative` and takes this as its first child.
pub(crate) fn row_background(
    selected: bool,
    group: &'static str,
    p: &impulse_kit::chrome::ChromePalette,
) -> gpui_kit::Div {
    let hover = hsla(p.hover);
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .left(px(6.))
        .right(px(6.))
        .rounded(px(metrics::RADIUS_SMALL + 1.))
        .when(selected, |d| d.bg(hsla(p.selection)))
        .when(!selected, |d| d.group_hover(group, move |s| s.bg(hover)))
}

/// A filter field as Swift's `ChromeTextField`: 24 high, the icon inside,
/// the content color with a hairline that turns into the focus ring.
pub(crate) fn chrome_text_field(
    state: &gpui_kit::Entity<gpui_kit::component::input::InputState>,
    icon_name: &str,
    p: &impulse_kit::chrome::ChromePalette,
    window: &Window,
    cx: &gpui_kit::App,
) -> gpui_kit::Div {
    use gpui_kit::component::input::Input;
    use gpui_kit::component::Sizable as _;
    use gpui_kit::Focusable as _;
    let focused = state.read(cx).focus_handle(cx).is_focused(window);
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(24.))
        .px(px(8.))
        .rounded(px(metrics::RADIUS_SMALL + 1.))
        .bg(hsla(p.content))
        .border_1()
        .border_color(hsla(if focused {
            p.focus_ring
        } else {
            p.hairline_strong
        }))
        .child(icon(icon_name, 12., p.text_tertiary))
        .child(
            div().flex_1().min_w_0().child(
                Input::new(state)
                    .small()
                    .cleanable(true)
                    .appearance(false)
                    .px_0(),
            ),
        )
}

/// `~/…` for paths in the home folder.
/// "+41 −9" in the git colors (the Swift `DiffStat`).
pub(crate) fn diff_stat(
    added: usize,
    removed: usize,
    p: &impulse_kit::chrome::ChromePalette,
) -> impl IntoElement {
    div()
        .flex()
        .gap(px(4.))
        .font_family("JetBrains Mono")
        .text_size(px(11.))
        .when(added > 0 || removed == 0, |d| {
            d.child(
                div()
                    .text_color(hsla(p.git_added))
                    .child(format!("+{added}")),
            )
        })
        .when(removed > 0, |d| {
            d.child(
                div()
                    .text_color(hsla(p.git_deleted))
                    .child(format!("\u{2212}{removed}")),
            )
        })
}

/// One status-bar entry (the Swift `StatusItem`): an icon and a label,
/// highlighted on hover when it does something.
pub(crate) fn status_item(
    id: impl Into<gpui_kit::ElementId>,
    icon_name: Option<&str>,
    help: &str,
    tint: Option<KitRgba>,
    p: &impulse_kit::chrome::ChromePalette,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let color = tint.unwrap_or(p.text_secondary);
    let help: gpui_kit::SharedString = help.to_string().into();
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .h(px(20.))
        .px(px(6.))
        .rounded(px(metrics::RADIUS_SMALL))
        .whitespace_nowrap()
        .text_color(hsla(color))
        .hover(|s| s.bg(hsla(p.hover)))
        .role(gpui_kit::Role::Button)
        .aria_label(help.clone())
        .tooltip(move |window, cx| {
            gpui_kit::component::tooltip::Tooltip::new(help.clone()).build(window, cx)
        })
        .children(icon_name.map(|name| icon(name, 11.5, color)))
}

pub(crate) fn abbreviate_home(path: &str) -> String {
    let home = crate::workspace::home().to_string_lossy().into_owned();
    match path.strip_prefix(&home) {
        Some(rest) if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => {
            format!("~{rest}")
        }
        _ => path.to_string(),
    }
}

/// One row of the file tree.
struct TreeRow {
    path: PathBuf,
    name: String,
    is_dir: bool,
    depth: usize,
}

/// The visible rows: the root's entries, and those of every expanded
/// folder under them.
fn tree_rows(workspace: &Workspace, root: &Path) -> Vec<TreeRow> {
    fn walk(w: &Workspace, dir: &Path, depth: usize, rows: &mut Vec<TreeRow>) {
        for (name, is_dir) in w.children.get(dir).map(Vec::as_slice).unwrap_or_default() {
            let path = dir.join(name);
            let expanded = *is_dir && w.expanded_dirs.contains(&path);
            rows.push(TreeRow {
                path: path.clone(),
                name: name.clone(),
                is_dir: *is_dir,
                depth,
            });
            if expanded {
                walk(w, &path, depth + 1, rows);
            }
        }
    }
    let mut rows = Vec::new();
    walk(workspace, root, 0, &mut rows);
    rows
}

impl Workbench {
    /// The folder the file tree shows: a folder workspace's root, or for
    /// Scratch the active tab's folder.
    pub(crate) fn tree_root(&self, cx: &gpui_kit::App) -> PathBuf {
        let workspace = self.workspace();
        match workspace.kind {
            Kind::Folder => workspace.root.clone(),
            Kind::Scratch => self
                .focused_surface()
                .and_then(|s| s.directory(cx))
                .filter(|d| d.is_dir())
                .unwrap_or_else(|| workspace.root.clone()),
        }
    }

    /// The sidebar's current panel has the keyboard.
    pub(crate) fn panel_has_focus(&self, window: &Window, cx: &gpui_kit::App) -> bool {
        match self.panel {
            SidebarPanel::Files => self.tree_focus.contains_focused(window, cx),
            SidebarPanel::Changes => self.changes_list_focused(window, cx),
            SidebarPanel::Search => self.search.read(cx).is_focused(window, cx),
        }
    }

    /// ⇧⌘E: the tree takes the keyboard, its selection on the open file
    /// (else its first row).
    pub(crate) fn focus_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.tree_focus, cx);
        let root = self.tree_root(cx);
        let rows = tree_rows(self.workspace(), &root);
        let open = self.active_editor().map(|e| e.read(cx).path.clone());
        let current = self.tree_selection.clone().or(open);
        if !rows.iter().any(|r| Some(&r.path) == current.as_ref()) {
            self.tree_selection = rows.first().map(|r| r.path.clone());
        } else {
            self.tree_selection = current;
        }
        cx.notify();
    }

    /// The tree's keys (Swift's file list): ↑↓ move, → expands or steps
    /// into a folder, ← collapses or goes to the parent, Return opens a
    /// file or toggles a folder.
    fn tree_key(
        &mut self,
        event: &gpui_kit::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = self.tree_root(cx);
        let rows = tree_rows(self.workspace(), &root);
        if rows.is_empty() {
            return;
        }
        let index = rows
            .iter()
            .position(|r| Some(&r.path) == self.tree_selection.as_ref());
        let select = |this: &mut Self, i: usize| {
            let i = i.min(rows.len() - 1);
            this.tree_selection = Some(rows[i].path.clone());
            this.tree_scroll.scroll_to_item(i);
        };
        let row = index.map(|i| &rows[i]);
        let expanded = |path: &PathBuf, this: &Self| this.workspace().expanded_dirs.contains(path);
        match event.keystroke.key.as_str() {
            "up" => select(self, index.map_or(0, |i| i.saturating_sub(1))),
            "down" => select(self, index.map_or(0, |i| i + 1)),
            "right" => match row {
                Some(r) if r.is_dir && !expanded(&r.path, self) => {
                    self.toggle_dir(r.path.clone(), cx)
                }
                Some(r) if r.is_dir => {
                    if let Some(i) =
                        index.filter(|i| rows.get(i + 1).is_some_and(|n| n.depth > r.depth))
                    {
                        select(self, i + 1);
                    }
                }
                _ => return,
            },
            "left" => match row {
                Some(r) if r.is_dir && expanded(&r.path, self) => {
                    self.toggle_dir(r.path.clone(), cx)
                }
                Some(r) => {
                    if let Some(parent) = rows[..index.unwrap_or(0)]
                        .iter()
                        .rposition(|p| p.is_dir && p.depth + 1 == r.depth)
                    {
                        select(self, parent);
                    }
                }
                None => return,
            },
            "enter" => match row {
                Some(r) if r.is_dir => self.toggle_dir(r.path.clone(), cx),
                Some(r) => {
                    let path = r.path.clone();
                    self.open_file(&path, window, cx);
                }
                None => return,
            },
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn toggle_dir(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let show_hidden = SettingsStore::get(cx).sidebar_show_hidden;
        let workspace = self.workspace_mut();
        if !workspace.expanded_dirs.remove(&path) {
            let entries = crate::workspace::list_dir(&path, show_hidden);
            workspace.children.insert(path.clone(), entries);
            workspace.expanded_dirs.insert(path);
        }
        workspace.save_expanded();
        cx.notify();
    }

    /// A tab's context menu. Its actions find the tab by id when chosen:
    /// tabs can close or move while the menu is open.
    fn tab_menu(
        &self,
        ws: usize,
        index: usize,
        cx: &Context<Self>,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        type Run = Rc<dyn Fn(&mut Workbench, usize, usize, &mut Window, &mut Context<Workbench>)>;
        let workspace = &self.workspaces[ws];
        let tab = &workspace.tabs[index];
        let current = ws == self.active;
        let mut items: Vec<(&'static str, Option<Run>)> = vec![
            (
                if tab.pinned { "Unpin Tab" } else { "Pin Tab" },
                Some(Rc::new(|w, ws, t, _, cx| w.toggle_pin(ws, t, cx))),
            ),
            (
                "Close Tab",
                Some(Rc::new(|w, ws, t, window, cx| {
                    w.active = ws;
                    w.close_tab_at(t, window, cx)
                })),
            ),
            ("", None),
        ];
        if current && index != workspace.active {
            items.push((
                "Move into Current Tab, Right",
                Some(Rc::new(|w, _, t, window, cx| {
                    w.join_tab(t, false, window, cx)
                })),
            ));
            items.push((
                "Move into Current Tab, Below",
                Some(Rc::new(|w, _, t, window, cx| {
                    w.join_tab(t, true, window, cx)
                })),
            ));
            items.push(("", None));
        } else if current && tab.is_split() {
            items.push((
                "Even Out Panes",
                Some(Rc::new(|w, _, _, window, cx| {
                    w.run_command("equalize_panes", window, cx)
                })),
            ));
            items.push((
                "Move Pane to New Tab",
                Some(Rc::new(|w, _, _, window, cx| {
                    w.run_command("move_pane_to_tab", window, cx)
                })),
            ));
            items.push(("", None));
        }
        items.push((
            "New Tab",
            Some(Rc::new(|w, _, _, window, cx| {
                w.run_command("new_tab", window, cx)
            })),
        ));
        let (id, workbench) = (tab.id, cx.entity().downgrade());
        move |mut menu, _, _| {
            for (title, run) in &items {
                let Some(run) = run.clone() else {
                    menu = menu.separator();
                    continue;
                };
                let workbench = workbench.clone();
                menu = menu.item(PopupMenuItem::new(*title).on_click(move |_, window, cx| {
                    let _ = workbench.update(cx, |w, cx| {
                        let found = w.workspaces.iter().enumerate().find_map(|(ws, workspace)| {
                            let t = workspace.tabs.iter().position(|t| t.id == id)?;
                            Some((ws, t))
                        });
                        if let Some((ws, t)) = found {
                            run(w, ws, t, window, cx)
                        }
                    });
                }));
            }
            menu
        }
    }

    /// `sidebar_tabs`: the sidebar lists the tabs (while it's shown) and the
    /// titlebar doesn't.
    fn tabs_in_sidebar(&self, cx: &gpui_kit::App) -> bool {
        let settings = SettingsStore::get(cx);
        settings.sidebar_tabs && settings.sidebar_visible
    }

    pub(crate) fn titlebar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &self.palette;
        // macOS draws its traffic lights in this inset; on Linux the window
        // controls are Impulse's own (client-side decorations).
        let leading = if cfg!(target_os = "macos") {
            metrics::TRAFFIC_LIGHT_INSET
        } else {
            12.0
        };
        let workspace = self.workspace();
        let shortcut = impulse_kit::keybindings::display_for(
            "command_palette",
            &SettingsStore::get(cx).keybinding_overrides,
            impulse_kit::keybindings::Platform::current(),
        )
        .unwrap_or_default();
        let mut tabs = div().flex().items_center().gap(px(2.)).min_w_0();
        for (index, tab) in workspace.tabs.iter().enumerate() {
            let active = index == workspace.active;
            let surface = tab.focused_surface();
            let dirty = tab.panes.values().any(|s| s.is_dirty(cx));
            let attention = !active && Self::tab_needs_attention(tab, cx);
            let color = if active { p.text } else { p.text_secondary };
            let compact = tab.pinned;
            let drop_color = hsla(p.selection);
            let title = surface.title(cx);
            // The title, the preview hint and a program's status.
            let help = std::iter::once(title.clone())
                .chain(
                    tab.preview
                        .then(|| "Preview: double-click to keep".to_string()),
                )
                .chain(
                    surface
                        .terminal()
                        .and_then(|t| t.read(cx).session_status.summary()),
                )
                .collect::<Vec<_>>()
                .join("\n");
            // Read out as the title, then whether it's unsaved, waiting or
            // a preview (Swift's tab accessibility label).
            let spoken = [
                Some(title.clone()),
                dirty.then(|| "unsaved".to_string()),
                attention.then(|| "needs attention".to_string()),
                tab.preview.then(|| "preview".to_string()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", ");
            let mut t = div()
                .id(("tab", tab.id))
                .role(gpui_kit::Role::Tab)
                .aria_label(spoken)
                .aria_selected(active)
                .group("tab")
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(metrics::TAB_HEIGHT))
                .pl(px(if compact { 8. } else { 9. }))
                .pr(px(if compact { 8. } else { 5. }))
                .rounded(px(metrics::RADIUS))
                .text_color(hsla(color))
                .hover(|s| s.bg(hsla(p.hover)))
                .on_click(
                    cx.listener(move |w, event: &gpui_kit::ClickEvent, window, cx| {
                        if event.click_count() >= 2 {
                            w.keep_tab(index, cx);
                        }
                        w.activate_tab(index, window, cx)
                    }),
                )
                .on_drag(
                    DraggedTab {
                        index,
                        title: title.clone(),
                    },
                    |dragged, _, _, cx| {
                        cx.new(|_| crate::file_tree_actions::DragPreview(dragged.title.clone()))
                    },
                )
                .drag_over::<DraggedTab>(move |style, _, _, _| style.bg(drop_color))
                .on_drop(cx.listener(move |w, dragged: &DraggedTab, _, cx| {
                    w.move_tab(dragged.index, index, cx)
                }))
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(help.clone()).build(window, cx)
                })
                .child(self.tab_icon(
                    tab,
                    surface.icon(),
                    metrics::ICON,
                    if active { p.text } else { p.text_tertiary },
                    cx,
                ));
            if !compact {
                let panes = tab.panes.len();
                t = t
                    .child(
                        div()
                            .max_w(px(180.))
                            .truncate()
                            .text_size(px(12.))
                            .when(active, |d| d.font_weight(gpui_kit::FontWeight::MEDIUM))
                            .when(tab.preview, |d| d.italic())
                            .child(title),
                    )
                    .when(panes > 1, |d| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(2.))
                                .child(icon(
                                    if tab.zoomed.is_some() {
                                        "maximize-2"
                                    } else {
                                        "columns-2"
                                    },
                                    10.,
                                    p.text_tertiary,
                                ))
                                .child(
                                    div()
                                        .font_family("JetBrains Mono")
                                        .text_size(px(10.))
                                        .text_color(hsla(p.text_tertiary))
                                        .child(panes.to_string()),
                                ),
                        )
                    })
                    .child(
                        div()
                            .id(("close", tab.id))
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(16.))
                            .h(px(16.))
                            .rounded(px(metrics::RADIUS_SMALL))
                            .hover(|s| s.bg(hsla(p.pressed)))
                            .on_click(cx.listener(move |w, _, window, cx| {
                                cx.stop_propagation();
                                w.close_tab_at(index, window, cx)
                            }))
                            // The close button on hover; otherwise a dot for
                            // attention or unsaved changes, as in Swift.
                            .child(
                                div()
                                    .invisible()
                                    .group_hover("tab", |s| s.visible())
                                    .absolute()
                                    .child(icon("x", metrics::ICON_SMALL, p.text_secondary)),
                            )
                            .when(attention || dirty, |d| {
                                d.child(div().group_hover("tab", |s| s.invisible()).child(
                                    crate::agent_views::status_dot(
                                        7.,
                                        if attention {
                                            p.attention
                                        } else {
                                            p.text_secondary
                                        },
                                    ),
                                ))
                            }),
                    );
            }
            if active {
                t = t.bg(hsla(p.raised)).relative().child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left(px(8.))
                        .right(px(8.))
                        .h(px(2.))
                        .rounded_full()
                        .bg(hsla(p.accent)),
                );
            }
            let t = t.context_menu(self.tab_menu(self.active, index, cx));
            tabs = tabs.child(t);
        }
        let tabs_in_sidebar = self.tabs_in_sidebar(cx);
        let sidebar_visible = SettingsStore::get(cx).sidebar_visible;
        // Over the sidebar the leading part fills its width, so tabs start
        // where the content column starts; without it, it hugs its content.
        let leading_width = sidebar_visible.then(|| self.sidebar_width(cx).max(220.));
        let repo = crate::repository_state::RepositoryStore::summary(&workspace.root, cx);
        let leading_part =
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .h_full()
                .pl(px(leading))
                .pr(px(8.))
                .overflow_hidden()
                .when_some(leading_width, |d, width| d.w(px(width)))
                .children(crate::menus::menu_button(cx))
                .child(
                    div()
                        .id("toggle-sidebar")
                        .flex_none()
                        .p(px(3.))
                        .rounded(px(metrics::RADIUS_SMALL))
                        .cursor_pointer()
                        .hover(|s| s.bg(hsla(p.hover)))
                        .on_click(cx.listener(|w, _, window, cx| {
                            w.run_command("toggle_sidebar", window, cx)
                        }))
                        .child(icon(
                            "panel-left",
                            metrics::ICON_LARGE,
                            if sidebar_visible {
                                p.text
                            } else {
                                p.text_secondary
                            },
                        )),
                )
                .child(
                    div()
                        .id("titlebar-workspace")
                        .flex_shrink(1.)
                        .min_w(px(30.))
                        .truncate()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(hsla(p.text))
                        .cursor_pointer()
                        .on_click(cx.listener(|w, _, window, cx| {
                            w.run_command("switch_workspace", window, cx)
                        }))
                        .child(workspace.name()),
                )
                .when(!workspace.branch.is_empty(), |d| {
                    d.child(
                        div()
                            .id("titlebar-branch")
                            .flex()
                            // The branch truncates before the workspace's name.
                            .flex_shrink(10.)
                            .min_w_0()
                            .items_center()
                            .gap(px(4.))
                            .cursor_pointer()
                            .on_click(cx.listener(|w, _, window, cx| {
                                w.run_command("switch_branch", window, cx)
                            }))
                            .child(icon("git-branch", 11., p.text_tertiary))
                            .child(
                                div()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis_middle()
                                    .text_size(px(11.5))
                                    .text_color(hsla(p.text_secondary))
                                    .child(workspace.branch.clone()),
                            )
                            .children(self.branch_sync_text(cx).map(|sync| {
                                div()
                                    .flex_none()
                                    .font_family("JetBrains Mono")
                                    .text_size(px(10.5))
                                    .text_color(hsla(p.text_tertiary))
                                    .child(sync)
                            })),
                    )
                });
        div()
            .flex()
            .items_center()
            .h(px(metrics::TITLEBAR_HEIGHT))
            .pr(px(8.))
            .bg(hsla(p.chrome))
            .border_b_1()
            .border_color(hsla(p.hairline))
            .child(leading_part)
            // With `sidebar_tabs`, the sidebar lists the tabs while it's open.
            .when(!tabs_in_sidebar, |d| {
                d.child(
                    tabs.child(
                        div()
                            .id("new-tab")
                            .flex_none()
                            .ml(px(4.))
                            .p(px(4.))
                            .rounded(px(metrics::RADIUS_SMALL))
                            .hover(|s| s.bg(hsla(p.hover)))
                            .on_click(
                                cx.listener(|w, _, window, cx| {
                                    w.run_command("new_tab", window, cx)
                                }),
                            )
                            .child(icon("plus", metrics::ICON, p.text_secondary)),
                    ),
                )
            })
            .child(crate::window_frame::drag_region(window))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(4.))
                    .children(self.agent_inbox_button(cx))
                    .child(
                        div()
                            .id("search-field")
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(6.))
                            .h(px(24.))
                            .pl(px(8.))
                            .pr(px(4.))
                            .rounded(px(metrics::RADIUS))
                            .bg(hsla(p.raised))
                            .hover(|s| s.bg(hsla(p.pressed)))
                            .text_size(px(11.5))
                            .text_color(hsla(p.text_tertiary))
                            .cursor_pointer()
                            .on_click(cx.listener(|w, _, window, cx| {
                                w.run_command("command_palette", window, cx)
                            }))
                            .child(icon("search", 12., p.text_tertiary))
                            .child("Search or run a command")
                            .child(
                                // The Swift `KeyHint`.
                                div()
                                    .h(px(16.))
                                    .px(px(4.))
                                    .flex()
                                    .items_center()
                                    .rounded(px(3.))
                                    .border_1()
                                    .border_color(hsla(p.hairline_strong))
                                    .text_size(px(10.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(shortcut),
                            ),
                    )
                    .children(repo.filter(|r| r.changed > 0).map(|r| {
                        let help: gpui_kit::SharedString = format!(
                            "Review {} changed file{}",
                            r.changed,
                            if r.changed == 1 { "" } else { "s" }
                        )
                        .into();
                        div()
                            .id("diff-pill")
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(6.))
                            .h(px(24.))
                            .px(px(8.))
                            .rounded(px(metrics::RADIUS))
                            .bg(hsla(p.raised))
                            .hover(|s| s.bg(hsla(p.pressed)))
                            .cursor_pointer()
                            .tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(help.clone())
                                    .build(window, cx)
                            })
                            .on_click(cx.listener(|w, _, window, cx| {
                                w.run_command("review_changes", window, cx)
                            }))
                            .child(icon("file-diff", 12., p.text_secondary))
                            .child(
                                div()
                                    .font_family("JetBrains Mono")
                                    .text_size(px(11.))
                                    .text_color(hsla(p.text_secondary))
                                    .child(r.changed.to_string()),
                            )
                            .child(diff_stat(r.added, r.removed, p))
                    })),
            )
            .children(crate::window_frame::controls(window, p))
    }

    fn workspace_row(&self, index: usize, indented: bool, cx: &mut Context<Self>) -> AnyElement {
        let p = &self.palette;
        let workspace = &self.workspaces[index];
        let selected = index == self.active;
        let id = workspace.id;
        let tabs_in_sidebar = self.tabs_in_sidebar(cx);
        // Listed tabs stay open for the active workspace.
        let expanded = workspace.expanded || tabs_in_sidebar && selected;
        let drop_color = hsla(p.selection);
        let repo = crate::repository_state::RepositoryStore::summary(&workspace.root, cx);
        let attention = workspace
            .tabs
            .iter()
            .filter(|tab| Self::tab_needs_attention(tab, cx))
            .count();
        let mut column = div().flex().flex_col();
        let row = div()
            .id(("workspace", id))
            .role(gpui_kit::Role::ListItem)
            .aria_label(match workspace.branch.as_str() {
                "" => workspace.name(),
                branch => format!("{}, {branch}", workspace.name()),
            })
            .aria_selected(selected)
            .group("workspace")
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(26.))
            .pl(px(if indented { 16. } else { 8. }))
            .pr(px(6.))
            .relative()
            // When its tabs are listed, the selected tab carries the highlight.
            .child(row_background(selected && !expanded, "workspace", p))
            .when(selected, |d| {
                d.child(
                    div()
                        .absolute()
                        .left(px(3.))
                        .top(px(6.))
                        .w(px(2.))
                        .h(px(14.))
                        .rounded_full()
                        .bg(hsla(p.accent)),
                )
            })
            .on_click(cx.listener(move |w, _, window, cx| w.activate_workspace(index, window, cx)))
            .on_drag(
                crate::task_sidebar::DraggedWorkspace {
                    id,
                    name: workspace.name(),
                },
                |dragged, _, _, cx| {
                    cx.new(|_| crate::file_tree_actions::DragPreview(dragged.name.clone()))
                },
            )
            .drag_over::<crate::task_sidebar::DraggedWorkspace>(move |style, _, _, _| {
                style.bg(drop_color)
            })
            .on_drop(cx.listener(
                move |w, dragged: &crate::task_sidebar::DraggedWorkspace, _, cx| {
                    w.drop_workspace(dragged.id, id, cx)
                },
            ))
            .child(
                div()
                    .id(("expand", id))
                    .w(px(12.))
                    .flex_none()
                    // Shown on hover or while the tabs are listed, as Swift's.
                    .when(!expanded, |d| {
                        d.invisible().group_hover("workspace", |s| s.visible())
                    })
                    .when(tabs_in_sidebar && selected, |d| d.invisible())
                    .on_click(cx.listener(move |w, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(ws) = w.workspaces.get_mut(index) {
                            ws.expanded = !ws.expanded;
                        }
                        cx.notify();
                    }))
                    .child(icon(
                        if expanded {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        },
                        10.,
                        p.text_tertiary,
                    )),
            )
            // A progress ring while a program in one of its terminals
            // reports progress.
            .child(
                match workspace
                    .all_surfaces()
                    .filter_map(Surface::terminal)
                    .find_map(|t| {
                        let (state, percent) = t.read(cx).progress?;
                        Some(
                            (state != impulse_terminal::osc_scanner::ProgressState::Indeterminate)
                                .then(|| percent.map(|p| p as f32 / 100.))
                                .flatten(),
                        )
                    }) {
                    Some(fraction) => div()
                        .size(px(13.))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .child(progress_ring(fraction, p.working, 12., 1.6, 0.))
                        .into_any_element(),
                    None => icon(
                        match (workspace.kind, repo.is_some()) {
                            (Kind::Scratch, _) => "square-terminal",
                            (Kind::Folder, true) => "folder-git-2",
                            (Kind::Folder, false) => "folder",
                        },
                        13.,
                        if selected { p.accent } else { p.text_tertiary },
                    )
                    .into_any_element(),
                },
            )
            .child(
                div()
                    .flex_none()
                    .max_w(px(150.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis_middle()
                    .text_size(px(12.))
                    .text_color(hsla(if selected { p.text } else { p.text_secondary }))
                    .font_weight(if selected {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .child(workspace.name()),
            )
            .when(
                !workspace.branch.is_empty() && workspace.branch != workspace.name(),
                |d| {
                    d.child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis_middle()
                            .font_family("JetBrains Mono")
                            .text_size(px(10.5))
                            .text_color(hsla(p.text_tertiary))
                            .child(workspace.branch.clone()),
                    )
                },
            )
            .when(workspace.kind == Kind::Scratch, |d| {
                d.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(px(10.5))
                        .text_color(hsla(p.text_tertiary))
                        .child("follows tab"),
                )
            })
            .child(div().flex_1().min_w(px(4.)))
            // The badges keep their size; the name and branch truncate.
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .children(self.task_chips(index, cx))
                    .children(
                        impulse_kit::listening_ports::chip(&workspace.ports).map(|chip| {
                            let help = workspace
                                .ports
                                .iter()
                                .map(|p| format!(":{} {}", p.port, p.process))
                                .collect::<Vec<_>>()
                                .join("\n");
                            div()
                                .id(("ports", id))
                                .font_family("JetBrains Mono")
                                .text_size(px(10.))
                                .text_color(hsla(p.text_tertiary))
                                .tooltip(move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(help.clone())
                                        .build(window, cx)
                                })
                                .child(chip)
                        }),
                    )
                    .children(self.workspace_agent_counts(index, cx))
                    .children(repo.as_ref().filter(|r| r.behind > 0).map(|r| {
                        let behind = r.behind;
                        let upstream = r.upstream.clone().unwrap_or_default();
                        let root = r.root.clone();
                        div()
                            .id(("behind", id))
                            .font_family("JetBrains Mono")
                            .text_size(px(10.))
                            .text_color(hsla(p.text_tertiary))
                            .cursor_pointer()
                            .tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(format!(
                                    "{behind} commit{} behind {upstream}: click to pull",
                                    if behind == 1 { "" } else { "s" }
                                ))
                                .build(window, cx)
                            })
                            .on_click(cx.listener(move |w, _, window, cx| {
                                cx.stop_propagation();
                                let repo =
                                    crate::repository_state::RepositoryStore::state(&root, cx);
                                w.git_pull(&repo, None, window, cx);
                            }))
                            .child(format!("\u{2193}{behind}"))
                    }))
                    .children(
                        repo.as_ref()
                            .filter(|r| r.changed > 0)
                            .map(|r| crate::workbench_chrome::diff_stat(r.added, r.removed, p)),
                    )
                    .when(attention > 0, |d| {
                        // The Swift `Badge`: the count on the attention color.
                        d.child(
                            div()
                                .min_w(px(16.))
                                .h(px(15.))
                                .px(px(5.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(hsla(p.attention))
                                .text_size(px(10.))
                                .font_weight(FontWeight::BOLD)
                                .text_color(hsla(p.text_on_accent))
                                .child(attention.to_string()),
                        )
                    })
                    .when(
                        attention == 0 && !expanded && workspace.tabs.len() > 1,
                        |d| {
                            d.child(
                                div()
                                    .font_family("JetBrains Mono")
                                    .text_size(px(10.))
                                    .text_color(hsla(p.text_tertiary))
                                    .child(workspace.tabs.len().to_string()),
                            )
                        },
                    ),
            )
            .child(self.new_workspace_button(index, cx))
            .tooltip({
                let help = self.workspace_help(index);
                move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(help.clone()).build(window, cx)
                }
            });
        column = column.child(row.context_menu(self.workspace_menu(index, cx)));
        if expanded {
            for (t, tab) in workspace.tabs.iter().enumerate() {
                let surface = tab.focused_surface();
                let active_tab = selected && t == workspace.active;
                let menu = self.tab_menu(index, t, cx);
                column = column.child(
                    div()
                        .id(("workspace-tab", tab.id))
                        .group("workspace-tab")
                        .relative()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .h(px(24.))
                        .pl(px(40.))
                        .pr(px(8.))
                        .text_size(px(11.5))
                        .text_color(hsla(if active_tab { p.text } else { p.text_secondary }))
                        .child(row_background(active_tab, "workspace-tab", p))
                        .on_click(cx.listener(
                            move |w, event: &gpui_kit::ClickEvent, window, cx| {
                                w.active = index;
                                if event.click_count() >= 2 {
                                    w.keep_tab(t, cx);
                                }
                                w.activate_tab(t, window, cx);
                            },
                        ))
                        .child(self.tab_icon(
                            tab,
                            surface.icon(),
                            metrics::ICON_SMALL,
                            p.text_tertiary,
                            cx,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis_middle()
                                .when(tab.preview, |d| d.italic())
                                .child(surface.title(cx)),
                        )
                        .children(
                            (!active_tab && Self::tab_needs_attention(tab, cx))
                                .then(|| crate::agent_views::status_dot(6., p.attention)),
                        )
                        .children(tab.pinned.then(|| icon("pin", 10., p.text_tertiary)))
                        .context_menu(menu),
                );
            }
            if tabs_in_sidebar && selected {
                column = column.child(
                    div()
                        .id(("workspace-new-tab", id))
                        .group("workspace-new-tab")
                        .relative()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .h(px(metrics::ROW_HEIGHT))
                        .pl(px(40.))
                        .text_size(px(11.5))
                        .text_color(hsla(p.text_tertiary))
                        .child(row_background(false, "workspace-new-tab", p))
                        .on_click(
                            cx.listener(|w, _, window, cx| w.run_command("new_tab", window, cx)),
                        )
                        .child(icon("plus", metrics::ICON_SMALL, p.text_tertiary))
                        .child("New Tab"),
                );
            }
        }
        column.into_any_element()
    }

    pub(crate) fn sidebar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &self.palette;
        let settings = SettingsStore::get(cx).clone();
        let open_path = self.active_editor().map(|e| e.read(cx).path.clone());
        let root = self.tree_root(cx);
        let workspace = self.workspace();
        let (root_drop, root_external) = (root.clone(), root.clone());
        let mut files = div()
            .id("files")
            .track_focus(&self.tree_focus)
            .on_key_down(cx.listener(Self::tree_key))
            .track_scroll(&self.tree_scroll)
            .on_drop(cx.listener(
                move |this, dragged: &crate::file_tree_actions::DraggedPath, window, cx| {
                    this.tree_drop(vec![dragged.0.clone()], &root_drop, window, cx)
                },
            ))
            .on_drop(
                cx.listener(move |this, paths: &gpui_kit::ExternalPaths, window, cx| {
                    this.tree_drop(paths.paths().to_vec(), &root_external, window, cx)
                }),
            )
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .py(px(4.));
        if self.panel == SidebarPanel::Files {
            let ignored = KitRgba::from_hex(&self.theme.git_ignored);
            // Scratch's tree follows the tab: list its folder when it's new.
            let rows = if workspace.children.contains_key(&root) {
                tree_rows(workspace, &root)
            } else {
                crate::workspace::list_dir(&root, settings.sidebar_show_hidden)
                    .into_iter()
                    .map(|(name, is_dir)| TreeRow {
                        path: root.join(&name),
                        name,
                        is_dir,
                        depth: 0,
                    })
                    .collect()
            };
            for (index, row) in rows.into_iter().enumerate() {
                let expanded = workspace.expanded_dirs.contains(&row.path);
                let selected = self.tree_selection.as_deref().or(open_path.as_deref())
                    == Some(row.path.as_path());
                let path = row.path.clone();
                let is_dir = row.is_dir;
                let status = self.git.tree_status(&row.path, p, ignored, cx);
                let menu = self.tree_menu(row.path.clone(), row.is_dir, cx);
                let row_element = div()
                    .id(("file", index))
                    .role(gpui_kit::Role::TreeItem)
                    .aria_label(row.name.clone())
                    .aria_level(row.depth + 1)
                    .aria_selected(selected)
                    .when(row.is_dir, |d| d.aria_expanded(expanded))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    // Swift's FlatFileRowView: 22 pt rows, 16 pt a level.
                    .h(px(22.))
                    .mx(px(4.))
                    .pl(px(4. + 16. * row.depth as f32))
                    .pr(px(8.))
                    .rounded(px(5.))
                    .text_size(px(13.))
                    .when(selected, |d| d.bg(hsla(p.accent).opacity(0.2)))
                    .when(!selected, |d| d.hover(|style| style.bg(hsla(p.hover))))
                    .on_click(
                        cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                            this.tree_selection = Some(path.clone());
                            if is_dir {
                                this.toggle_dir(path.clone(), cx);
                            } else if event.modifiers().alt {
                                // ⌥-click: beside the current tab, as Swift's
                                // tree opens it.
                                this.open_beside(&path, window, cx);
                            } else if event.click_count() >= 2 {
                                // The second click of a double-click keeps the
                                // preview.
                                this.open_file(&path, window, cx);
                            } else {
                                this.preview_file(&path, window, cx);
                            }
                        }),
                    )
                    .child(div().w(px(16.)).flex_none().flex().justify_center().child(
                        if row.is_dir {
                            icon(
                                if expanded {
                                    "chevron-down"
                                } else {
                                    "chevron-right"
                                },
                                metrics::ICON_SMALL,
                                p.text_tertiary,
                            )
                            .into_any_element()
                        } else {
                            div().into_any_element()
                        },
                    ))
                    .child(
                        gpui_kit::img(crate::file_icons::icon_path(
                            &row.name, row.is_dir, expanded,
                        ))
                        .size(px(16.))
                        .flex_none()
                        // Right after the chevron, as Swift's row has it.
                        .ml(px(-6.)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .when_some(status, |d, (_, color)| d.text_color(hsla(color)))
                            .child(row.name),
                    )
                    .children(status.filter(|(letter, _)| !letter.is_empty()).map(
                        |(letter, color)| {
                            div()
                                .flex_none()
                                .font_family("JetBrains Mono")
                                .text_size(px(10.))
                                .font_weight(FontWeight::BOLD)
                                .text_color(hsla(color))
                                .child(letter)
                        },
                    ));
                let (drag_path, drop_target, external_target) =
                    (row.path.clone(), row.path.clone(), row.path.clone());
                let drag_name = row
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let row_element = row_element
                    .on_drag(
                        crate::file_tree_actions::DraggedPath(drag_path),
                        move |_, _, _, cx| {
                            cx.new(|_| crate::file_tree_actions::DragPreview(drag_name.clone()))
                        },
                    )
                    .drag_over::<crate::file_tree_actions::DraggedPath>(|style, _, _, _| {
                        style.bg(gpui_kit::black().opacity(0.12))
                    })
                    .on_drop(cx.listener(
                        move |this, dragged: &crate::file_tree_actions::DraggedPath, window, cx| {
                            this.tree_drop(vec![dragged.0.clone()], &drop_target, window, cx)
                        },
                    ))
                    .on_drop(cx.listener(
                        move |this, paths: &gpui_kit::ExternalPaths, window, cx| {
                            this.tree_drop(paths.paths().to_vec(), &external_target, window, cx)
                        },
                    ));
                files = files.child(row_element.context_menu(menu));
            }
        }
        if self.panel == SidebarPanel::Search {
            let root = root.to_string_lossy().into_owned();
            self.search
                .update(cx, |search, cx| search.set_root(root, cx));
        }
        // With the Files panel's actions there's room for the selected
        // label only; otherwise every segment shows its label (Swift's
        // DockSegmentedTabs).
        let all_labels = self.panel != SidebarPanel::Files;
        let changed = crate::repository_state::RepositoryStore::summary(&workspace.root, cx)
            .map_or(0, |r| r.changed);
        let changes_label = if changed > 0 {
            format!("Changes {changed}")
        } else {
            "Changes".to_string()
        };
        let panel_button = |id: &'static str,
                            panel: SidebarPanel,
                            name: &'static str,
                            label: String,
                            cx: &mut Context<Self>| {
            let selected = self.panel == panel;
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(7.))
                .h(px(24.))
                .rounded(px(metrics::RADIUS))
                .cursor_pointer()
                .text_size(px(11.5))
                .font_weight(if selected {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::MEDIUM
                })
                .when(selected, |d| d.bg(hsla(p.raised)))
                .role(gpui_kit::Role::Tab)
                .aria_label(label.clone())
                .aria_selected(selected)
                .on_click(cx.listener(move |w, _, _, cx| {
                    w.panel = panel;
                    cx.notify();
                }))
                .child(icon(
                    name,
                    12.,
                    if selected { p.text } else { p.text_tertiary },
                ))
                .when(selected || all_labels, |d| {
                    d.text_color(hsla(if selected { p.text } else { p.text_secondary }))
                        .child(label)
                })
        };
        let show_hidden = settings.sidebar_show_hidden;
        let tree_action =
            |id: &'static str, name: &'static str, help: &'static str, active: bool| {
                div()
                    .id(id)
                    .flex_none()
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(metrics::RADIUS_SMALL))
                    .cursor_pointer()
                    .hover(|s| s.bg(hsla(p.hover)))
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(help).build(window, cx)
                    })
                    .child(icon(
                        name,
                        13.,
                        if active { p.accent } else { p.text_secondary },
                    ))
            };
        // Swift's WorkspacesSection: rows 1 pt apart, 6 pt above and below,
        // scrolling past 280 pt unless its handle set a height.
        let measured = self.workspaces_measured.clone();
        let mut workspaces = div()
            .id("workspaces")
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .map(|d| match self.workspaces_height {
                Some(height) => d.h(px(self.clamped_workspaces_height(height, window))),
                None => d.max_h(px(280.)),
            })
            .overflow_y_scroll()
            .py(px(6.))
            .gap(px(1.))
            .child(
                gpui_kit::canvas(
                    move |bounds, _, _| measured.set(f32::from(bounds.size.height)),
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            );
        // A repository's workspaces (its tasks) under its name.
        for (name, group) in self.workspace_groups() {
            let indented = name.is_some();
            if let Some(name) = name {
                workspaces = workspaces.child(self.repository_header(name, group[0], cx));
            }
            for index in group {
                workspaces = workspaces.child(self.workspace_row(index, indented, cx));
            }
        }
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(self.sidebar_width(cx)))
            .h_full()
            .bg(hsla(p.panel))
            .border_r_1()
            .border_color(hsla(p.hairline))
            .child(self.sidebar_divider(cx))
            .child(workspaces)
            .child(self.workspaces_handle(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .h(px(metrics::PANE_HEADER_HEIGHT + 6.))
                    .px(px(8.))
                    .child(panel_button(
                        "panel-files",
                        SidebarPanel::Files,
                        "folder-tree",
                        "Files".into(),
                        cx,
                    ))
                    .child(panel_button(
                        "panel-changes",
                        SidebarPanel::Changes,
                        "git-branch",
                        changes_label,
                        cx,
                    ))
                    .child(panel_button(
                        "panel-search",
                        SidebarPanel::Search,
                        "search",
                        "Search".into(),
                        cx,
                    ))
                    .when(self.panel == SidebarPanel::Files, |d| {
                        let (new_file_root, new_folder_root) = (root.clone(), root.clone());
                        d.child(div().flex_1().min_w(px(4.)))
                            .child(
                                tree_action("tree-new-file", "file-plus", "New File", false)
                                    .on_click(cx.listener(move |w, _, window, cx| {
                                        w.tree_new(&new_file_root, false, window, cx)
                                    })),
                            )
                            .child(
                                tree_action("tree-new-folder", "folder-plus", "New Folder", false)
                                    .on_click(cx.listener(move |w, _, window, cx| {
                                        w.tree_new(&new_folder_root, true, window, cx)
                                    })),
                            )
                            .child(
                                tree_action(
                                    "tree-refresh",
                                    "refresh-cw",
                                    "Refresh File Tree",
                                    false,
                                )
                                .on_click(cx.listener(|w, _, _, cx| w.reload_tree(cx))),
                            )
                            .child(
                                tree_action(
                                    "tree-collapse",
                                    "chevrons-down-up",
                                    "Collapse All Folders",
                                    false,
                                )
                                .on_click(cx.listener(
                                    |w, _, _, cx| {
                                        let workspace = w.workspace_mut();
                                        workspace.expanded_dirs.clear();
                                        workspace.save_expanded();
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                tree_action(
                                    "tree-hidden",
                                    if show_hidden { "eye" } else { "eye-off" },
                                    if show_hidden {
                                        "Hide Hidden Files"
                                    } else {
                                        "Show Hidden Files"
                                    },
                                    show_hidden,
                                )
                                .on_click(cx.listener(
                                    |w, _, _, cx| {
                                        SettingsStore::update(cx, |s| {
                                            s.sidebar_show_hidden = !s.sidebar_show_hidden
                                        });
                                        w.reload_tree(cx);
                                    },
                                )),
                            )
                    }),
            )
            .child(match self.panel {
                SidebarPanel::Changes => self.changes_panel(window, cx),
                SidebarPanel::Search => self.search.clone().into_any_element(),
                SidebarPanel::Files => files.into_any_element(),
            })
    }

    pub(crate) fn status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette.clone();
        let workspace = self.workspace();
        let repo = crate::repository_state::RepositoryStore::summary(&workspace.root, cx);
        let mut left: Vec<AnyElement> = Vec::new();
        let mut right: Vec<AnyElement> = Vec::new();
        // The newest piece of language-server work still going: after the
        // cursor items, before the language or shell (Swift's order).
        let mut progress = self.lsp_progress.last().map(|(_, status)| {
            let help = std::iter::once(status.title.as_str())
                .chain(status.message.as_deref())
                .collect::<Vec<_>>()
                .join(" · ");
            status_item(
                "status-lsp-progress",
                None,
                &format!("{}: {help}", status.server),
                None,
                &p,
            )
            .child(progress_ring(
                status.fraction,
                p.text_secondary,
                10.,
                1.4,
                0.,
            ))
            .child(
                div()
                    .whitespace_nowrap()
                    .max_w(px(220.))
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(status.title.clone()),
            )
            .children(
                status
                    .fraction
                    .map(|f| format!("{}%", (f * 100.).round() as u32)),
            )
            .into_any_element()
        });
        if self.is_restricted() {
            left.push(
                status_item(
                    "restricted",
                    Some("shield-alert"),
                    "Restricted: language servers, formatters on save and background fetch are off in this folder. Click to trust it.",
                    Some(p.warning),
                    &p,
                )
                .on_click(cx.listener(|w, _, window, cx| w.run_command("trust_folder", window, cx)))
                .child("Restricted")
                .into_any_element(),
            );
        }
        if !workspace.branch.is_empty() {
            left.push(
                status_item(
                    "status-branch",
                    Some("git-branch"),
                    "Switch branch",
                    None,
                    &p,
                )
                .on_click(
                    cx.listener(|w, _, window, cx| w.run_command("switch_branch", window, cx)),
                )
                .child(workspace.branch.clone())
                .into_any_element(),
            );
        }
        if let Some(repo) = repo.filter(|r| r.changed > 0) {
            left.push(
                status_item(
                    "status-changes",
                    Some("file-diff"),
                    "Review changes",
                    None,
                    &p,
                )
                .on_click(
                    cx.listener(|w, _, window, cx| w.run_command("review_changes", window, cx)),
                )
                .child(repo.changed.to_string())
                .child(diff_stat(repo.added, repo.removed, &p))
                .into_any_element(),
            );
        }
        left.extend(self.problems_status(cx));
        for port in workspace.ports.iter().take(3) {
            let number = port.port;
            left.push(
                status_item(
                    ("status-port", number as usize),
                    Some("globe"),
                    &format!(
                        "{} is listening on {number}: open in the browser",
                        port.process
                    ),
                    None,
                    &p,
                )
                .on_click(move |_, _, cx| cx.open_url(&format!("http://localhost:{number}")))
                .child(format!(":{number}"))
                .into_any_element(),
            );
        }
        if workspace.ports.len() > 3 {
            let help = workspace.ports[3..]
                .iter()
                .map(|p| format!(":{} {}", p.port, p.process))
                .collect::<Vec<_>>()
                .join("\n");
            left.push(
                status_item("status-more-ports", None, &help, None, &p)
                    .child(format!("+{}", workspace.ports.len() - 3))
                    .into_any_element(),
            );
        }
        match self.focused_surface() {
            Some(Surface::Editor(editor)) => {
                let editor = editor.read(cx);
                let (line, column) = editor.cursor(cx);
                if let Some(status) = editor.vim_status(cx) {
                    right.push(
                        div()
                            .font_family("JetBrains Mono")
                            .child(status)
                            .into_any_element(),
                    );
                }
                if let Some(notice) = editor.notice.clone() {
                    right.push(
                        div()
                            .text_color(hsla(p.warning))
                            .child(notice)
                            .into_any_element(),
                    );
                }
                right.push(
                    status_item("status-line", None, "Go to Line", None, &p)
                        .on_click(
                            cx.listener(|w, _, window, cx| w.run_command("go_to_line", window, cx)),
                        )
                        .child(format!("Ln {line}, Col {column}"))
                        .into_any_element(),
                );
                let indent = if editor.use_spaces {
                    format!("Spaces: {}", editor.tab_size)
                } else {
                    format!("Tab Size: {}", editor.tab_size)
                };
                right.push(
                    status_item("status-indent", None, "Indentation", None, &p)
                        .child(indent)
                        .into_any_element(),
                );
                right.push(
                    status_item("status-encoding", None, "Encoding", None, &p)
                        .child("UTF-8")
                        .into_any_element(),
                );
                right.extend(progress.take());
                right.push(
                    status_item("status-language", Some("code"), "Language", None, &p)
                        .child(impulse_kit::uri::editor_language_for_path(&editor.path))
                        .into_any_element(),
                );
                if impulse_kit::markdown_preview::is_previewable_file(
                    &editor.path.to_string_lossy(),
                ) {
                    let previewing = editor.preview.is_some();
                    right.push(
                        status_item(
                            "status-preview",
                            Some(if previewing { "eye-off" } else { "eye" }),
                            if previewing {
                                "Hide preview"
                            } else {
                                "Show preview"
                            },
                            previewing.then_some(p.accent),
                            &p,
                        )
                        .on_click(cx.listener(|w, _, window, cx| {
                            w.run_command("toggle_markdown_preview", window, cx)
                        }))
                        .child("Preview")
                        .into_any_element(),
                    );
                }
            }
            Some(Surface::Terminal(terminal)) => {
                let cwd = terminal.read(cx).cwd.clone();
                if !cwd.is_empty() {
                    left.push(
                        status_item("status-cwd", Some("folder"), &cwd, None, &p)
                            .child(abbreviate_home(&cwd))
                            .into_any_element(),
                    );
                }
                right.extend(progress.take());
                right.push(
                    status_item("status-shell", Some("terminal"), "Shell", None, &p)
                        .child(crate::workspace::shell_name())
                        .into_any_element(),
                );
            }
            // Review, History and tools have no shell or cursor, but the
            // folder still follows the tab (Swift's `currentCwd`).
            Some(surface) => {
                let folder = surface.directory(cx).or_else(|| {
                    (self.workspace().kind == Kind::Folder).then(|| self.workspace().root.clone())
                });
                if let Some(folder) = folder {
                    let cwd = folder.to_string_lossy().into_owned();
                    left.push(
                        status_item("status-cwd", Some("folder"), &cwd, None, &p)
                            .child(abbreviate_home(&cwd))
                            .into_any_element(),
                    );
                }
            }
            None => {}
        }
        right.extend(progress);
        if let Some(notice) = self.notice.clone() {
            left.push(
                div()
                    .px(px(6.))
                    .text_color(hsla(p.warning))
                    .child(notice)
                    .into_any_element(),
            );
        }
        div()
            .flex()
            .items_center()
            .gap(px(2.))
            .h(px(metrics::STATUS_BAR_HEIGHT))
            .px(px(6.))
            .bg(hsla(p.chrome))
            .border_t_1()
            .border_color(hsla(p.hairline))
            .text_size(px(11.))
            .text_color(hsla(p.text_secondary))
            .children(left)
            .child(div().flex_1().min_w(px(8.)))
            .children(self.update_item(cx))
            .children(right)
    }

    /// "Update x.y.z" when a newer Impulse is out, opening its page.
    fn update_item(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let update = cx
            .try_global::<crate::update_check::AvailableUpdate>()?
            .0
            .clone()?;
        let p = &self.palette;
        let help = format!("Impulse {} is available", update.version);
        Some(
            div()
                .id("update")
                .flex()
                .items_center()
                .gap(px(5.))
                .cursor_pointer()
                .text_color(hsla(p.success))
                .on_click(move |_, _, cx| cx.open_url(&update.url))
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(help.clone()).build(window, cx)
                })
                .child(icon("arrow-down", metrics::ICON_SMALL, p.success))
                .child(format!("Update {}", update.version))
                .into_any_element(),
        )
    }
}

/// A titlebar tab being dragged to a new place.
#[derive(Clone)]
pub(crate) struct DraggedTab {
    index: usize,
    title: String,
}

/// AppKit's `secondaryLabelColor` (SwiftUI's `.secondary`), for views that
/// use it rather than the chrome's text colors.
pub(crate) fn system_secondary(light: bool) -> KitRgba {
    if light {
        KitRgba::BLACK.opacity(0.5)
    } else {
        KitRgba::WHITE.opacity(0.55)
    }
}

/// AppKit's `tertiaryLabelColor` (SwiftUI's `.tertiary`).
pub(crate) fn system_tertiary(light: bool) -> KitRgba {
    if light {
        KitRgba::BLACK.opacity(0.26)
    } else {
        KitRgba::WHITE.opacity(0.25)
    }
}

/// The kinds of Swift's `ChromeButton`.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ButtonKind {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

/// Swift's `ChromeButton`: 22 high, 11.5 medium, the kind's colors; the
/// caller adds its click when it isn't `disabled`.
pub(crate) fn chrome_button(
    id: impl Into<gpui_kit::ElementId>,
    title: impl Into<gpui_kit::SharedString>,
    icon_name: Option<&str>,
    kind: ButtonKind,
    disabled: bool,
    p: &impulse_kit::chrome::ChromePalette,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let title: gpui_kit::SharedString = title.into();
    let (fg, bg, hover) = match kind {
        ButtonKind::Primary => (p.text_on_accent, Some(p.accent), p.accent.opacity(0.88)),
        ButtonKind::Secondary => (p.text, Some(p.raised), p.raised.opacity(0.9)),
        ButtonKind::Ghost => (p.text, None, p.hover),
        ButtonKind::Danger => (p.danger, None, p.hover),
    };
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.))
        .px(px(9.))
        .h(px(22.))
        .rounded(px(metrics::RADIUS - 1.))
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(hsla(fg))
        .when_some(bg, |d, bg| d.bg(hsla(bg)))
        .when(kind == ButtonKind::Secondary, |d| {
            d.border_1().border_color(hsla(p.hairline_strong))
        })
        .when(disabled, |d| d.opacity(0.45))
        .when(!disabled, |d| {
            d.cursor_pointer().hover(|s| s.bg(hsla(hover)))
        })
        .children(icon_name.map(|name| icon(name, metrics::ICON_SMALL, fg)))
        .role(gpui_kit::Role::Button)
        .aria_label(title.clone())
        .child(div().whitespace_nowrap().child(title))
}

impl Workbench {
    /// The row's + on hover (Swift's `newWorkspaceItems`): a task from its
    /// repository, named for the main checkout, or another folder.
    fn new_workspace_button(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::button::ButtonVariants as _;
        use gpui_kit::component::menu::DropdownMenu as _;
        use gpui_kit::component::Sizable as _;
        let workspace = &self.workspaces[index];
        let repository = workspace.task.common_dir.is_some().then(|| {
            let main = crate::tasks::main_checkout_root(&workspace.root.to_string_lossy());
            Path::new(&main)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| workspace.name())
        });
        let (id, workbench) = (workspace.id, cx.entity().downgrade());
        div()
            .flex_none()
            .invisible()
            .group_hover("workspace", |s| s.visible())
            .child(
                gpui_kit::component::button::Button::new(("workspace-new", id as usize))
                    .ghost()
                    .xsmall()
                    .icon(gpui_kit::component::Icon::empty().path("icons/plus.svg"))
                    .dropdown_menu(move |menu: PopupMenu, _, _| {
                        let mut menu = menu;
                        if let Some(repository) = &repository {
                            let workbench = workbench.clone();
                            menu = menu.item(
                                PopupMenuItem::new(format!("New Task from {repository}…"))
                                    .on_click(move |_, window, cx| {
                                        let _ = workbench.update(cx, |w, cx| {
                                            let index =
                                                w.workspaces.iter().position(|ws| ws.id == id);
                                            w.present_new_task(index, false, window, cx)
                                        });
                                    }),
                            );
                        }
                        let workbench = workbench.clone();
                        menu.item(PopupMenuItem::new("Open Folder as Workspace…").on_click(
                            move |_, window, cx| {
                                let _ = workbench.update(cx, |w, cx| {
                                    w.run_command("open_workspace", window, cx)
                                });
                            },
                        ))
                    }),
            )
    }
}

/// The sidebar's width while dragging its edge, and the limits Swift's
/// `WorkbenchView.leftDockRange` keeps it to.
const SIDEBAR_WIDTHS: std::ops::RangeInclusive<f32> = 180.0..=520.0;
/// Swift's `DockDivider.hitThickness`.
const DIVIDER_GRIP: f32 = 7.;

impl Workbench {
    /// The sidebar's width: the one being dragged, else the setting.
    pub(crate) fn sidebar_width(&self, cx: &gpui_kit::App) -> f32 {
        let width = self
            .sidebar_drag
            .map(|d| d.width)
            .unwrap_or(SettingsStore::get(cx).sidebar_width as f32);
        width.clamp(*SIDEBAR_WIDTHS.start(), *SIDEBAR_WIDTHS.end())
    }

    /// The sidebar's edge (Swift's `DockDivider`): drag to resize, double-
    /// click for the default width; the accent line while hovered or
    /// dragged. Over the content's edge too, so it's drawn last.
    fn sidebar_divider(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &self.palette;
        let dragging = self.sidebar_drag.is_some();
        let line = |d: gpui_kit::Div| {
            d.absolute()
                .top_0()
                .bottom_0()
                .left(px(DIVIDER_GRIP / 2. - 1.))
                .w(px(2.))
                .bg(hsla(p.accent.opacity(0.8)))
        };
        gpui_kit::deferred(
            div()
                .id("sidebar-divider")
                .group("sidebar-divider")
                .absolute()
                .top_0()
                .bottom_0()
                .right(px(-DIVIDER_GRIP / 2. - 0.5))
                .w(px(DIVIDER_GRIP))
                .cursor_col_resize()
                .on_mouse_down(
                    gpui_kit::MouseButton::Left,
                    cx.listener(|w, event: &gpui_kit::MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        if event.click_count == 2 {
                            w.sidebar_drag = None;
                            SettingsStore::update(cx, |s| {
                                s.sidebar_width = metrics::LEFT_DOCK_DEFAULT_WIDTH as i64
                            });
                            return;
                        }
                        w.sidebar_drag = Some(SidebarDrag {
                            width: w.sidebar_width(cx),
                            start_width: w.sidebar_width(cx),
                            start_x: f32::from(event.position.x),
                        });
                        cx.notify();
                    }),
                )
                .child(line(div()).when(!dragging, |d| {
                    d.invisible()
                        .group_hover("sidebar-divider", |s| s.visible())
                })),
        )
        .with_priority(1)
    }

    /// The pointer moved while the sidebar's edge is held.
    pub(crate) fn drag_sidebar(
        &mut self,
        event: &gpui_kit::MouseMoveEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(drag) = self.sidebar_drag.as_mut() else {
            return false;
        };
        if event.pressed_button != Some(gpui_kit::MouseButton::Left) {
            return self.end_sidebar_drag(cx);
        }
        drag.width = (drag.start_width + f32::from(event.position.x) - drag.start_x)
            .clamp(*SIDEBAR_WIDTHS.start(), *SIDEBAR_WIDTHS.end());
        cx.notify();
        true
    }

    /// Let go of the sidebar's edge: its width becomes the setting.
    pub(crate) fn end_sidebar_drag(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(drag) = self.sidebar_drag.take() else {
            return false;
        };
        let width = drag.width.round() as i64;
        SettingsStore::update(cx, |s| s.sidebar_width = width);
        cx.notify();
        true
    }
}

impl Workbench {
    /// The Workspaces section's height kept between nothing and the dock
    /// less the Files panel's 120 (Swift's `heightLimit`).
    fn clamped_workspaces_height(&self, height: f32, window: &Window) -> f32 {
        let dock = f32::from(window.viewport_size().height)
            - metrics::TITLEBAR_HEIGHT
            - metrics::STATUS_BAR_HEIGHT;
        height.clamp(0., (dock - 120.).max(0.))
    }

    /// The hairline under the workspaces, with a taller grab band (Swift's
    /// `SectionResizeHandle`): drag to resize, double-click to fit again;
    /// accent while hovered or dragged.
    fn workspaces_handle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &self.palette;
        let dragging = self.workspaces_drag.is_some();
        div()
            .relative()
            .flex_none()
            .h(px(1.))
            .group("workspaces-handle")
            .bg(hsla(p.hairline))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(-0.5))
                    .h(px(2.))
                    .bg(hsla(p.accent.opacity(0.8)))
                    .when(!dragging, |d| {
                        d.invisible()
                            .group_hover("workspaces-handle", |s| s.visible())
                    }),
            )
            .child(
                div()
                    .id("workspaces-handle")
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(-(DIVIDER_GRIP / 2.)))
                    .h(px(DIVIDER_GRIP))
                    .cursor_row_resize()
                    .on_mouse_down(
                        gpui_kit::MouseButton::Left,
                        cx.listener(|w, event: &gpui_kit::MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            if event.click_count == 2 {
                                w.workspaces_height = None;
                                w.workspaces_drag = None;
                            } else {
                                let height =
                                    w.workspaces_height.unwrap_or(w.workspaces_measured.get());
                                w.workspaces_drag = Some((height, f32::from(event.position.y)));
                            }
                            cx.notify();
                        }),
                    ),
            )
    }

    /// The pointer moved while the Workspaces section's handle is held.
    pub(crate) fn drag_workspaces(
        &mut self,
        event: &gpui_kit::MouseMoveEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((height, y)) = self.workspaces_drag else {
            return false;
        };
        if event.pressed_button != Some(gpui_kit::MouseButton::Left) {
            self.workspaces_drag = None;
            return true;
        }
        let wanted = height + f32::from(event.position.y) - y;
        self.workspaces_height = Some(self.clamped_workspaces_height(wanted, window));
        cx.notify();
        true
    }
}

/// The sidebar's edge being dragged.
#[derive(Clone, Copy)]
pub(crate) struct SidebarDrag {
    width: f32,
    start_width: f32,
    start_x: f32,
}

/// Swift's `ChromeIconButton`: a 24-point square, the icon in the
/// secondary text color, the hover tint and `help` as its tooltip; the
/// caller adds its click.
pub(crate) fn chrome_icon_button(
    id: impl Into<gpui_kit::ElementId>,
    icon_name: &str,
    help: impl Into<gpui_kit::SharedString>,
    p: &impulse_kit::chrome::ChromePalette,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let help = help.into();
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(24.))
        .rounded(px(metrics::RADIUS_SMALL + 1.))
        .cursor_pointer()
        .hover(|s| s.bg(hsla(p.hover)))
        .role(gpui_kit::Role::Button)
        .aria_label(help.clone())
        .tooltip(move |window, cx| {
            gpui_kit::component::tooltip::Tooltip::new(help.clone()).build(window, cx)
        })
        .child(icon(icon_name, metrics::ICON, p.text_secondary))
}

/// The banner under the titlebar while settings.json doesn't load (Swift's
/// `WorkbenchBanner`), until it loads again or is dismissed.
impl Workbench {
    pub(crate) fn settings_banner(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let (serial, warning) = SettingsStore::warning(cx)?;
        if self.dismissed_settings_warning == Some(serial) {
            return None;
        }
        let p = &self.palette;
        let mut detail = if warning.using_defaults {
            "Using the default settings; changes aren't saved until the file is fixed.".to_string()
        } else {
            "Keeping the settings Impulse already had; changes aren't saved until the file is fixed."
                .to_string()
        };
        if let Some(backup) = &warning.backup_path {
            detail.push_str(&format!(
                " A copy of the broken file is at {}.",
                abbreviate_home(&backup.to_string_lossy())
            ));
        }
        let message: gpui_kit::SharedString = warning.message.clone().into();
        Some(
            div()
                .id("settings-banner")
                .flex()
                .flex_none()
                .items_center()
                .gap(px(10.))
                .px(px(12.))
                .py(px(8.))
                .bg(hsla(p.window))
                .child(
                    // The tint over the window's color, as SwiftUI lays it.
                    div()
                        .absolute()
                        .inset_0()
                        .bg(hsla(p.warning.opacity(0.12)))
                        .border_b_1()
                        .border_color(hsla(p.hairline)),
                )
                .relative()
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(message.clone()).build(window, cx)
                })
                .child(icon("triangle-alert", 14., p.warning))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(hsla(p.text))
                                .child("Settings file could not be loaded"),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(hsla(p.text_secondary))
                                .line_clamp(2)
                                .child(detail),
                        ),
                )
                .child(
                    chrome_button(
                        "settings-banner-open",
                        "Open Settings File",
                        Some("external-link"),
                        ButtonKind::Secondary,
                        false,
                        p,
                    )
                    .on_click(cx.listener(|w, _, window, cx| {
                        w.run_command("open_settings_json", window, cx)
                    })),
                )
                .child(
                    chrome_icon_button("settings-banner-dismiss", "x", "Dismiss", p).on_click(
                        cx.listener(move |w, _, _, cx| {
                            w.dismissed_settings_warning = Some(serial);
                            cx.notify();
                        }),
                    ),
                ),
        )
    }
}

/// SwiftUI's `.checkbox` toggle (an `NSButton` checkbox): a 16-point box,
/// rounded 4, in the system blue with a white check when on (the sheets
/// don't tint it), its label (12, or as `label_style` says) 6.5 after.
/// `small` is the 12-point control. The builder names are GPUI Kit's
/// `Checkbox`'s.
#[derive(IntoElement)]
pub(crate) struct ChromeCheckbox {
    id: gpui_kit::ElementId,
    label: Option<gpui_kit::SharedString>,
    checked: bool,
    disabled: bool,
    small: bool,
    label_size: f32,
    label_color: Option<KitRgba>,
    mono: bool,
    on_change: Option<CheckboxHandler>,
}

/// What a checkbox click asks for, told to its owner.
type CheckboxHandler = Rc<dyn Fn(&bool, &mut Window, &mut gpui_kit::App)>;

impl ChromeCheckbox {
    pub(crate) fn new(id: impl Into<gpui_kit::ElementId>) -> Self {
        ChromeCheckbox {
            id: id.into(),
            label: None,
            checked: false,
            disabled: false,
            small: false,
            label_size: 12.,
            label_color: None,
            mono: false,
            on_change: None,
        }
    }

    /// The label's size and color (it inherits the color otherwise).
    pub(crate) fn label_style(mut self, size: f32, color: KitRgba) -> Self {
        self.label_size = size;
        self.label_color = Some(color);
        self
    }

    /// The label in the monospaced font (a path).
    pub(crate) fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    pub(crate) fn label(mut self, label: impl Into<gpui_kit::SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub(crate) fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub(crate) fn small(mut self) -> Self {
        self.small = true;
        self
    }

    /// Hears the value a click asks for.
    pub(crate) fn on_change(
        mut self,
        handler: impl Fn(&bool, &mut Window, &mut gpui_kit::App) + 'static,
    ) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }

    pub(crate) fn on_click(
        self,
        handler: impl Fn(&bool, &mut Window, &mut gpui_kit::App) + 'static,
    ) -> Self {
        self.on_change(handler)
    }
}

impl RenderOnce for ChromeCheckbox {
    fn render(self, _: &mut Window, cx: &mut gpui_kit::App) -> impl IntoElement {
        let light = crate::alert::is_light(cx);
        let ink = if light {
            KitRgba::BLACK
        } else {
            KitRgba::WHITE
        };
        let (size, radius, gap, mark) = if self.small {
            (12., 3., 5., 9.)
        } else {
            (16., 4., 6.5, 12.)
        };
        let checked = self.checked;
        let handler = self.on_change.filter(|_| !self.disabled);
        div()
            .id(self.id)
            .flex()
            .items_start()
            .gap(px(gap))
            .when(self.disabled, |d| d.opacity(0.5))
            .when_some(handler, |d, handler| {
                d.cursor_pointer()
                    .on_click(move |_, window, cx| handler(&!checked, window, cx))
            })
            .child(
                div()
                    .flex_none()
                    .size(px(size))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(radius))
                    .bg(if checked {
                        hsla(KitRgba::from_hex(if light { "#007AFF" } else { "#0A84FF" }))
                    } else {
                        hsla(ink.opacity(0.1))
                    })
                    .when(!checked, |d| {
                        d.border_1().border_color(hsla(ink.opacity(0.12)))
                    })
                    .when(checked, |d| d.child(icon("check", mark, KitRgba::WHITE))),
            )
            .children(self.label.map(|label| {
                div()
                    .min_w_0()
                    .text_size(px(self.label_size))
                    .when_some(self.label_color, |d, c| d.text_color(hsla(c)))
                    .when(self.mono, |d| d.font_family("JetBrains Mono"))
                    .child(label)
            }))
    }
}
