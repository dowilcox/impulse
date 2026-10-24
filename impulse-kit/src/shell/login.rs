//! The user's login shell and the PATH it sees (the Swift `LoginShell`).
//!
//! The login shell comes from the user database: Open Directory (`dscl`) on
//! macOS, where `/etc/passwd` only holds system accounts, and `getent
//! passwd` on Linux (which also asks LDAP, SSSD and the like). Both lookups
//! run once per process and are cached; `dscl` can take seconds on
//! directory-bound Macs.
//!
//! The PATH is a login shell's, not the app's: started from the Dock or a
//! desktop launcher, Impulse inherits a minimal PATH that misses Homebrew,
//! mise/asdf shims, `~/.local/bin` and so on, so tools like `git` and
//! language servers are resolved against the user's real login environment.

use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use super::ShellType;
use crate::child_process::ChildProcess;

/// Common install prefixes added after the login PATH and the app's own.
#[cfg(target_os = "macos")]
const EXTRA_PATH: &str = "/opt/homebrew/bin:/usr/local/bin";
#[cfg(not(target_os = "macos"))]
const EXTRA_PATH: &str = "/usr/local/bin";

/// The user's login shell from the user database, or `None` when it can't
/// be determined or the shell binary doesn't exist.
pub fn user_login_shell() -> Option<String> {
    static SHELL: OnceLock<Option<String>> = OnceLock::new();
    SHELL.get_or_init(look_up_user_login_shell).clone()
}

fn look_up_user_login_shell() -> Option<String> {
    let username = std::env::var("USER").ok()?;
    // Validated, so it can't reach anywhere else in the lookup's argument.
    let allowed = !username.is_empty()
        && username
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.');
    if !allowed {
        log::warn!("Refusing $USER with unexpected characters: {username}");
        return None;
    }
    let shell = query_user_database(&username)?;
    (!shell.is_empty() && Path::new(&shell).exists()).then_some(shell)
}

#[cfg(target_os = "macos")]
fn query_user_database(username: &str) -> Option<String> {
    let result = ChildProcess::new("/usr/bin/dscl")
        .args([".", "-read", &format!("/Users/{username}"), "UserShell"])
        .timeout(Duration::from_secs(5))
        .run()
        .ok()?;
    if result.status != 0 || result.timed_out {
        return None;
    }
    let output = String::from_utf8(result.stdout).ok()?;
    Some(output.strip_prefix("UserShell:")?.trim().to_string())
}

#[cfg(not(target_os = "macos"))]
fn query_user_database(username: &str) -> Option<String> {
    let getent = ["/usr/bin/getent", "/bin/getent"]
        .into_iter()
        .find(|p| Path::new(p).exists())?;
    let result = ChildProcess::new(getent)
        .args(["passwd", username])
        .timeout(Duration::from_secs(5))
        .run()
        .ok()?;
    if result.status != 0 || result.timed_out {
        return None;
    }
    let output = String::from_utf8(result.stdout).ok()?;
    passwd_shell(output.lines().next()?)
}

/// The shell field of a passwd line (`name:x:uid:gid:gecos:home:shell`).
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn passwd_shell(line: &str) -> Option<String> {
    let fields: Vec<&str> = line.split(':').collect();
    (fields.len() >= 7).then(|| fields[6].trim().to_string())
}

/// The default shell path, preferring the user database over `$SHELL`.
pub fn default_shell_path() -> String {
    static PATH: OnceLock<String> = OnceLock::new();
    PATH.get_or_init(|| {
        user_login_shell()
            .or_else(|| std::env::var("SHELL").ok().filter(|s| !s.is_empty()))
            .unwrap_or_else(|| "/bin/bash".to_string())
    })
    .clone()
}

/// How the default shell quotes (what terminals run).
pub fn default_shell_type() -> ShellType {
    ShellType::detect(&default_shell_path())
}

/// The short name of the user's default shell (`fish`, `zsh`, `bash`).
pub fn default_shell_name() -> String {
    let path = default_shell_path();
    let name = path.rsplit('/').next().unwrap_or_default();
    if name.is_empty() {
        "shell".to_string()
    } else {
        name.to_string()
    }
}

/// The `PATH` a login shell would see. Captured once (blocking, with a
/// timeout; the app warms it at launch) and cached. Falls back to the
/// process `PATH` plus common install prefixes.
pub fn login_path() -> String {
    static CACHED: Mutex<Option<String>> = Mutex::new(None);
    let mut cached = CACHED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(path) = cached.as_ref() {
        return path.clone();
    }
    let captured = capture_login_path().unwrap_or_default();
    let fallback = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin:/usr/sbin:/sbin".into());
    let path = merge_paths(&[&captured, &fallback, EXTRA_PATH]);
    *cached = Some(path.clone());
    path
}

/// Joins colon-separated PATH strings, dropping empty and duplicate entries
/// while keeping the first-seen order.
pub fn merge_paths(paths: &[&str]) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<&str> = Vec::new();
    for entry in paths.iter().flat_map(|p| p.split(':')) {
        if !entry.is_empty() && seen.insert(entry) {
            out.push(entry);
        }
    }
    out.join(":")
}

/// Resolve an executable name against the login `PATH`. A name with a slash
/// is checked as it is.
pub fn which(name: &str) -> Option<String> {
    if name.contains('/') {
        return is_executable_file(Path::new(name)).then(|| name.to_string());
    }
    login_path()
        .split(':')
        .filter(|dir| !dir.is_empty())
        .map(|dir| format!("{dir}/{name}"))
        .find(|candidate| is_executable_file(Path::new(candidate)))
}

/// A file (after symlinks) the user may execute.
pub fn is_executable_file(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file()) && may_execute(path)
}

#[cfg(unix)]
fn may_execute(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: access only reads the NUL-terminated path.
    unsafe { libc::access(c_path.as_ptr(), libc::X_OK) == 0 }
}

#[cfg(not(unix))]
fn may_execute(_path: &Path) -> bool {
    true
}

/// The PATH of an interactive login shell, like a terminal's, so entries
/// added in `.zshrc` or `.bashrc` (nvm, pyenv, cargo, ...) count too. An rc
/// file that never finishes (one that execs tmux, say) falls back to a
/// login-only shell. `IMPULSE_RESOLVING_ENVIRONMENT=1` lets rc files skip
/// slow or interactive setup.
fn capture_login_path() -> Option<String> {
    let shell = default_shell_path();
    // `printf '%s' "$PATH"` works in bash, zsh and fish (fish joins PATH
    // variables with ':' when quoted). Markers isolate it from rc-file noise.
    let script = "printf '__IMPULSE_PATH__%s__IMPULSE_PATH__' \"$PATH\"";
    capture_path(&shell, &["-i", "-l", "-c", script], Duration::from_secs(5))
        .or_else(|| capture_path(&shell, &["-l", "-c", script], Duration::from_secs(3)))
}

fn capture_path(shell: &str, arguments: &[&str], timeout: Duration) -> Option<String> {
    let result = ChildProcess::new(shell)
        .args(arguments)
        .env("IMPULSE_RESOLVING_ENVIRONMENT", "1")
        .timeout(timeout)
        .run()
        .ok()?;
    if result.timed_out {
        return None;
    }
    let output = result.stdout_text();
    let parts: Vec<&str> = output.split("__IMPULSE_PATH__").collect();
    if parts.len() < 3 {
        return None;
    }
    let path = parts[1].trim();
    (!path.is_empty()).then(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_paths_in_order_without_duplicates() {
        assert_eq!(
            merge_paths(&["/a:/b", "", "/b::/c", "/a:/d"]),
            "/a:/b:/c:/d"
        );
        assert_eq!(merge_paths(&[]), "");
    }

    #[test]
    fn reads_the_shell_from_a_passwd_line() {
        assert_eq!(
            passwd_shell("me:x:1000:1000:Me,,,:/home/me:/usr/bin/fish"),
            Some("/usr/bin/fish".to_string())
        );
        assert_eq!(passwd_shell("broken:x:1"), None);
    }

    #[cfg(unix)]
    #[test]
    fn which_checks_executables() {
        assert_eq!(which("/bin/sh"), Some("/bin/sh".to_string()));
        assert_eq!(which("/etc/passwd"), None);
        assert_eq!(which("/nonexistent/tool"), None);
        assert!(which("sh").is_some_and(|p| p.ends_with("/sh")));
    }
}
