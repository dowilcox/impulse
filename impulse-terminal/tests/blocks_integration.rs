//! Ground-truth integration tests for command-block tracking.
//!
//! Spawns a small shell script as the "shell" that emits the exact OSC 133 /
//! OSC 6973 byte stream a real fish/zsh/bash session produces, then inspects
//! `block_overlay()` to verify consecutive commands map to distinct,
//! separator-bearing block regions.

use std::io::Write;
use std::time::{Duration, Instant};

use impulse_terminal::{TerminalBackend, TerminalConfig};

/// Write an executable shell script to a temp path and return it.
fn write_script(body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir();
    let path = dir.join(format!("impulse_blocks_test_{}.sh", std::process::id()));
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(body.as_bytes()).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

fn backend_running(script: &std::path::Path) -> TerminalBackend {
    let config = TerminalConfig {
        shell_path: script.to_string_lossy().to_string(),
        ..TerminalConfig::default()
    };
    TerminalBackend::new(config, 80, 24, 8, 16).expect("spawn backend")
}

/// Poll until the overlay reports at least `want` blocks, or time out.
fn wait_for_blocks(backend: &TerminalBackend, want: usize) -> impulse_terminal::BlockOverlay {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last = backend.block_overlay();
    while Instant::now() < deadline {
        last = backend.block_overlay();
        if last.blocks.len() >= want {
            // Give a beat for any trailing marks to land.
            std::thread::sleep(Duration::from_millis(150));
            return backend.block_overlay();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    last
}

/// Emit one full command cycle: prompt mark, echoed prompt+command line,
/// the command-text mark, command-start, output, command-end.
///
/// The OSC payload (`encoded`) is passed as a `printf` *argument* via `%s`, not
/// embedded in the format string — otherwise bash's `printf` would interpret a
/// percent-encoded space like `%20` as a field-width conversion and corrupt it.
fn cmd_cycle(label: &str, encoded: &str, output: &str, exit: i32) -> String {
    format!(
        "printf '\\033]133;A\\007'\n\
         printf '$ {label}\\n'\n\
         printf '\\033]6973;Command=%s\\007' '{encoded}'\n\
         printf '\\033]133;C\\007'\n\
         {output_cmd}\
         printf '\\033]133;D;{exit}\\007'\n",
        output_cmd = if output.is_empty() {
            String::new()
        } else {
            format!("printf '%s\\n' '{output}'\n")
        }
    )
}

#[test]
fn consecutive_commands_become_distinct_blocks() {
    let body = format!(
        "#!/bin/bash\n{}{}{}sleep 4\n",
        cmd_cycle("echo first", "echo%20first", "first", 0),
        cmd_cycle("false", "false", "", 1),
        cmd_cycle("echo third", "echo%20third", "third", 0),
    );
    let script = write_script(&body);
    let backend = backend_running(&script);

    let overlay = wait_for_blocks(&backend, 3);
    let _ = std::fs::remove_file(&script);

    assert!(
        overlay.blocks.len() >= 3,
        "expected 3 distinct blocks, got {}: {:#?}",
        overlay.blocks.len(),
        overlay.blocks
    );

    // Distinct, increasing start rows — i.e. real separators between them.
    let starts: Vec<i32> = overlay.blocks.iter().map(|b| b.start_row).collect();
    for w in starts.windows(2) {
        assert!(
            w[1] > w[0],
            "block start rows should strictly increase (separators between blocks): {starts:?}"
        );
    }

    // The middle command (`false`) must be flagged failed.
    let failed: Vec<bool> = overlay.blocks.iter().map(|b| b.failed).collect();
    assert!(
        failed.iter().any(|&f| f),
        "the `false` command should be a failed block: {failed:?}"
    );
}

#[test]
fn commands_after_an_inline_interactive_program_stay_distinct() {
    // Simulate: a normal command, then an inline raw-mode program (like Claude
    // Code) that produces a screenful of output WITHOUT emitting OSC 133, then
    // more normal commands. The later commands must remain distinct blocks.
    let mut inline = String::from(
        "printf '\\033]133;A\\007'\nprintf '$ claude\\n'\n\
         printf '\\033]6973;Command=claude\\007'\nprintf '\\033]133;C\\007'\n",
    );
    // A screenful-plus of plain output (no OSC marks), as an inline TUI would.
    inline.push_str("for i in $(seq 1 40); do printf 'claude line %d\\n' $i; done\n");
    inline.push_str("printf '\\033]133;D;0\\007'\n");

    let body = format!(
        "#!/bin/bash\n{}{}{}{}sleep 4\n",
        inline,
        cmd_cycle("cd ..", "cd%20..", "", 0),
        cmd_cycle(
            "cat AGENTS.md",
            "cat%20AGENTS.md",
            "fish: Unknown command",
            127
        ),
        cmd_cycle("echo done", "echo%20done", "done", 0),
    );
    let script = write_script(&body);
    let backend = backend_running(&script);

    // The inline program's own block scrolls off the top once its screenful of
    // output pushes past the viewport; what must survive is that each of the
    // three commands *after* it stays a distinct, separator-bearing region.
    let overlay = wait_for_blocks(&backend, 3);
    let _ = std::fs::remove_file(&script);

    let starts: Vec<i32> = overlay.blocks.iter().map(|b| b.start_row).collect();
    assert!(
        overlay.blocks.len() >= 3,
        "expected >=3 distinct blocks after an inline program, got {}: {starts:?}\n{:#?}",
        overlay.blocks.len(),
        overlay.blocks
    );
    for w in starts.windows(2) {
        assert!(
            w[1] > w[0],
            "block start rows should strictly increase (separators between blocks): {starts:?}"
        );
    }

    // The failed `cat` must be flagged so its failure stripe/wash renders.
    assert!(
        overlay.blocks.iter().any(|b| b.failed),
        "the failed `cat` command should be a failed block: {:#?}",
        overlay.blocks
    );
}
