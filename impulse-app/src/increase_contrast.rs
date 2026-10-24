//! The system's Increase Contrast setting, which strengthens the chrome's
//! own colors (`ChromePalette::new`'s `increase_contrast`), as the Swift
//! `ChromePalette` reads `accessibilityDisplayShouldIncreaseContrast` and
//! re-applies the theme when it changes. On macOS that's the Accessibility
//! display option; on Linux, GNOME's high-contrast setting
//! (`org.gnome.desktop.a11y.interface high-contrast`, read with
//! `gsettings`). Where neither is there it's off.
//!
//! Windows observe the `IncreaseContrast` global to rebuild their palette.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::{App, Global};

pub struct IncreaseContrast(pub bool);

impl Global for IncreaseContrast {}

/// Whether the chrome should use its stronger colors.
pub fn get(cx: &App) -> bool {
    cx.try_global::<IncreaseContrast>().is_some_and(|c| c.0)
}

/// Read the setting and follow its changes.
pub fn init(cx: &mut App) {
    let Some(on) = read() else {
        cx.set_global(IncreaseContrast(false));
        return;
    };
    cx.set_global(IncreaseContrast(on));
    // Read off the main thread (on Linux it starts a process), checked
    // here once a second.
    let flag = Arc::new(AtomicBool::new(on));
    let reader = flag.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(POLL);
        if let Some(on) = read() {
            reader.store(on, Ordering::Relaxed);
        }
    });
    cx.spawn(async move |cx| loop {
        cx.background_executor().timer(Duration::from_secs(1)).await;
        let on = flag.load(Ordering::Relaxed);
        cx.update(|cx| {
            if get(cx) != on {
                cx.set_global(IncreaseContrast(on));
            }
        });
    })
    .detach();
}

#[cfg(target_os = "macos")]
const POLL: Duration = Duration::from_secs(1);
#[cfg(not(target_os = "macos"))]
const POLL: Duration = Duration::from_secs(5);

/// The setting now, or `None` where the system has none to read.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
fn read() -> Option<bool> {
    use objc::runtime::{Class, Object, BOOL, YES};
    use objc::{msg_send, sel, sel_impl};
    let class = Class::get("NSWorkspace")?;
    // SAFETY: `sharedWorkspace` and the BOOL property are plain getters on
    // AppKit's shared object, which lives for the whole process.
    unsafe {
        let workspace: *mut Object = msg_send![class, sharedWorkspace];
        if workspace.is_null() {
            return None;
        }
        let on: BOOL = msg_send![workspace, accessibilityDisplayShouldIncreaseContrast];
        Some(on == YES)
    }
}

#[cfg(not(target_os = "macos"))]
fn read() -> Option<bool> {
    let output = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.a11y.interface", "high-contrast"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim() == "true")
}
