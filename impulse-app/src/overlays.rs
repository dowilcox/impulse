//! Dialogs open through `open_dialog` here rather than GPUI Kit's directly,
//! so debug builds can tell the headless snapshots where a dialog is on
//! screen (`probe`, also in the palette and the agents inbox; the docs
//! shots crop to it).

use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{AnyElement, App, IntoElement as _, ParentElement as _, Styled as _, Window};

/// GPUI Kit's `open_dialog`, with the probe in the dialog's body and the
/// UI font's line height (the sheets' SwiftUI text, not GPUI's 1.6).
pub(crate) fn open_dialog<F>(window: &mut Window, cx: &mut App, build: F)
where
    F: Fn(Dialog, &mut Window, &mut App) -> Dialog + 'static,
{
    window.open_dialog(cx, move |dialog, window, cx| {
        build(dialog, window, cx)
            .line_height(gpui_kit::relative(crate::workbench::UI_LINE_HEIGHT))
            .child(probe())
    });
}

/// In debug builds, records where an overlay is painted (fills its
/// relatively positioned parent; `debug_snapshot::overlay_probe`); nothing
/// in release builds.
pub(crate) fn probe() -> AnyElement {
    #[cfg(debug_assertions)]
    return crate::debug_snapshot::overlay_probe().into_any_element();
    #[cfg(not(debug_assertions))]
    return gpui_kit::Empty.into_any_element();
}

/// In debug builds, the first thing a window paints, so the snapshots
/// report only an overlay open in the frame they capture
/// (`debug_snapshot::frame_start`); nothing in release builds.
pub(crate) fn frame_start() -> AnyElement {
    #[cfg(debug_assertions)]
    return crate::debug_snapshot::frame_start().into_any_element();
    #[cfg(not(debug_assertions))]
    return gpui_kit::Empty.into_any_element();
}
