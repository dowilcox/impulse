//! Whether a command word names something the terminal's shell can run, for
//! the input bar's unknown-command underline (the Swift `CommandLookup`).
//! The shell integration reports the shell's aliases, functions, builtins
//! and keywords (OSC 6973;Names) and its PATH (OSC 6973;Path); executables
//! on that PATH are listed on a background thread.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

/// After this long, a word not found makes the PATH be listed again (it may
/// have been installed since).
const STALE_AFTER: Duration = Duration::from_secs(10);

/// Always runnable, whatever the shell reported (and before it has).
const BASE_NAMES: &[&str] = &[
    "!", ".", ":", "[", "[[", "]]", "{", "}", "alias", "bg", "builtin", "case", "cd", "command",
    "do", "done", "echo", "elif", "else", "esac", "eval", "exec", "exit", "export", "false", "fc",
    "fg", "fi", "for", "function", "hash", "history", "if", "jobs", "kill", "local", "popd",
    "printf", "pushd", "pwd", "read", "return", "select", "set", "shift", "source", "test", "then",
    "time", "trap", "true", "type", "ulimit", "umask", "unalias", "unset", "until", "wait",
    "while",
];

type ListDirectory = Box<dyn Fn(&str) -> Vec<String> + Send + Sync>;

#[derive(Default)]
struct State {
    names: HashSet<String>,
    path: Option<String>,
    executables: HashSet<String>,
    scanned_path: Option<String>,
    scanned_at: Option<Instant>,
    scanning: bool,
}

struct Inner {
    state: Mutex<State>,
    scan_done: Condvar,
    list_directory: ListDirectory,
}

/// One per terminal. Cheap to clone; clones share the same lookup.
#[derive(Clone)]
pub struct CommandLookup {
    inner: Arc<Inner>,
}

impl Default for CommandLookup {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandLookup {
    pub fn new() -> Self {
        Self::with_lister(|dir| {
            std::fs::read_dir(dir)
                .map(|entries| {
                    entries
                        .flatten()
                        .filter_map(|e| e.file_name().into_string().ok())
                        .collect()
                })
                .unwrap_or_default()
        })
    }

    /// With `list_directory` listing a folder's names (a seam for tests).
    pub fn with_lister(
        list_directory: impl Fn(&str) -> Vec<String> + Send + Sync + 'static,
    ) -> Self {
        CommandLookup {
            inner: Arc::new(Inner {
                state: Mutex::new(State::default()),
                scan_done: Condvar::new(),
                list_directory: Box::new(list_directory),
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The shell's aliases, functions, builtins and keywords.
    pub fn set_names<I: IntoIterator<Item = String>>(&self, names: I) {
        self.state().names = names.into_iter().collect();
    }

    /// The shell's PATH; its executables are listed in the background.
    pub fn set_path(&self, path: &str) {
        let changed = {
            let mut state = self.state();
            let changed = state.path.as_deref() != Some(path);
            state.path = Some(path.to_string());
            changed
        };
        if changed {
            self.rescan();
        }
    }

    /// Wait for a pending PATH listing (tests and headless runs).
    pub fn wait_for_scan(&self) {
        let mut state = self.state();
        while state.scanning {
            state = self
                .inner
                .scan_done
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
    }

    /// `Some(true)` or `Some(false)` when it can tell; `None` before the
    /// shell has reported (or while its PATH is still being listed), and for
    /// quoted or expanded words.
    pub fn is_known(&self, word: &str, cwd: Option<&str>) -> Option<bool> {
        // `\ls` skips aliases, and still runs `ls`.
        let command = word.strip_prefix('\\').unwrap_or(word);
        if command.is_empty() {
            return Some(true);
        }
        if command != "{" && command != "}" && command.contains(|c| "\"'$`*?(){}".contains(c)) {
            return None;
        }
        if command.contains('/') {
            return is_runnable_file(command, cwd);
        }
        if BASE_NAMES.contains(&command) {
            return Some(true);
        }

        let (reported, ready, known, stale) = {
            let state = self.state();
            (
                !state.names.is_empty() || state.path.is_some(),
                state.path.is_some() && state.scanned_path == state.path,
                state.names.contains(command) || state.executables.contains(command),
                state.scanned_at.is_none_or(|at| at.elapsed() > STALE_AFTER),
            )
        };
        if !reported {
            return None;
        }
        if known {
            return Some(true);
        }
        if !ready {
            return None;
        }
        // Maybe it was installed since the last listing.
        if stale {
            self.rescan();
        }
        Some(false)
    }

    /// List the PATH's folders on a background thread, again whenever the
    /// PATH changed while a listing ran.
    fn rescan(&self) {
        let mut path = {
            let mut state = self.state();
            match (&state.path, state.scanning) {
                (Some(path), false) => {
                    let path = path.clone();
                    state.scanning = true;
                    path
                }
                _ => return,
            }
        };
        let inner = self.inner.clone();
        thread::spawn(move || loop {
            let mut found = HashSet::new();
            for directory in path.split(':').filter(|d| !d.is_empty()) {
                found.extend((inner.list_directory)(directory));
            }
            let mut state = inner.state.lock().unwrap_or_else(|e| e.into_inner());
            state.executables = found;
            state.scanned_path = Some(path.clone());
            state.scanned_at = Some(Instant::now());
            match &state.path {
                Some(current) if *current != path => path = current.clone(),
                _ => {
                    state.scanning = false;
                    inner.scan_done.notify_all();
                    return;
                }
            }
        });
    }
}

fn is_runnable_file(command: &str, cwd: Option<&str>) -> Option<bool> {
    let path = if let Some(rest) = command.strip_prefix("~/") {
        dirs::home_dir()?.join(rest)
    } else if command.starts_with('/') {
        Path::new(command).to_path_buf()
    } else {
        Path::new(cwd?).join(command)
    };
    let Ok(meta) = std::fs::metadata(&path) else {
        return Some(false);
    };
    // A directory runs as `cd` in zsh and fish (auto_cd).
    Some(meta.is_dir() || super::login::is_executable_file(&path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn lookup() -> CommandLookup {
        CommandLookup::with_lister(|directory| match directory {
            "/bin" => vec!["ls".into(), "cat".into()],
            "/opt/tools" => vec!["rg".into()],
            _ => Vec::new(),
        })
    }

    #[test]
    fn unsure_until_the_shell_reports() {
        let commands = lookup();
        assert_eq!(commands.is_known("gti", None), None);
        assert_eq!(
            commands.is_known("cd", None),
            Some(true),
            "builtins everyone has"
        );
    }

    #[test]
    fn names_and_path_executables() {
        let commands = lookup();
        commands.set_names(["gst".to_string(), "myfn".to_string()]);
        assert_eq!(commands.is_known("gst", None), Some(true));
        assert_eq!(commands.is_known("ls", None), None, "PATH not listed yet");
        commands.set_path("/bin:/opt/tools");
        commands.wait_for_scan();
        assert_eq!(commands.is_known("ls", None), Some(true));
        assert_eq!(commands.is_known("rg", None), Some(true));
        assert_eq!(
            commands.is_known("\\ls", None),
            Some(true),
            "a backslash only skips aliases"
        );
        assert_eq!(commands.is_known("gti", None), Some(false));
    }

    #[test]
    fn a_path_change_during_a_scan_is_picked_up() {
        let (started_tx, started) = mpsc::channel::<()>();
        let (release_tx, release) = mpsc::channel::<()>();
        let started_tx = Mutex::new(started_tx);
        let release = Mutex::new(release);
        let commands = CommandLookup::with_lister(move |directory| match directory {
            "/bin" => {
                started_tx.lock().unwrap().send(()).unwrap();
                release.lock().unwrap().recv().unwrap();
                vec!["ls".into()]
            }
            "/opt/tools" => vec!["rg".into()],
            _ => Vec::new(),
        });
        commands.set_path("/bin");
        started.recv().unwrap();
        commands.set_path("/opt/tools"); // arrives while /bin is being listed
        release_tx.send(()).unwrap();
        commands.wait_for_scan();
        assert_eq!(commands.is_known("rg", None), Some(true));
        assert_eq!(commands.is_known("ls", None), Some(false));
    }

    #[test]
    fn quoted_and_expanded_words_are_left_alone() {
        let commands = lookup();
        commands.set_path("/bin");
        commands.wait_for_scan();
        assert_eq!(commands.is_known("$EDITOR", None), None);
        assert_eq!(commands.is_known("\"ls\"", None), None);
        assert_eq!(commands.is_known("ls*", None), None);
    }

    #[cfg(unix)]
    #[test]
    fn paths_are_checked_on_disk() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("run.sh");
        std::fs::write(&script, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "").unwrap();
        let cwd = dir.path().to_str();

        let commands = lookup();
        assert_eq!(commands.is_known("./run.sh", cwd), Some(true));
        assert_eq!(
            commands.is_known(script.to_str().unwrap(), None),
            Some(true)
        );
        assert_eq!(commands.is_known("./missing", cwd), Some(false));
        assert_eq!(
            commands.is_known("./run.sh", None),
            None,
            "relative without a directory"
        );
        assert_eq!(
            commands.is_known("./notes.txt", cwd),
            Some(false),
            "not executable"
        );
    }
}
