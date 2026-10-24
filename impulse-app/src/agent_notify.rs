//! Desktop notifications (the Swift `DesktopNotifier`), behind one
//! function: freedesktop notifications through `notify-send` on Linux,
//! Notification Center through `osascript` on macOS. Either is optional:
//! without it nothing shows, and the tab's attention dot still says so.

use std::process::{Command, Stdio};

/// Show a notification: `title`, then `subtitle` (the workspace and tab)
/// and `body`. Runs off the main thread; failures are ignored.
pub fn post(title: &str, subtitle: &str, body: &str) {
    let Some(mut command) = command(title, subtitle, body) else {
        return;
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    std::thread::spawn(move || {
        if let Ok(mut child) = command.spawn() {
            let _ = child.wait();
        }
    });
}

#[cfg(target_os = "linux")]
fn command(title: &str, subtitle: &str, body: &str) -> Option<Command> {
    let mut command = Command::new("notify-send");
    command
        .arg("--app-name=Impulse")
        .arg(title)
        .arg(lines(subtitle, body));
    Some(command)
}

#[cfg(target_os = "macos")]
fn command(title: &str, subtitle: &str, body: &str) -> Option<Command> {
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
fn command(_: &str, _: &str, _: &str) -> Option<Command> {
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
    fn subtitle_and_body_share_the_text() {
        assert_eq!(lines("impulse · zsh", "Bell"), "impulse · zsh\nBell");
        assert_eq!(lines("", "Bell"), "Bell");
    }
}
