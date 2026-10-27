//! Closing terminals whose shells take a while to exit on SIGHUP: `close`
//! doesn't wait, and the shells hang up together,
//! so quitting with many open waits for the slowest rather than their sum.
//! In a file of its own: `wait_for_closed_terminals` counts every terminal
//! closed in the process.

use std::time::{Duration, Instant};

use impulse_terminal::{wait_for_closed_terminals, TerminalBackend, TerminalConfig};

#[test]
fn closed_terminals_hang_up_together() {
    let backends: Vec<TerminalBackend> = (0..4)
        .map(|_| {
            let config = TerminalConfig {
                shell_path: "/bin/sh".into(),
                shell_args: vec![
                    "-c".into(),
                    "trap 'sleep 0.5; exit 0' HUP; printf 'ready\\n'; while :; do sleep 0.05; done"
                        .into(),
                ],
                ..TerminalConfig::default()
            };
            TerminalBackend::new(config, 80, 24, 8, 16).expect("spawn backend")
        })
        .collect();
    let pids: Vec<libc::pid_t> = backends
        .iter()
        .map(|b| b.child_pid() as libc::pid_t)
        .collect();

    // Wait until every trap is set.
    let deadline = Instant::now() + Duration::from_secs(5);
    for backend in &backends {
        while Instant::now() < deadline && !backend.transcript(24, false).contains("ready") {
            backend.poll_events();
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    let started = Instant::now();
    for backend in &backends {
        backend.close();
    }
    drop(backends);
    assert!(
        started.elapsed() < Duration::from_millis(250),
        "closing waited for the shells: {:?}",
        started.elapsed()
    );

    wait_for_closed_terminals();
    // One after another would take at least 2 s.
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "the shells didn't exit together: {:?}",
        started.elapsed()
    );
    for pid in pids {
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "shell {pid} is still running"
        );
    }
}
