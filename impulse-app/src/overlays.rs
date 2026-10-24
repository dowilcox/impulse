//! Dialogs open through `open_dialog` here rather than GPUI Kit's directly,
//! so debug builds can tell the headless snapshots where a dialog is on
//! screen (`probe`; the docs shots crop to it).

use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{AnyElement, App, IntoElement as _, ParentElement as _, Window};

/// GPUI Kit's `open_dialog`, with the probe in the dialog's body.
pub(crate) fn open_dialog<F>(window: &mut Window, cx: &mut App, build: F)
where
    F: Fn(Dialog, &mut Window, &mut App) -> Dialog + 'static,
{
    window.open_dialog(cx, move |dialog, window, cx| {
        build(dialog, window, cx).child(probe())
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
