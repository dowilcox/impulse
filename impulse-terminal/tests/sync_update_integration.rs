//! A synchronized update (DEC mode 2026) that is never closed — the program
//! died mid-frame, an ssh connection dropped — must not hide output forever.
//! The backend flushes it once vte's sync timeout passes.

use std::io::Write;
use std::time::{Duration, Instant};

use impulse_terminal::{TerminalBackend, TerminalConfig};

fn write_script(body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::temp_dir().join(format!("impulse_sync_test_{}.sh", std::process::id()));
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(body.as_bytes()).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

fn screen_text(backend: &TerminalBackend) -> String {
    backend.select_all();
    let text = backend.selected_text().unwrap_or_default();
    backend.clear_selection();
    text
}

#[test]
fn unfinished_synchronized_update_is_flushed() {
    // Begin a synchronized update, draw, and never end it.
    let script = write_script("#!/bin/bash\nprintf '\\033[?2026hstuck-frame'\nsleep 5\n");
    let config = TerminalConfig {
        shell_path: script.to_string_lossy().to_string(),
        ..TerminalConfig::default()
    };
    let backend = TerminalBackend::new(config, 80, 24, 8, 16).expect("spawn backend");

    let end = Instant::now() + Duration::from_secs(4);
    let mut seen = false;
    while Instant::now() < end {
        let _ = backend.poll_events();
        if screen_text(&backend).contains("stuck-frame") {
            seen = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = std::fs::remove_file(&script);
    assert!(
        seen,
        "output after an unterminated BSU should appear once the sync timeout passes"
    );
}
