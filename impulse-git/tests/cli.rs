//! Running git and classifying its failures, git folders, and the quick-open
//! file index (`GitCLITests.swift`: `GitCLITests`, `GitDirectoryTests`,
//! `FileIndexTests`).

mod common;

use common::{git_overrides, temp_dir, RemoveOnDrop, TempRepo};
use impulse_git::cli::{self, ErrorKind, RunOptions};
use impulse_git::file_index::{self, DEFAULT_LIMIT};
use impulse_git::{common_git_directory, git_directory};

#[test]
fn classifies_common_failures() {
    let cases: &[(&str, &[&str], ErrorKind)] = &[
        ("fatal: not a git repository (or any of the parent directories): .git", &["status"], ErrorKind::NotARepository),
        ("*** Please tell me who you are.\n\nRun\n  git config --global user.email", &["commit"], ErrorKind::IdentityNotConfigured),
        ("fatal: Unable to create '/r/.git/index.lock': File exists.", &["add"], ErrorKind::IndexLocked),
        (
            "error: Your local changes to the following files would be overwritten by checkout:\n\ta.txt",
            &["switch", "x"],
            ErrorKind::LocalChangesWouldBeOverwritten,
        ),
        (" ! [rejected]        main -> main (fetch first)\nerror: failed to push some refs", &["push"], ErrorKind::NonFastForward),
        ("fatal: The current branch topic has no upstream branch.", &["push"], ErrorKind::NoUpstream),
        (
            "fatal: could not read Username for 'https://github.com': terminal prompts disabled",
            &["fetch"],
            ErrorKind::AuthenticationFailed,
        ),
        ("git@github.com: Permission denied (publickey).", &["fetch"], ErrorKind::AuthenticationFailed),
        ("CONFLICT (content): Merge conflict in a.txt", &["merge", "x"], ErrorKind::MergeConflict),
        ("nothing to commit, working tree clean", &["commit"], ErrorKind::NothingToCommit),
        ("fatal: a branch named 'x' already exists", &["branch", "x"], ErrorKind::BranchAlreadyExists),
        ("fatal: tag 'v1' already exists", &["tag", "v1", "HEAD"], ErrorKind::TagAlreadyExists),
        (
            " ! [rejected]        v1 -> v1 (already exists)\nerror: failed to push some refs",
            &["push", "--progress", "origin", "refs/tags/v1"],
            ErrorKind::TagAlreadyExists,
        ),
        (
            "hint: Diverging branches can't be fast-forwarded\nfatal: Not possible to fast-forward, aborting.",
            &["pull", "--ff-only"],
            ErrorKind::Diverged,
        ),
        ("fatal: invalid reference: nope", &["switch", "nope"], ErrorKind::UnknownRevision),
        (
            "error: pathspec 'nope' did not match any file(s) known to git",
            &["checkout", "nope"],
            ErrorKind::UnknownRevision,
        ),
        ("husky - pre-commit hook exited with code 1 (error)", &["commit", "-F", "-"], ErrorKind::HookFailed),
        ("something unexpected", &["status"], ErrorKind::Other),
    ];
    for (output, arguments, expected) in cases {
        assert_eq!(cli::classify(output, arguments), *expected, "{output}");
    }
}

fn overrides_options(env: &std::collections::HashMap<String, String>) -> RunOptions<'_> {
    RunOptions {
        environment: Some(env),
        ..Default::default()
    }
}

#[test]
fn runs_git_and_returns_output() {
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "first");
    let env = git_overrides();
    let output = cli::run(&["log", "--format=%s"], &repo.root, overrides_options(&env)).unwrap();
    assert_eq!(output.status, 0);
    assert_eq!(output.stdout.trim(), "first");
}

/// `/usr/bin/git` (the xcrun shim) becomes the git it would start, so each
/// command is one program launch; any other git is kept as it is.
#[cfg(target_os = "macos")]
#[test]
fn the_tools_shim_is_run_past() {
    use impulse_git::process;
    if cli::is_missing_tools_shim("/usr/bin/git") {
        return; // needs the developer tools
    }
    assert_eq!(
        cli::past_tools_shim("/opt/homebrew/bin/git"),
        "/opt/homebrew/bin/git"
    );
    let git = cli::past_tools_shim("/usr/bin/git");
    assert_ne!(git, "/usr/bin/git");
    let shim = process::run(
        "/usr/bin/git",
        &["--exec-path"],
        process::Options::default(),
    )
    .unwrap();
    let direct = process::run(&git, &["--exec-path"], process::Options::default()).unwrap();
    assert_eq!(direct.status, 0);
    assert_eq!(direct.stdout, shim.stdout);
}

/// Elsewhere `/usr/bin/git` is git itself, never a missing shim.
#[cfg(not(target_os = "macos"))]
#[test]
fn usr_bin_git_is_git_itself() {
    assert!(!cli::is_missing_tools_shim("/usr/bin/git"));
    assert_eq!(cli::past_tools_shim("/usr/bin/git"), "/usr/bin/git");
}

#[test]
fn switching_to_unknown_branch_is_classified() {
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "commit");
    let env = git_overrides();
    let error = cli::run(
        &["switch", "does-not-exist"],
        &repo.root,
        overrides_options(&env),
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::UnknownRevision);
    assert!(!error.message().is_empty());
}

#[test]
fn passes_stdin_to_git() {
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "commit");
    repo.write("a.txt", "two\n");
    repo.git(&["add", "a.txt"]).unwrap();
    let env = git_overrides();
    let message = "Subject line\n\nBody paragraph.\n";
    cli::run(
        &["commit", "-q", "-F", "-"],
        &repo.root,
        RunOptions {
            stdin: Some(message.as_bytes()),
            environment: Some(&env),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        repo.git(&["log", "-1", "--format=%B"]).unwrap(),
        "Subject line\n\nBody paragraph."
    );
}

#[test]
fn dirty_tree_switch_is_classified() {
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "commit");
    repo.git(&["branch", "other"]).unwrap();
    repo.git(&["switch", "-q", "other"]).unwrap();
    repo.commit(&[("a.txt", "other\n")], "commit");
    repo.git(&["switch", "-q", "main"]).unwrap();
    repo.write("a.txt", "local edit\n");
    let env = git_overrides();
    let error = cli::run(&["switch", "other"], &repo.root, overrides_options(&env)).unwrap_err();
    assert_eq!(error.kind, ErrorKind::LocalChangesWouldBeOverwritten);
}

#[test]
fn streams_progress_lines() {
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "commit");
    let env = git_overrides();
    let mut lines: Vec<String> = Vec::new();
    let mut collect = |line: &str| lines.push(line.to_string());
    let _ = cli::run(
        &["rev-parse", "--verify", "nope"],
        &repo.root,
        RunOptions {
            environment: Some(&env),
            on_output_line: Some(&mut collect),
            ..Default::default()
        },
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("fatal") || l.contains("Needed a single revision")),
        "{lines:?}"
    );
}

#[test]
fn resolves_git_dir_for_checkout_and_linked_worktree() {
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "commit");
    let dot_git = format!("{}/.git", repo.root);
    assert_eq!(git_directory(&repo.root).as_deref(), Some(dot_git.as_str()));
    assert_eq!(
        common_git_directory(&repo.root).as_deref(),
        Some(dot_git.as_str())
    );

    let worktree = format!("{}-wt", repo.root);
    let _cleanup = RemoveOnDrop(worktree.clone());
    repo.git(&["worktree", "add", "-q", "-b", "topic", &worktree])
        .unwrap();
    let name = std::path::Path::new(&worktree)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let git_dir = git_directory(&worktree).unwrap();
    assert_eq!(git_dir, format!("{dot_git}/worktrees/{name}"));
    assert_eq!(
        common_git_directory(&worktree).as_deref(),
        Some(dot_git.as_str())
    );
    assert!(std::path::Path::new(&git_dir).join("index").exists());
}

#[test]
fn file_index_lists_tracked_and_untracked_but_not_ignored() {
    let repo = TempRepo::create();
    repo.commit(
        &[(".gitignore", "build/\n"), ("src/a.swift", "a\n")],
        "commit",
    );
    repo.write("src/b.swift", "b\n");
    repo.write("build/out.o", "x\n");
    let mut files = file_index::files(&repo.root, DEFAULT_LIMIT, false);
    files.sort();
    assert_eq!(files, [".gitignore", "src/a.swift", "src/b.swift"]);
}

#[test]
fn file_index_walks_non_repositories() {
    let dir = temp_dir("impulse-index");
    let _cleanup = RemoveOnDrop(dir.clone());
    std::fs::create_dir_all(format!("{dir}/sub")).unwrap();
    std::fs::write(format!("{dir}/one.txt"), "x").unwrap();
    std::fs::write(format!("{dir}/sub/two.txt"), "y").unwrap();
    assert_eq!(
        file_index::files(&dir, DEFAULT_LIMIT, false),
        ["one.txt", "sub/two.txt"]
    );
}
