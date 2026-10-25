//! Desktop notifications (the Swift `DesktopNotifier`), behind one
//! function: freedesktop notifications through `notify-send` on Linux,
//! Notification Center through `osascript` on macOS. Either is optional:
//! without it nothing shows, and the tab's attention dot still says so.
//! Where `notify-send` takes actions (libnotify 0.7.10 and later), the
//! notification's default action reports a click, so it can show its
//! terminal; `osascript` has no way to say.

use std::io::Read as _;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// The name of the action a click on the notification invokes.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const CLICK: &str = "default";

/// Show a notification: `title`, then `subtitle` (the workspace and tab)
/// and `body`. Runs off the main thread; failures are ignored. The
/// receiver hears once if the notification is clicked (it closes
/// otherwise).
pub fn post(title: &str, subtitle: &str, body: &str) -> Option<async_channel::Receiver<()>> {
    let (title, subtitle, body) = (title.to_string(), subtitle.to_string(), body.to_string());
    let (clicked, receiver) = async_channel::bounded(1);
    std::thread::Builder::new()
        .name("impulse-notification".into())
        .spawn(move || {
            let actions = takes_actions();
            let Some(mut command) = command(&title, &subtitle, &body, actions) else {
                return;
            };
            command
                .stdin(Stdio::null())
                .stdout(if actions {
                    Stdio::piped()
                } else {
                    Stdio::null()
                })
                .stderr(Stdio::null());
            let Ok(mut child) = command.spawn() else {
                return;
            };
            // With an action it waits until the notification closes, then
            // says which action (if any) was invoked.
            let mut output = String::new();
            if let Some(mut stdout) = child.stdout.take() {
                let _ = stdout.read_to_string(&mut output);
            }
            let _ = child.wait();
            if was_clicked(&output) {
                let _ = clicked.try_send(());
            }
        })
        .ok()?;
    Some(receiver)
}

/// Whether `notify-send` takes `--action` (asked once).
fn takes_actions() -> bool {
    static TAKES: OnceLock<bool> = OnceLock::new();
    *TAKES.get_or_init(|| {
        cfg!(target_os = "linux")
            && Command::new("notify-send")
                .arg("--help")
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
                .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("--action"))
    })
}

/// `notify-send`'s output names the default action: the user clicked it.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn was_clicked(output: &str) -> bool {
    output.lines().any(|line| line.trim() == CLICK)
}

/// `notify-send`'s arguments.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn linux_arguments(title: &str, subtitle: &str, body: &str, actions: bool) -> Vec<String> {
    let mut arguments = vec!["--app-name=Impulse".to_string()];
    if actions {
        arguments.push(format!("--action={CLICK}=Show"));
    }
    // The title and body come from programs (OSC 9, 777 and 99): after
    // `--`, one starting with `-` stays text instead of becoming an option.
    arguments.push("--".to_string());
    arguments.push(title.to_string());
    arguments.push(lines(subtitle, body));
    arguments
}

#[cfg(target_os = "linux")]
fn command(title: &str, subtitle: &str, body: &str, actions: bool) -> Option<Command> {
    let mut command = Command::new("notify-send");
    command.args(linux_arguments(title, subtitle, body, actions));
    Some(command)
}

#[cfg(target_os = "macos")]
fn command(title: &str, subtitle: &str, body: &str, _: bool) -> Option<Command> {
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
fn command(_: &str, _: &str, _: &str, _: bool) -> Option<Command> {
    None
}

/// The subtitle over the body, either one left out when empty.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
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
            linux_arguments("Codex finished", "impulse · zsh", "", true),
            vec![
                "--app-name=Impulse",
                "--action=default=Show",
                "--",
                "Codex finished",
                "impulse · zsh"
            ]
        );
        assert_eq!(linux_arguments("Bell", "", "", false).len(), 4);
        // A title that looks like an option stays text.
        let arguments = linux_arguments("--icon=/x", "", "", false);
        assert_eq!(arguments[1], "--");
        assert!(was_clicked("default\n"));
        assert!(!was_clicked(""));
        assert!(!was_clicked("other\n"));
    }

    #[test]
    fn subtitle_and_body_share_the_text() {
        assert_eq!(lines("impulse · zsh", "Bell"), "impulse · zsh\nBell");
        assert_eq!(lines("", "Bell"), "Bell");
    }
}
