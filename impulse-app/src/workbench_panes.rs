//! A tab's panes, as `PaneLayoutView` lays them out: the `LayoutTree`'s
//! splits side by side with 1pt hairline gaps that can be dragged (a
//! double-click evens out the split), inactive panes dimmed, a 2pt accent
//! bar on the focused one, and a zoomed pane shown alone. Clicking in a
//! pane focuses it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::{
    canvas, div, prelude::*, px, relative, AnyElement, Bounds, Context, MouseButton,
    MouseMoveEvent, MouseUpEvent, Pixels, Window,
};
use impulse_kit::layout_tree::{LayoutTree, PaneDirection, SplitAxis};

use crate::workbench::Workbench;
use crate::workbench_chrome::hsla;
use crate::workspace::{PaneId, Surface, Tab};

/// The 1pt gap between panes.
const GAP: f32 = 1.0;
/// Panes can't be dragged narrower or shorter than this.
const MINIMUM_PANE: f32 = 80.0;
/// How far one Grow Pane moves an edge, as a share of the split.
const RESIZE_STEP: f64 = 0.05;

/// Where each split of each tab was drawn, for turning a drag into a ratio.
pub type SplitBounds = Rc<RefCell<HashMap<(u64, Vec<usize>), Bounds<Pixels>>>>;

/// A divider being dragged.
#[derive(Clone)]
pub struct DividerDrag {
    tab: u64,
    path: Vec<usize>,
    index: usize,
    axis: SplitAxis,
}

impl Workbench {
    /// The active tab's panes.
    pub(crate) fn content(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(tab) = self.workspace().active_tab() else {
            return div().flex_1().into_any_element();
        };
        let body = match tab.zoomed.filter(|z| tab.panes.contains_key(z)) {
            Some(zoomed) => self.pane(tab, zoomed, false, window, cx),
            None => self.layout_node(tab, &tab.layout, Vec::new(), window, cx),
        };
        div()
            .flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(hsla(self.palette.hairline))
            .child(body)
            .into_any_element()
    }

    fn layout_node(
        &self,
        tab: &Tab,
        node: &LayoutTree<PaneId>,
        path: Vec<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match node {
            LayoutTree::Leaf(id) => self.pane(tab, *id, tab.is_split(), window, cx),
            LayoutTree::Split {
                axis,
                children,
                ratios,
            } => {
                let horizontal = *axis == SplitAxis::Horizontal;
                let bounds = self.split_bounds.clone();
                let key = (tab.id, path.clone());
                let mut row = div()
                    .flex()
                    .size_full()
                    .relative()
                    .when(horizontal, |d| d.flex_row())
                    .when(!horizontal, |d| d.flex_col())
                    .child(
                        canvas(
                            move |b, _, _| {
                                bounds.borrow_mut().insert(key, b);
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    );
                for (index, (child, ratio)) in children.iter().zip(ratios).enumerate() {
                    if index > 0 {
                        row = row.child(self.divider(tab.id, path.clone(), index - 1, *axis, cx));
                    }
                    let mut child_path = path.clone();
                    child_path.push(index);
                    row = row.child(
                        div()
                            .flex()
                            .flex_basis(relative(*ratio as f32))
                            .flex_grow(0.)
                            .flex_shrink(1.)
                            .min_w_0()
                            .min_h_0()
                            .overflow_hidden()
                            .child(self.layout_node(tab, child, child_path, window, cx)),
                    );
                }
                row.into_any_element()
            }
        }
    }

    /// The hairline between two panes; dragging moves it, a double-click
    /// evens out its split.
    fn divider(
        &self,
        tab: u64,
        path: Vec<usize>,
        index: usize,
        axis: SplitAxis,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let horizontal = axis == SplitAxis::Horizontal;
        let drag = DividerDrag {
            tab,
            path: path.clone(),
            index,
            axis,
        };
        div()
            .flex_none()
            .relative()
            .when(horizontal, |d| d.w(px(GAP)).h_full())
            .when(!horizontal, |d| d.h(px(GAP)).w_full())
            .child(
                // A wider grip than the line itself.
                div()
                    .id(gpui_kit::ElementId::Name(
                        format!("divider-{tab}-{path:?}-{index}").into(),
                    ))
                    .absolute()
                    .when(horizontal, |d| {
                        d.top_0()
                            .bottom_0()
                            .left(px(-3.))
                            .w(px(GAP + 6.))
                            .cursor_col_resize()
                    })
                    .when(!horizontal, |d| {
                        d.left_0()
                            .right_0()
                            .top(px(-3.))
                            .h(px(GAP + 6.))
                            .cursor_row_resize()
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |workbench, e: &gpui_kit::MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            if e.click_count >= 2 {
                                workbench.even_out(drag.tab, &drag.path, cx);
                            } else {
                                workbench.drag = Some(drag.clone());
                            }
                        }),
                    ),
            )
            .into_any_element()
    }

    /// One pane: its surface, dimmed when another pane of the split has
    /// focus, with the accent bar when it has it.
    fn pane(
        &self,
        tab: &Tab,
        id: PaneId,
        split: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(surface) = tab.panes.get(&id) else {
            return div().into_any_element();
        };
        let focused = tab.focused == id;
        let p = &self.palette;
        let dim = if p.is_light { 0.28 } else { 0.34 };
        let tab_id = tab.id;
        let body: AnyElement = match surface {
            Surface::Terminal(t) => t.clone().into_any_element(),
            Surface::Editor(e) => e.clone().into_any_element(),
            Surface::Review(r) => r.clone().into_any_element(),
            Surface::History(h) => h.clone().into_any_element(),
            Surface::Settings(s) => s.clone().into_any_element(),
            Surface::Keybindings(k) => k.clone().into_any_element(),
        };
        div()
            .relative()
            .size_full()
            .bg(hsla(p.content))
            .on_any_mouse_down(cx.listener(move |workbench, _, _, cx| {
                workbench.note_pane_focus(tab_id, id, cx);
            }))
            .child(body)
            .when(split && !focused, |d| {
                d.child(div().absolute().inset_0().bg(hsla(p.chrome.opacity(dim))))
            })
            .when(split && focused, |d| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(px(2.))
                        .bg(hsla(p.accent)),
                )
            })
            .into_any_element()
    }

    /// A click landed in pane `id` of tab `tab`.
    fn note_pane_focus(&mut self, tab: u64, id: PaneId, cx: &mut Context<Self>) {
        if let Some(t) = self.workspace_mut().tabs.iter_mut().find(|t| t.id == tab) {
            if t.focused != id {
                t.focused = id;
                cx.notify();
            }
        }
    }

    pub(crate) fn drag_divider(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.drag.clone() else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag = None;
            return;
        }
        let Some(container) = self
            .split_bounds
            .borrow()
            .get(&(drag.tab, drag.path.clone()))
            .copied()
        else {
            return;
        };
        let Some(tab) = self
            .workspace_mut()
            .tabs
            .iter_mut()
            .find(|t| t.id == drag.tab)
        else {
            return;
        };
        let children = match tab.layout.node(&drag.path) {
            Some(LayoutTree::Split { children, .. }) => children.len(),
            _ => return,
        };
        let horizontal = drag.axis == SplitAxis::Horizontal;
        let (length, origin, at) = if horizontal {
            (container.size.width, container.origin.x, event.position.x)
        } else {
            (container.size.height, container.origin.y, event.position.y)
        };
        let available = (f32::from(length) - GAP * (children as f32 - 1.0)).max(1.0);
        let position = (f32::from(at - origin) - drag.index as f32 * GAP) as f64 / available as f64;
        let minimum = (MINIMUM_PANE / available).min(0.45) as f64;
        let updated = tab
            .layout
            .moving_divider(&drag.path, drag.index, position, Some(minimum));
        if updated != tab.layout {
            tab.layout = updated;
            cx.notify();
        }
    }

    pub(crate) fn end_drag(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.drag = None;
    }

    /// Double-click on a divider: every child of that split the same size.
    fn even_out(&mut self, tab: u64, path: &[usize], cx: &mut Context<Self>) {
        let Some(t) = self.workspace_mut().tabs.iter_mut().find(|t| t.id == tab) else {
            return;
        };
        let Some(LayoutTree::Split { ratios, .. }) = t.layout.node(path).map(|n| n.equalized())
        else {
            return;
        };
        let mut boundary = 0.0;
        for (index, ratio) in ratios.iter().take(ratios.len() - 1).enumerate() {
            boundary += ratio;
            t.layout = t.layout.moving_divider(path, index, boundary, Some(0.0));
        }
        cx.notify();
    }

    // MARK: Pane commands

    pub(crate) fn split_focused(
        &mut self,
        surface: Surface,
        axis: SplitAxis,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane = self.next_id();
        let Some(tab) = self.workspace_mut().active_tab_mut() else {
            return;
        };
        tab.split(pane, surface, axis, false);
        self.focus_active(window, cx);
        cx.notify();
    }

    pub(crate) fn focus_neighbor(
        &mut self,
        direction: PaneDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.workspace_mut().active_tab_mut() else {
            return;
        };
        if let Some(next) = tab.layout.neighbor(&tab.focused, direction) {
            tab.focused = next;
            tab.zoomed = None;
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    pub(crate) fn cycle_pane(&mut self, step: i32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.workspace_mut().active_tab_mut() else {
            return;
        };
        let leaves = tab.layout.leaves();
        let Some(index) = leaves.iter().position(|id| *id == tab.focused) else {
            return;
        };
        let count = leaves.len() as i32;
        tab.focused = leaves[((index as i32 + step).rem_euclid(count)) as usize];
        tab.zoomed = None;
        self.focus_active(window, cx);
        cx.notify();
    }

    pub(crate) fn resize_focused(&mut self, direction: PaneDirection, cx: &mut Context<Self>) {
        let Some(tab) = self.workspace_mut().active_tab_mut() else {
            return;
        };
        tab.layout = tab.layout.resizing(&tab.focused, direction, RESIZE_STEP);
        cx.notify();
    }

    pub(crate) fn toggle_zoom(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.workspace_mut().active_tab_mut() else {
            return;
        };
        if tab.is_split() {
            tab.zoomed = if tab.zoomed.is_some() {
                None
            } else {
                Some(tab.focused)
            };
            cx.notify();
        }
    }

    pub(crate) fn equalize(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.workspace_mut().active_tab_mut() {
            tab.layout = tab.layout.equalized();
            cx.notify();
        }
    }

    /// The focused pane of a split into a tab of its own.
    pub(crate) fn move_pane_to_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.workspace_mut().active_tab_mut() else {
            return;
        };
        if !tab.is_split() {
            return;
        }
        let pane = tab.focused;
        let surface = tab.panes[&pane].clone();
        tab.remove(pane);
        let active = self.active;
        self.add_tab(active, surface, window, cx);
    }
}
