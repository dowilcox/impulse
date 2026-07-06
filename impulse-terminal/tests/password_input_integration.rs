//! Ground-truth test for password-input detection.
//!
//! Spawns a real shell whose script reads a line with `read -s` — the same
//! termios shape (ECHO off, ICANON on) sudo and ssh use for password prompts —
//! and verifies the backend flips `PasswordInputChanged` on and back off, so
//! frontends can mask the input bar.

use std::io::Write;
use std::time::{Duration, Instant};

use impulse_terminal::{TerminalBackend, TerminalConfig, TerminalEvent};

/// Write an executable shell script to a temp path and return it.
fn write_script(body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir();
    let path = dir.join(format!("impulse_password_test_{}.sh", std::process::id()));
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(body.as_bytes()).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

/// Poll events until the predicate matches one, or time out.
fn wait_for_event(
    backend: &TerminalBackend,
    deadline: Duration,
    mut pred: impl FnMut(&TerminalEvent) -> bool,
) -> bool {
    let end = Instant::now() + deadline;
    while Instant::now() < end {
        if backend.poll_events().iter().any(&mut pred) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

#[test]
fn read_s_flips_password_input_on_and_off() {
    let script = write_script(
        "#!/bin/bash\n\
         printf 'Password: '\n\
         read -s pw\n\
         printf '\\nok\\n'\n\
         sleep 4\n",
    );
    let config = TerminalConfig {
        shell_path: script.to_string_lossy().to_string(),
        ..TerminalConfig::default()
    };
    let backend = TerminalBackend::new(config, 80, 24, 8, 16).expect("spawn backend");

    // `read -s` disables ECHO while keeping canonical mode → password mode on.
    assert!(
        wait_for_event(&backend, Duration::from_secs(5), |ev| matches!(
            ev,
            TerminalEvent::PasswordInputChanged(true)
        )),
        "expected PasswordInputChanged(true) while `read -s` waits for input"
    );
    assert!(backend.password_input(), "query should mirror the event");

    // Replying restores ECHO → password mode off.
    backend.write(b"hunter2\n");
    assert!(
        wait_for_event(&backend, Duration::from_secs(5), |ev| matches!(
            ev,
            TerminalEvent::PasswordInputChanged(false)
        )),
        "expected PasswordInputChanged(false) after the reply restored echo"
    );
    assert!(!backend.password_input(), "query should mirror the event");

    let _ = std::fs::remove_file(&script);
}
