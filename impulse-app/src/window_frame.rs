//! The window's frame on Linux, where Impulse draws it itself (client-side
//! decorations, asked for in `open_main_window`): the titlebar's empty
//! stretch moves the window, zooms it on a double click and opens the
//! window menu on a right click, and minimize, maximize and close sit at
//! its right end, as far as the window manager supports them. On macOS
//! AppKit does all this around the traffic lights, and when a Linux window
//! manager decorates the window itself, these stay out of its way.

use std::cell::Cell;

use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::{div, prelude::*, px, AnyElement, Decorations, MouseButton, Window};
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

/// The titlebar's empty stretch: on Linux with our own frame, a drag moves
/// the window, a double click zooms it and a right click opens the window
/// manager's menu.
pub(crate) fn drag_region(window: &Window) -> gpui_kit::Stateful<gpui_kit::Div> {
    let region = div().id("titlebar-drag").flex_1().h_full();
    if !is_ours(window) {
        return region;
    }
    region
        .on_mouse_down(MouseButton::Left, |_, _, _| PRESSED.set(true))
        .on_mouse_up(MouseButton::Left, |_, _, _| PRESSED.set(false))
        .on_mouse_move(|_, window, _| {
            if PRESSED.replace(false) {
                window.start_window_move();
            }
        })
        .on_mouse_down(MouseButton::Right, |event, window, _| {
            window.show_window_menu(event.position)
        })
        .on_click(|event, window, _| {
            if event.click_count() >= 2 {
                window.zoom_window();
            }
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
