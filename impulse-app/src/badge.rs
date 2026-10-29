//! The app icon's badge (the Swift `DesktopNotifier.setBadge`): how many
//! terminals, in every window, need attention. On macOS it's the Dock
//! tile's badge label; on Linux the Unity LauncherEntry signal that docks
//! and task managers read (KDE's, Dash to Dock, Plank), sent with `gdbus`
//! for the desktop entry `impulse.desktop`. Without `gdbus` nothing shows,
//! and the tabs' dots still say so.

use std::cell::Cell;

use gpui_kit::App;

thread_local! {
    /// The count last shown, so an unchanged one isn't sent again.
    static SHOWN: Cell<usize> = const { Cell::new(0) };
}

/// Count the terminals that need attention and show it, when it changed.
pub fn update(cx: &mut App) {
    // Not a window that closed but whose workbench hasn't been dropped yet.
    let open = cx.windows();
    let count = crate::session::workbenches(cx)
        .iter()
        .filter(|(_, handle)| open.contains(handle))
        .map(|(workbench, _)| {
            workbench
                .read(cx)
                .workspaces
                .iter()
                .flat_map(|w| w.all_surfaces())
                .filter_map(|s| s.terminal())
                .filter(|t| t.read(cx).agent.needs_attention)
                .count()
        })
        .sum::<usize>();
    if SHOWN.with(|shown| shown.replace(count)) != count {
        show(count);
    }
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
fn show(count: usize) {
    use objc::runtime::{Class, Object};
    use objc::{msg_send, sel, sel_impl};
    let (Some(app_class), Some(string_class)) =
        (Class::get("NSApplication"), Class::get("NSString"))
    else {
        return;
    };
    let text = std::ffi::CString::new(count.to_string()).unwrap_or_default();
    // SAFETY: AppKit's shared application and its Dock tile live for the
    // whole process; the label is an autoreleased NSString (or nil, which
    // hides the badge), copied by the tile.
    unsafe {
        let app: *mut Object = msg_send![app_class, sharedApplication];
        if app.is_null() {
            return;
        }
        let tile: *mut Object = msg_send![app, dockTile];
        let label: *mut Object = if count > 0 {
            msg_send![string_class, stringWithUTF8String: text.as_ptr()]
        } else {
            std::ptr::null_mut()
        };
        let _: () = msg_send![tile, setBadgeLabel: label];
    }
}

#[cfg(not(target_os = "macos"))]
fn show(count: usize) {
    let properties = format!(
        "{{'count': <int64 {count}>, 'count-visible': <{}>}}",
        count > 0
    );
    std::thread::spawn(move || {
        let _ = std::process::Command::new("gdbus")
            .args([
                "emit",
                "--session",
                "--object-path",
                "/com/canonical/unity/launcherentry/impulse",
                "--signal",
                "com.canonical.Unity.LauncherEntry.Update",
                "application://impulse.desktop",
                &properties,
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    });
}
