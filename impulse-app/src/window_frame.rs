//! The window's frame: on Linux, where Impulse draws it itself (client-side
//! decorations, asked for in `open_main_window`), the titlebar's empty
//! stretch moves the window, zooms it on a double click and opens the
//! window menu on a right click, and minimize, maximize and close sit at
//! its right end, as far as the window manager supports them. On macOS
//! AppKit draws the traffic lights, but the app owns the titlebar's drag
//! (`app_owns_titlebar_drag`), so tabs can be dragged and double-clicked
//! without moving or zooming the window: the empty stretches move it and a
//! double click does what the system setting says. When a Linux window
//! manager decorates the window itself, these stay out of its way.

use std::cell::Cell;

use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::{
    div, prelude::*, px, AnyElement, CursorStyle, Decorations, MouseButton, ResizeEdge, Window,
};
use impulse_kit::chrome::ChromePalette;

use crate::workbench_chrome::hsla;

thread_local! {
    /// A press on the drag region that hasn't moved yet.
    static PRESSED: Cell<bool> = const { Cell::new(false) };
}

/// Whether Impulse draws the frame (Linux, client-side decorated).
pub(crate) fn is_ours(window: &Window) -> bool {
    cfg!(target_os = "linux") && matches!(window.window_decorations(), Decorations::Client { .. })
}

/// Whether the titlebar's empty stretches move the window: always on
/// macOS, on Linux with our own frame.
fn drags(window: &Window) -> bool {
    cfg!(target_os = "macos") || is_ours(window)
}

/// The titlebar's empty stretch, filling what's left: a drag moves the
/// window and a double click zooms it (on the Mac as the system setting
/// says); on Linux with our own frame a right click opens the window
/// manager's menu.
pub(crate) fn drag_region(window: &Window) -> gpui_kit::Stateful<gpui_kit::Div> {
    drag_area("titlebar-drag", window).flex_1().h_full()
}

/// An empty part of the titlebar that moves the window, sized by the caller.
pub(crate) fn drag_area(
    id: impl Into<gpui_kit::ElementId>,
    window: &Window,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let region = div().id(id);
    if !drags(window) {
        return region;
    }
    let region = region
        .on_mouse_down(MouseButton::Left, |_, _, _| PRESSED.set(true))
        .on_mouse_up(MouseButton::Left, |_, _, _| PRESSED.set(false))
        .on_mouse_move(|_, window, _| {
            if PRESSED.replace(false) {
                window.start_window_move();
            }
        })
        .on_click(|event, window, _| {
            if event.click_count() >= 2 {
                if cfg!(target_os = "macos") {
                    window.titlebar_double_click();
                } else {
                    window.zoom_window();
                }
            }
        });
    if cfg!(target_os = "macos") {
        return region;
    }
    region.on_mouse_down(MouseButton::Right, |event, window, _| {
        window.show_window_menu(event.position)
    })
}

/// Minimize, maximize (or restore) and close, when the frame is ours.
pub(crate) fn controls(window: &Window, p: &ChromePalette) -> Option<AnyElement> {
    if !is_ours(window) {
        return None;
    }
    let supported = window.window_controls();
    let button = |id: &'static str, name: IconName, danger: bool| {
        let (hover, pressed) = if danger {
            (hsla(p.danger), hsla(p.danger.opacity(0.85)))
        } else {
            (hsla(p.hover), hsla(p.pressed))
        };
        div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(24.))
            .rounded_full()
            .text_color(hsla(p.text_secondary))
            .hover(move |s| s.bg(hover))
            .active(move |s| s.bg(pressed))
            // Not a drag of the window.
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .child(Icon::new(name).small())
    };
    Some(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .ml(px(8.))
            .when(supported.minimize, |d| {
                d.child(
                    button("window-minimize", IconName::WindowMinimize, false)
                        .on_click(|_, window, _| window.minimize_window()),
                )
            })
            .when(supported.maximize, |d| {
                let name = if window.is_maximized() {
                    IconName::WindowRestore
                } else {
                    IconName::WindowMaximize
                };
                d.child(
                    button("window-maximize", name, false)
                        .on_click(|_, window, _| window.zoom_window()),
                )
            })
            .child(
                // Closing goes through the same questions as ⌘W on the window.
                button("window-close", IconName::WindowClose, true).on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(crate::menus::CloseWindow), cx)
                }),
            )
            .into_any_element(),
    )
}

/// How far in from the window's edge a press resizes it, and how far along
/// it the corners reach.
const EDGE: f32 = 5.;
const CORNER: f32 = 10.;

/// The window's edges and corners, which resize it when the frame is ours
/// (no window manager border to grab), except edges the window manager has
/// tiled and while it's maximized.
pub(crate) fn resize_edges(window: &Window) -> Option<AnyElement> {
    let Decorations::Client { tiling } = window.window_decorations() else {
        return None;
    };
    if !cfg!(target_os = "linux") || window.is_maximized() || window.is_fullscreen() {
        return None;
    }
    let handle = |id: &'static str, edge: ResizeEdge, cursor: CursorStyle| {
        div()
            .id(id)
            .absolute()
            .cursor(cursor)
            .occlude()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                window.start_window_resize(edge);
                cx.stop_propagation();
            })
    };
    let (top, bottom, left, right) = (!tiling.top, !tiling.bottom, !tiling.left, !tiling.right);
    let mut layer = div().absolute().inset_0();
    if top {
        layer = layer.child(
            handle("resize-top", ResizeEdge::Top, CursorStyle::ResizeUpDown)
                .top_0()
                .left(px(CORNER))
                .right(px(CORNER))
                .h(px(EDGE)),
        );
    }
    if bottom {
        layer = layer.child(
            handle(
                "resize-bottom",
                ResizeEdge::Bottom,
                CursorStyle::ResizeUpDown,
            )
            .bottom_0()
            .left(px(CORNER))
            .right(px(CORNER))
            .h(px(EDGE)),
        );
    }
    if left {
        layer = layer.child(
            handle(
                "resize-left",
                ResizeEdge::Left,
                CursorStyle::ResizeLeftRight,
            )
            .left_0()
            .top(px(CORNER))
            .bottom(px(CORNER))
            .w(px(EDGE)),
        );
    }
    if right {
        layer = layer.child(
            handle(
                "resize-right",
                ResizeEdge::Right,
                CursorStyle::ResizeLeftRight,
            )
            .right_0()
            .top(px(CORNER))
            .bottom(px(CORNER))
            .w(px(EDGE)),
        );
    }
    let corner = |id, edge, cursor| handle(id, edge, cursor).size(px(CORNER));
    if top && left {
        layer = layer.child(
            corner(
                "resize-top-left",
                ResizeEdge::TopLeft,
                CursorStyle::ResizeUpLeftDownRight,
            )
            .top_0()
            .left_0(),
        );
    }
    if top && right {
        layer = layer.child(
            corner(
                "resize-top-right",
                ResizeEdge::TopRight,
                CursorStyle::ResizeUpRightDownLeft,
            )
            .top_0()
            .right_0(),
        );
    }
    if bottom && left {
        layer = layer.child(
            corner(
                "resize-bottom-left",
                ResizeEdge::BottomLeft,
                CursorStyle::ResizeUpRightDownLeft,
            )
            .bottom_0()
            .left_0(),
        );
    }
    if bottom && right {
        layer = layer.child(
            corner(
                "resize-bottom-right",
                ResizeEdge::BottomRight,
                CursorStyle::ResizeUpLeftDownRight,
            )
            .bottom_0()
            .right_0(),
        );
    }
    Some(layer.into_any_element())
}
