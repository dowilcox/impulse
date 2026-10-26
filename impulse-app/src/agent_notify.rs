//! Desktop notifications (the Swift `DesktopNotifier`), behind one
//! function: freedesktop notifications through `notify-send` on Linux,
//! Notification Center on macOS (GPUI's `UNUserNotificationCenter`, as
//! Impulse itself, so a click brings Impulse forward; a binary run outside
//! an app bundle, which the center refuses, falls back to `osascript`,
//! whose notifications are Script Editor's). Either is optional: without
//! it nothing shows, and the tab's attention dot still says so. A click
//! shows the notification's terminal: on macOS, and where `notify-send`
//! takes actions (libnotify 0.7.10 and later). A terminal's notifications
//! are withdrawn when its attention clears (`clear`), and only its latest
//! ten stay up, as Swift's `DesktopNotifier` does: on macOS, and where
//! `notify-send` prints the notification's id. With `sound`, Impulse's own
//! sound (`notification_sound`) plays with it, and the notification has
//! none of its own; on Linux it asks the desktop not to play one
//! (`suppress-sound`).

use std::collections::HashMap;
use std::io::{BufRead as _, BufReader};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

/// A terminal's notifications kept up; older ones are withdrawn.
const DELIVERED_LIMIT: usize = 10;

/// The notifications up (freedesktop ids, Notification Center tags), by
/// the terminal they're for, oldest first.
static DELIVERED: Mutex<Option<HashMap<u64, Vec<String>>>> = Mutex::new(None);

/// Whether Impulse is the app in front (any of its windows active), as
/// `NSApp.isActive`: notifications and the Dock's bounce are for while
/// it isn't.
#[allow(unexpected_cfgs)]
pub(crate) fn app_in_front(cx: &gpui_kit::App) -> bool {
    #[cfg(target_os = "macos")]
    {
        use objc::runtime::{BOOL, Class, NO, Object};
        use objc::{msg_send, sel, sel_impl};
        let _ = cx;
        let Some(class) = Class::get("NSApplication") else {
            return true;
        };
        // SAFETY: the shared application's `isActive`, read on the main
        // thread.
        unsafe {
            let app: *mut Object = msg_send![class, sharedApplication];
            let active: BOOL = msg_send![app, isActive];
            active != NO
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        cx.active_window().is_some()
    }
}

/// Withdraw the terminal's notifications that are still up (it's been
/// seen).
pub(crate) fn clear(terminal: u64, cx: &gpui_kit::App) {
    let ids = DELIVERED
        .lock()
        .ok()
        .and_then(|mut d| d.as_mut()?.remove(&terminal))
        .unwrap_or_default();
    #[cfg(target_os = "macos")]
    center::withdraw(ids, cx);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cx;
        close(ids);
    }
}

/// Note a notification that's up; returns the terminal's oldest past the
/// limit, to withdraw.
fn delivered(terminal: u64, id: String) -> Vec<String> {
    let Ok(mut delivered) = DELIVERED.lock() else {
        return Vec::new();
    };
    let ids = delivered
        .get_or_insert_with(HashMap::new)
        .entry(terminal)
        .or_default();
    ids.push(id);
    let extra = ids.len().saturating_sub(DELIVERED_LIMIT);
    ids.drain(..extra).collect()
}

/// Close notifications by id (freedesktop's `CloseNotification`), off the
/// main thread.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn close(ids: Vec<String>) {
    if ids.is_empty() || !cfg!(target_os = "linux") {
        return;
    }
    std::thread::spawn(move || {
        for id in ids {
            let _ = Command::new("gdbus")
                .args([
                    "call",
                    "--session",
                    "--dest",
                    "org.freedesktop.Notifications",
                    "--object-path",
                    "https://rp2.alfred511.top/hf/org/freedesktop/Notifications",
                    "--method",
                    "org.freedesktop.Notifications.CloseNotification",
                    &id.to_string(),
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    });
}

/// The notification's id from the line `notify-send --print-id` starts
/// with.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn notification_id(line: &str) -> Option<u32> {
    line.trim().parse().ok()
}

/// The name of the action a click on the notification invokes.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const CLICK: &str = "default";

/// Show a notification, for a terminal (`terminal`, its entity id) or
/// not:
/// `title`, then `subtitle` (the workspace and tab) and `body`, with
/// Impulse's `sound` (the `notification_sound` setting's). Failures are
/// ignored. The receiver hears once if the notification is clicked.
pub fn post(
    title: &str,
    subtitle: &str,
    body: &str,
    terminal: Option<u64>,
    sound: Option<crate::notification_sound::Sound>,
    cx: &gpui_kit::App,
) -> Option<async_channel::Receiver<()>> {
    // A snapshot posts no sound, and on the Mac no notification: nothing
    // of it shows on screen (`debug_snapshot`).
    #[cfg(debug_assertions)]
    let sound = sound.filter(|_| !crate::debug_snapshot::is_active());
    #[cfg(all(debug_assertions, target_os = "macos"))]
    if crate::debug_snapshot::is_active() {
        return None;
    }
    if let Some(sound) = sound {
        crate::notification_sound::play(sound);
    }
    #[cfg(target_os = "macos")]
    if center::available() {
        return Some(center::post(title, &lines(subtitle, body), terminal, cx));
    }
    let _ = cx;
    let (title, subtitle, body) = (title.to_string(), subtitle.to_string(), body.to_string());
    let (clicked, receiver) = async_channel::bounded(1);
    std::thread::Builder::new()
        .name("impulse-notification".into())
        .spawn(move || {
            let options = options();
            let Some(mut command) = command(&title, &subtitle, &body, options, sound.is_some())
            else {
                return;
            };
            let piped = options.actions || options.print_id;
            command
                .stdin(Stdio::null())
                .stdout(if piped { Stdio::piped() } else { Stdio::null() })
                .stderr(Stdio::null());
            let Ok(mut child) = command.spawn() else {
                return;
            };
            // It prints the id first; with an action it then waits until
            // the notification closes and says which action (if any) was
            // invoked.
            let mut output = String::new();
            if let Some(stdout) = child.stdout.take() {
                for (index, line) in BufReader::new(stdout)
                    .lines()
                    .map_while(Result::ok)
                    .enumerate()
                {
                    match notification_id(&line).filter(|_| index == 0 && options.print_id) {
                        Some(id) => {
                            if let Some(terminal) = terminal {
                                close(delivered(terminal, id.to_string()));
                            }
                        }
                        None => {
                            output.push_str(&line);
                            output.push('\n');
                        }
                    }
                }
            }
            let _ = child.wait();
            if was_clicked(&output) {
                let _ = clicked.try_send(());
            }
        })
        .ok()?;
    Some(receiver)
}

/// What this `notify-send` takes.
#[derive(Clone, Copy, Default)]
struct Options {
    /// `--action` (libnotify 0.7.10 and later).
    actions: bool,
    /// `--print-id` (0.7.9 and later).
    print_id: bool,
}

/// What `notify-send` takes (asked once).
fn options() -> Options {
    static OPTIONS: OnceLock<Options> = OnceLock::new();
    *OPTIONS.get_or_init(|| {
        if !cfg!(target_os = "linux") {
            return Options::default();
        }
        let help = Command::new("notify-send")
            .arg("--help")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        Options {
            actions: help.contains("--action"),
            print_id: help.contains("--print-id"),
        }
    })
}

/// `notify-send`'s output names the default action: the user clicked it.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn was_clicked(output: &str) -> bool {
    output.lines().any(|line| line.trim() == CLICK)
}

/// `notify-send`'s arguments; with `sound` (Impulse plays its own) the
/// desktop is asked to play none.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn linux_arguments(
    title: &str,
    subtitle: &str,
    body: &str,
    options: Options,
    sound: bool,
) -> Vec<String> {
    let mut arguments = vec!["--app-name=Impulse".to_string()];
    if options.print_id {
        arguments.push("--print-id".to_string());
    }
    if options.actions {
        arguments.push(format!("--action={CLICK}=Show"));
    }
    if sound {
        arguments.push("--hint=boolean:suppress-sound:true".to_string());
    }
    // The title and body come from programs (OSC 9, 777 and 99): after
    // `--`, one starting with `-` stays text instead of becoming an option.
    arguments.push("--".to_string());
    arguments.push(title.to_string());
    arguments.push(lines(subtitle, body));
    arguments
}

#[cfg(target_os = "linux")]
fn command(
    title: &str,
    subtitle: &str,
    body: &str,
    options: Options,
    sound: bool,
) -> Option<Command> {
    let mut command = Command::new("notify-send");
    command.args(linux_arguments(title, subtitle, body, options, sound));
    Some(command)
}

/// Notification Center through GPUI, as Impulse: a click on one
/// (identified by its tag) is handed to the `post` that showed it.
#[cfg(target_os = "macos")]
mod center {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicU64, Ordering};

    use gpui_kit::{App, SystemNotification};

    thread_local! {
        /// Where to say a notification was clicked, by tag.
        static CLICKS: RefCell<HashMap<String, async_channel::Sender<()>>> =
            RefCell::new(HashMap::new());
        static LISTENING: Cell<bool> = const { Cell::new(false) };
    }

    static NEXT: AtomicU64 = AtomicU64::new(0);

    /// Run from an app bundle: the notification center aborts a process
    /// without one (a bare `cargo run`), so GPUI shows nothing there.
    #[allow(unexpected_cfgs)]
    pub(super) fn available() -> bool {
        static BUNDLED: OnceLock<bool> = OnceLock::new();
        *BUNDLED.get_or_init(|| {
            use objc::runtime::{Class, Object};
            use objc::{msg_send, sel, sel_impl};
            let Some(class) = Class::get("NSBundle") else {
                return false;
            };
            // SAFETY: the main bundle and its identifier (an NSString or
            // nil), read once.
            unsafe {
                let bundle: *mut Object = msg_send![class, mainBundle];
                if bundle.is_null() {
                    return false;
                }
                let identifier: *mut Object = msg_send![bundle, bundleIdentifier];
                !identifier.is_null()
            }
        })
    }

    pub(super) fn post(
        title: &str,
        body: &str,
        terminal: Option<u64>,
        cx: &App,
    ) -> async_channel::Receiver<()> {
        if !LISTENING.with(|listening| listening.replace(true)) {
            cx.on_system_notification_response(|response, _| {
                let clicked =
                    CLICKS.with(|clicks| clicks.borrow_mut().remove(response.tag.as_ref()));
                if let Some(clicked) = clicked {
                    let _ = clicked.try_send(());
                }
            });
        }
        let tag = format!(
            "impulse-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let (clicked, receiver) = async_channel::bounded(1);
        CLICKS.with(|clicks| clicks.borrow_mut().insert(tag.clone(), clicked));
        cx.show_system_notification(SystemNotification {
            tag: tag.clone().into(),
            title: title.to_string().into(),
            body: body.to_string().into(),
            actions: Vec::new(),
        });
        if let Some(terminal) = terminal {
            withdraw(super::delivered(terminal, tag), cx);
        }
        receiver
    }

    /// Take notifications out of Notification Center.
    pub(super) fn withdraw(tags: Vec<String>, cx: &App) {
        for tag in tags {
            CLICKS.with(|clicks| clicks.borrow_mut().remove(&tag));
            cx.dismiss_system_notification(&tag);
        }
    }
}

/// Silent (no `sound name`): Impulse plays its own sound.
#[cfg(target_os = "macos")]
fn command(title: &str, subtitle: &str, body: &str, _: Options, _: bool) -> Option<Command> {
    let script = format!(
        "display notification {} with title {} subtitle {}",
        applescript_string(body),
        applescript_string(title),
        applescript_string(subtitle)
    );
    let mut command = Command::new("/usr/bin/osascript");
    command.arg("-e").arg(script);
    Some(command)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn command(_: &str, _: &str, _: &str, _: Options, _: bool) -> Option<Command> {
    None
}

/// The subtitle over the body, either one left out when empty.
fn lines(subtitle: &str, body: &str) -> String {
    [subtitle, body]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A quoted AppleScript string.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn applescript_string(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applescript_strings_escape_quotes() {
        assert_eq!(
            applescript_string(r#"say "hi" \o/"#),
            r#""say \"hi\" \\o/""#
        );
    }

    #[test]
    fn a_click_shows_the_terminal_where_notify_send_says() {
        assert_eq!(
            linux_arguments(
                "Codex finished",
                "impulse · zsh",
                "",
                Options {
                    actions: true,
                    print_id: true
                },
                false
            ),
            vec![
                "--app-name=Impulse",
                "--print-id",
                "--action=default=Show",
                "--",
                "Codex finished",
                "impulse · zsh"
            ]
        );
        assert_eq!(
            linux_arguments("Bell", "", "", Options::default(), false).len(),
            4
        );
        // A title that looks like an option stays text.
        let arguments = linux_arguments("--icon=/x", "", "", Options::default(), false);
        assert_eq!(arguments[1], "--");
        assert!(was_clicked("default\n"));
        assert!(!was_clicked(""));
        assert!(!was_clicked("other\n"));
        assert_eq!(notification_id("42\n"), Some(42));
        assert_eq!(notification_id("default"), None);
    }

    #[test]
    fn the_desktop_plays_no_sound_over_impulses() {
        let arguments = linux_arguments("Bell", "", "", Options::default(), true);
        assert_eq!(
            arguments,
            vec![
                "--app-name=Impulse",
                "--hint=boolean:suppress-sound:true",
                "--",
                "Bell",
                ""
            ]
        );
        assert!(
            !linux_arguments("Bell", "", "", Options::default(), false)
                .iter()
                .any(|a| a.contains("suppress-sound"))
        );
    }

    #[test]
    fn subtitle_and_body_share_the_text() {
        assert_eq!(lines("impulse · zsh", "Bell"), "impulse · zsh\nBell");
        assert_eq!(lines("", "Bell"), "Bell");
    }
}
