//! A terminal started with a previous session's output: what it saves again
//! before any command runs is that output, not the new shell's greeting and
//! prompt, and the overlay says where that output ends.

use std::io::Write;
use std::time::{Duration, Instant};

use impulse_terminal::{TerminalBackend, TerminalConfig};

const RULE: &str = "-- restored --";

fn write_script(body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "impulse_restore_test_{}_{n}.sh",
        std::process::id()
    ));
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(body.as_bytes()).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

/// A shell that greets with more lines than the screen holds, then waits
/// at a prompt.
fn greeting_shell() -> String {
    "#!/bin/bash\n\
     for i in $(seq 1 30); do printf 'greeting %d\\n' $i; done\n\
     printf '\\033]133;A;Nonce=%s\\007' \"$IMPULSE_SHELL_NONCE\"\n\
     printf '~/project main\\n> '\n\
     sleep 4\n"
        .to_string()
}

fn start(script: &std::path::Path, restored: Option<&str>) -> TerminalBackend {
    let config = TerminalConfig {
        shell_path: script.to_string_lossy().to_string(),
        restored_transcript: restored.map(str::to_string),
        ..TerminalConfig::default()
    };
    TerminalBackend::new(config, 40, 24, 8, 16).expect("spawn backend")
}

fn wait_for_prompt(backend: &TerminalBackend) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let _ = backend.poll_events();
        if backend.block_overlay().prompt_row.is_some() {
            std::thread::sleep(Duration::from_millis(100));
            let _ = backend.poll_events();
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("no prompt");
}

#[test]
fn before_a_command_the_restored_output_is_what_is_kept() {
    let script = write_script(&greeting_shell());
    let restored = format!("old output\r\nold line two\r\n{RULE}\r\n");
    let mut backend = start(&script, Some(&restored));
    wait_for_prompt(&backend);

    let transcript = backend.transcript(1000, false);
    assert_eq!(
        transcript,
        format!("old output\r\nold line two\r\n{RULE}\r\n")
    );

    // The greeting pushed the rule above the viewport.
    let overlay = backend.block_overlay();
    let prompt = overlay.prompt_row.unwrap();
    let rule = overlay.restored_row.expect("restored row");
    assert_eq!(prompt - rule, 31, "{overlay:?}");

    // A narrower grid rewraps the greeting's lines, and the rule moves with it.
    backend.resize(8, 24, 8, 16);
    std::thread::sleep(Duration::from_millis(200));
    let _ = backend.poll_events();
    let transcript = backend.transcript(1000, false);
    assert!(
        transcript.ends_with(&format!("{RULE}\r\n")),
        "{transcript:?}"
    );
    assert!(!transcript.contains("greeting"), "{transcript:?}");
    let _ = std::fs::remove_file(&script);
}

#[test]
fn a_fresh_terminal_with_no_command_keeps_nothing() {
    let script = write_script(&greeting_shell());
    let backend = start(&script, None);
    wait_for_prompt(&backend);

    assert_eq!(backend.transcript(1000, false), "");
    assert_eq!(backend.block_overlay().restored_row, None);
    let _ = std::fs::remove_file(&script);
}
