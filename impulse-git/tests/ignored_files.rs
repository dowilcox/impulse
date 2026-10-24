//! Ignored files and nested repositories in a working tree, checked against
//! `git check-ignore`, and reads in folders that aren't trusted running
//! nothing the repository names (`IgnoredFilesTests.swift`).

mod common;

use std::time::{Duration, SystemTime};

use common::{git_overrides, setup, RemoveOnDrop, TempRepo};
use impulse_git::cli::restricted_environment;
use impulse_git::file_index::{self, DEFAULT_LIMIT};
use impulse_git::ignored::{ignored_entries, ignored_entry_list, nested_repositories};
use impulse_git::operations;

fn sorted(mut items: Vec<String>) -> Vec<String> {
    items.sort();
    items
}

#[test]
fn lists_ignored_files_and_whole_folders() {
    setup();
    let repo = TempRepo::create();
    repo.commit(
        &[
            (".gitignore", ".env\nnode_modules/\n*.log\n"),
            ("a.txt", "a\n"),
        ],
        "commit",
    );
    assert!(ignored_entries(&repo.root, false).is_empty());

    repo.write(".env", "SECRET=1\n");
    repo.write("node_modules/pkg/index.js", "x\n");
    repo.write("web/debug.log", "x\n");
    repo.write("notes.txt", "untracked, not ignored\n");
    let entries = ignored_entries(&repo.root, false);
    assert_eq!(
        sorted(entries.clone()),
        [".env", "node_modules/", "web/debug.log"]
    );
    // The oracle: git itself says each one is ignored, and the untracked file
    // that isn't stays out.
    for entry in &entries {
        assert_eq!(&repo.git(&["check-ignore", entry]).unwrap(), entry);
    }
    assert!(repo.git(&["check-ignore", "notes.txt"]).is_err());
}

#[test]
fn lists_repositories_inside_the_checkout() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "a\n")], "commit");
    for path in ["clone", "deps/inner/lib"] {
        let nested = TempRepo::at(format!("{}/{path}", repo.root));
        std::fs::create_dir_all(&nested.root).unwrap();
        nested.git(&["init", "-q", "-b", "main"]).unwrap();
        nested.commit(&[("x.txt", "x\n")], "commit");
    }
    repo.write("plain/untracked.txt", "x\n");
    assert_eq!(
        nested_repositories(&repo.root, false).map(sorted),
        Some(vec!["clone/".into(), "deps/inner/lib/".into()])
    );
}

#[test]
fn lists_checked_out_submodules_and_committed_repositories() {
    setup();
    let lib = TempRepo::create();
    let repo = TempRepo::create();
    let task = format!("{}-task", repo.root);
    let _cleanup = RemoveOnDrop(task.clone());
    lib.commit(&[("lib.txt", "lib\n")], "commit");
    repo.commit(&[("a.txt", "a\n")], "commit");
    repo.git(&[
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "add",
        "-q",
        &lib.root,
        "sub",
    ])
    .unwrap();
    repo.git(&["commit", "-q", "-m", "add sub"]).unwrap();
    // A task: in a linked worktree, a submodule's repository lives in the
    // worktree's own git folder, which removing the worktree deletes.
    repo.git(&["worktree", "add", "-q", "-b", "task", &task])
        .unwrap();
    assert_eq!(
        nested_repositories(&task, false),
        Some(vec![]),
        "not checked out: an empty folder"
    );
    let worktree = TempRepo::at(task.as_str());
    worktree
        .git(&[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "update",
            "-q",
            "--init",
        ])
        .unwrap();
    TempRepo::at(format!("{task}/sub")).commit(&[("unpushed.txt", "work\n")], "unpushed");
    assert_eq!(nested_repositories(&task, false), Some(vec!["sub/".into()]));

    // A repository committed as a gitlink, with no .gitmodules.
    let embedded = TempRepo::at(format!("{}/embedded", repo.root));
    std::fs::create_dir_all(&embedded.root).unwrap();
    embedded.git(&["init", "-q", "-b", "main"]).unwrap();
    embedded.commit(&[("x.txt", "x\n")], "commit");
    repo.git(&["add", "embedded"]).unwrap();
    repo.git(&["commit", "-q", "-m", "embed"]).unwrap();
    assert_eq!(
        nested_repositories(&repo.root, false).map(sorted),
        Some(vec!["embedded/".into(), "sub/".into()])
    );
}

#[test]
fn a_configuration_that_cant_be_read_isnt_read_around() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "a\n")], "commit");
    assert!(restricted_environment(&repo.root, &git_overrides()).is_some());
    std::fs::write(repo.path(".git/config"), "[broken\n").unwrap();
    assert!(restricted_environment(&repo.root, &git_overrides()).is_none());
    assert_eq!(
        nested_repositories(&repo.root, false),
        None,
        "not knowing isn't \"none\""
    );
    assert_eq!(ignored_entry_list(&repo.root, false), None);
}

/// A repository whose own configuration names programs (an fsmonitor hook,
/// a clean filter), as a downloaded one can.
fn hostile_repository() -> (TempRepo, String, RemoveOnDrop, RemoveOnDrop) {
    setup();
    let repo = TempRepo::create();
    let ran = format!("{}-ran", repo.root);
    let hook = format!("{}-hook.sh", repo.root);
    std::fs::write(
        &hook,
        format!("#!/bin/sh\necho fsmonitor >> '{ran}'\nexit 1\n"),
    )
    .unwrap();
    set_executable(&hook);
    repo.write(".gitattributes", "*.txt filter=evil\n");
    repo.commit(&[("a.txt", "a\n")], "commit");
    repo.git(&[
        "config",
        "filter.evil.clean",
        &format!("echo filter >> '{ran}'; cat"),
    ])
    .unwrap();
    repo.git(&["config", "core.fsmonitor", &hook]).unwrap();
    (repo, ran.clone(), RemoveOnDrop(ran), RemoveOnDrop(hook))
}

/// What ran during `body`; the file's timestamp moved first, so git status
/// has to look at its content (through the clean filter).
fn what_ran(repo: &TempRepo, ran: &str, age: u64, body: impl FnOnce()) -> Vec<String> {
    let _ = std::fs::remove_file(ran);
    std::fs::File::options()
        .write(true)
        .open(repo.path("a.txt"))
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(age))
        .unwrap();
    body();
    let text = std::fs::read_to_string(ran).unwrap_or_default();
    let mut lines: Vec<String> = text
        .split('\n')
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    lines.sort();
    lines.dedup();
    lines
}

#[test]
fn reads_in_a_folder_that_isnt_trusted_run_nothing_the_repository_names() {
    let (repo, ran, _ran, _hook) = hostile_repository();
    assert!(what_ran(&repo, &ran, 3600, || {
        ignored_entries(&repo.root, false);
    })
    .is_empty());
    assert!(what_ran(&repo, &ran, 7200, || {
        nested_repositories(&repo.root, false);
    })
    .is_empty());
    assert!(what_ran(&repo, &ran, 10800, || {
        file_index::files(&repo.root, DEFAULT_LIMIT, false);
    })
    .is_empty());
    // The oracle: in a trusted folder git does run them.
    assert_eq!(
        what_ran(&repo, &ran, 14400, || {
            ignored_entries(&repo.root, true);
        }),
        ["filter", "fsmonitor"]
    );
}

#[test]
fn a_merge_check_in_a_folder_that_isnt_trusted_runs_no_merge_driver() {
    setup();
    let repo = TempRepo::create();
    let ran = format!("{}-merge-ran", repo.root);
    let _cleanup = RemoveOnDrop(ran.clone());
    repo.commit(
        &[
            (".gitattributes", "*.txt merge=evil\n"),
            ("a.txt", "1\n2\n3\n"),
        ],
        "base",
    );
    repo.git(&[
        "config",
        "merge.evil.driver",
        &format!("echo driver >> '{ran}'; false"),
    ])
    .unwrap();
    repo.git(&["switch", "-q", "-c", "side"]).unwrap();
    repo.commit(&[("a.txt", "1\n2\nside\n")], "side");
    repo.git(&["switch", "-q", "main"]).unwrap();
    repo.commit(&[("a.txt", "main\n2\n3\n")], "main");

    assert!(!operations::is_merged("side", "main", &repo.root, false));
    assert!(!std::path::Path::new(&ran).exists());
    // The oracle: trusted, git does run it.
    operations::is_merged("side", "main", &repo.root, true);
    assert!(std::path::Path::new(&ran).exists());
}

fn set_executable(path: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}
