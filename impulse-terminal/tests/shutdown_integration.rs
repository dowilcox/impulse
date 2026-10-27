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

/// A shell that writes as it hangs up (fish does) exits promptly: its output
/// is read meanwhile, where on macOS it would otherwise wait to drain, past
/// SIGKILL, until the PTY closed.
#[test]
fn shutdown_reads_what_the_shell_writes_as_it_exits() {
    let config = TerminalConfig {
        shell_path: "/bin/sh".into(),
        shell_args: vec![
            "-c".into(),
            "trap 'yes | head -c 200000; exit 0' HUP; printf 'ready\\n'; while :; do sleep 0.05; done"
                .into(),
        ],
        ..TerminalConfig::default()
    };
    let backend = TerminalBackend::new(config, 80, 24, 8, 16).expect("spawn backend");
    let pid = backend.child_pid() as libc::pid_t;

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && !backend.transcript(24, false).contains("ready") {
        backend.poll_events();
        std::thread::sleep(Duration::from_millis(20));
    }

    let started = Instant::now();
    backend.shutdown();
    // Well inside the grace before SIGKILL.
    assert!(
        started.elapsed() < Duration::from_millis(900),
        "the shell took {:?} to exit",
        started.elapsed()
    );
    assert_eq!(
        unsafe { libc::kill(pid, 0) },
        -1,
        "the shell is still running"
    );
}
