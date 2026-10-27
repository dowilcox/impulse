//! The app's shell-integration scripts (bash, zsh, fish) send the terminal's
//! nonce with every mark, so once they run, marks a program prints are
//! ignored while the shell's own still make blocks.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use impulse_terminal::{TerminalBackend, TerminalConfig, TerminalEvent};

fn script(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../impulse-kit/resources/shell-integration")
        .join(name)
}

/// A mark with no nonce, as a program (or a `cat`ed file) would print it.
const FORGED: &str = "printf '\\033]133;C\\033\\134x\\n\\033]133;D;1\\007\\033]7;file:///\\007'";

/// Run `shell_args` under `shell` until it exits, collecting what it reports.
fn run(shell: &str, shell_args: Vec<String>) -> (Vec<TerminalEvent>, TerminalBackend) {
    let config = TerminalConfig {
        shell_path: shell.into(),
        shell_args,
        ..TerminalConfig::default()
    };
    let backend = TerminalBackend::new(config, 80, 24, 8, 16).expect("spawn backend");
    let mut events = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && !events.iter().any(|e| matches!(e, TerminalEvent::Exit)) {
        events.extend(backend.poll_events());
        std::thread::sleep(Duration::from_millis(25));
    }
    (events, backend)
}

fn check(shell: &str, events: &[TerminalEvent], backend: &TerminalBackend) {
    let blocks = backend.command_block_summaries();
    assert_eq!(blocks[0].command.as_deref(), Some("echo hi"), "{shell}");
    assert_eq!(blocks[0].exit_code, Some(0), "{shell}");
    // (bash's DEBUG trap also reports the commands that follow.)
    assert!(
        blocks.iter().all(|block| block.exit_code != Some(1)),
        "{shell}: the forged end mark counted: {blocks:#?}"
    );
    let count = |kind: fn(&TerminalEvent) -> bool| events.iter().filter(|e| kind(e)).count();
    assert_eq!(
        count(|e| matches!(e, TerminalEvent::PromptStart)),
        2,
        "{shell}"
    );
    assert!(
        count(|e| matches!(e, TerminalEvent::ShellPath(_))) >= 1,
        "{shell}"
    );
    assert!(
        count(|e| matches!(e, TerminalEvent::ShellNames(_))) >= 1,
        "{shell}"
    );
    let cwds: Vec<&String> = events
        .iter()
        .filter_map(|e| match e {
            TerminalEvent::CwdChanged(path) => Some(path),
            _ => None,
        })
        .collect();
    assert!(!cwds.is_empty(), "{shell}: no cwd");
    assert!(
        cwds.iter().all(|cwd| *cwd != "/"),
        "{shell}: the forged cwd counted"
    );
}

#[test]
fn zsh_marks_carry_the_nonce() {
    if !Path::new("/bin/zsh").exists() {
        return;
    }
    let body = format!(
        "source '{}'; __impulse_precmd; __impulse_preexec 'echo hi'; echo hi; \
         __impulse_precmd; {FORGED}; sleep 1",
        script("zsh.sh").display()
    );
    let (events, backend) = run("/bin/zsh", vec!["-f".into(), "-c".into(), body]);
    check("zsh", &events, &backend);
}

#[test]
fn bash_marks_carry_the_nonce() {
    if !Path::new("/bin/bash").exists() {
        return;
    }
    // The DEBUG trap reports `echo hi`; the prompt command runs as bash's
    // PROMPT_COMMAND would.
    let body = format!(
        "source '{}'\n__impulse_prompt_command\n__impulse_prompt_end\necho hi\n\
         __impulse_prompt_command\n__impulse_prompt_end\n{FORGED}\nsleep 1\n",
        script("bash.sh").display()
    );
    let (events, backend) = run("/bin/bash", vec!["--norc".into(), "-c".into(), body]);
    check("bash", &events, &backend);
}

#[test]
fn fish_marks_carry_the_nonce() {
    let Some(fish) = [
        "/opt/homebrew/bin/fish",
        "/usr/local/bin/fish",
        "/usr/bin/fish",
    ]
    .into_iter()
    .find(|path| Path::new(path).exists()) else {
        return;
    };
    let body = format!(
        "source '{}'; emit fish_prompt; emit fish_preexec 'echo hi'; echo hi; \
         emit fish_prompt; {FORGED}; sleep 1",
        script("fish.sh").display()
    );
    let (events, backend) = run(fish, vec!["--no-config".into(), "-c".into(), body]);
    check("fish", &events, &backend);
}
