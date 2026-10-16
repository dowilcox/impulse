//! Closing a terminal whose shell ignores SIGHUP: alacritty's PTY waits for
//! the child when it's dropped, so the reader thread used to hang there.

use std::time::{Duration, Instant};

use impulse_terminal::{TerminalBackend, TerminalConfig};

#[test]
fn shutdown_ends_a_shell_that_ignores_sighup() {
    let config = TerminalConfig {
        shell_path: "/bin/sh".into(),
        shell_args: vec![
            "-c".into(),
            "trap '' HUP; printf 'ready\\n'; read line".into(),
        ],
        ..TerminalConfig::default()
    };
    let backend = TerminalBackend::new(config, 80, 24, 8, 16).expect("spawn backend");
    let pid = backend.child_pid() as libc::pid_t;

    // Wait until the trap is set.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && !backend.transcript(24, false).contains("ready") {
        backend.poll_events();
        std::thread::sleep(Duration::from_millis(20));
    }

    let started = Instant::now();
    backend.shutdown();
    // The reader thread finished (shutdown gives up waiting after 2 s), and
    // with it the shell: killed and reaped.
    assert!(started.elapsed() < Duration::from_millis(1900));
    assert_eq!(
        unsafe { libc::kill(pid, 0) },
        -1,
        "the shell is still running"
    );
}
