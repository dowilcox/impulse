//! The shell's environment: alacritty_terminal adds its window ids to every
//! child, and Impulse's own launch environment may carry color overrides;
//! neither should reach a terminal's shell.

use std::time::{Duration, Instant};

use impulse_terminal::{TerminalBackend, TerminalConfig};

#[test]
fn the_shell_does_not_look_like_alacritty() {
    let config = TerminalConfig {
        shell_path: "/bin/sh".into(),
        shell_args: vec![
            "-c".into(),
            "printf 'ids=[%s%s]\\n' \"$ALACRITTY_WINDOW_ID\" \"$WINDOWID\"; sleep 3".into(),
        ],
        ..TerminalConfig::default()
    };
    let backend = TerminalBackend::new(config, 80, 24, 8, 16).expect("spawn backend");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut text = String::new();
    while Instant::now() < deadline {
        backend.poll_events();
        text = backend.transcript(24, false);
        if text.contains("ids=[") {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(text.contains("ids=[]"), "unexpected output: {text:?}");
    backend.shutdown();
}
