//! Tags, remotes, pull modes, merge and rebase, conflicts taken and
//! reopened, against a bare repository standing in for the remote. The git
//! CLI is the oracle (`GitRemoteTests.swift`).

mod common;

use std::collections::HashMap;
use std::time::{Duration, UNIX_EPOCH};

use common::{setup, RemoveOnDrop, TempRepo};
use impulse_git::cli::ErrorKind;
use impulse_git::operations::{self, ConflictPoint, OperationError, PullMode, PushOptions};
use impulse_git::safety::{self, OPLOG_PREFIX};
use impulse_git::snapshot::current_operation;
use impulse_git::{log, snapshot, HistoryQuery, RepoOperation};

/// A clone of a bare "origin" with one pushed commit on main.
fn clone_with_origin() -> (TempRepo, TempRepo, TempRepo) {
    setup();
    let origin = TempRepo::create();
    origin.git(&["config", "core.bare", "true"]).unwrap();
    let seed = TempRepo::create();
    seed.commit(&[("a.txt", "1\n")], "first");
    let url = format!("{}/.git", origin.root);
    seed.git(&["remote", "add", "origin", &url]).unwrap();
    seed.git(&["push", "-q", "-u", "origin", "main"]).unwrap();
    let repo = TempRepo::create();
    repo.git(&["remote", "add", "origin", &url]).unwrap();
    repo.git(&["fetch", "-q", "origin"]).unwrap();
    repo.git(&["checkout", "-q", "-B", "main", "--track", "origin/main"])
        .unwrap();
    (repo, origin, seed)
}

fn set_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// What a server said on a push, from git's `remote:` lines (ImpulseKit's
/// `GitServerMessage.parse` in the Swift app): the lines, the first link, and
/// its caption (the text before it, or the line above).
fn server_message(output: &[String]) -> Option<(Vec<String>, Option<String>, Option<String>)> {
    let progress = [
        "Enumerating objects",
        "Counting objects",
        "Compressing objects",
        "Total ",
        "Resolving deltas",
    ];
    let lines: Vec<String> = output
        .iter()
        .filter_map(|raw| raw.trim().strip_prefix("remote:"))
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty() && !progress.iter().any(|p| text.starts_with(p)))
        .collect();
    if lines.is_empty() {
        return None;
    }
    for (index, line) in lines.iter().enumerate() {
        let Some(start) = line.find("http://").or_else(|| line.find("https://")) else {
            continue;
        };
        let link: String = line[start..]
            .chars()
            .take_while(|c| !c.is_whitespace())
            .collect();
        let before = line[..start].trim();
        let caption = if before.is_empty() {
            index.checked_sub(1).map(|i| lines[i].clone())
        } else {
            Some(before.to_string())
        };
        return Some((lines.clone(), Some(link), caption));
    }
    Some((lines, None, None))
}

fn host(link: &str) -> &str {
    let rest = link.split_once("://").map_or(link, |(_, rest)| rest);
    rest.split(['/', '?']).next().unwrap_or(rest)
}

/// How HEAD moved, by its reflog subject (ImpulseKit's
/// `DependencyChanges.move` in the Swift app).
#[derive(Debug, PartialEq, Eq)]
enum Move {
    OwnCommit,
    Merge,
    Checkout,
}

fn head_move(subject: &str) -> Move {
    let action = subject.split(':').next().unwrap_or("").trim();
    if action.starts_with("commit")
        || action.starts_with("cherry-pick")
        || action.starts_with("revert")
    {
        Move::OwnCommit
    } else if action.starts_with("pull")
        || action.starts_with("merge")
        || action.starts_with("rebase")
    {
        Move::Merge
    } else {
        Move::Checkout
    }
}

#[test]
fn a_push_carries_the_servers_message() {
    let (repo, origin, _seed) = clone_with_origin();
    // A server hook prints what hosts print: git relays it as "remote:"
    // lines.
    let hook = origin.path(".git/hooks/post-receive");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(
        &hook,
        "#!/bin/sh\necho \"To create a merge request for topic, visit:\"\necho \"  https://git.example.edu/web/repo/-/merge_requests/new?merge_request%5Bsource_branch%5D=topic\"\n",
    )
    .unwrap();
    set_executable(&hook);
    repo.git(&["switch", "-q", "-c", "topic"]).unwrap();
    repo.commit(&[("b.txt", "2\n")], "topic");

    let mut lines = Vec::new();
    let mut collect = |line: &str| lines.push(line.to_string());
    operations::push(
        &PushOptions {
            set_upstream: true,
            branch: Some("topic".into()),
            ..Default::default()
        },
        &repo.root,
        Some(&mut collect),
    )
    .unwrap();
    let (_, link, caption) = server_message(&lines).unwrap();
    assert_eq!(
        caption.as_deref(),
        Some("To create a merge request for topic, visit:")
    );
    assert_eq!(link.as_deref().map(host), Some("git.example.edu"));
}

#[test]
fn new_worktree_branch_does_not_track_a_remote_base() {
    let (repo, _origin, _seed) = clone_with_origin();
    let path = format!("{}-wt-task", repo.root);
    let _cleanup = RemoveOnDrop(path.clone());
    operations::add_worktree(&path, "task", true, Some("origin/main"), false, &repo.root).unwrap();
    assert_eq!(
        repo.git(&["rev-parse", "task"]).unwrap(),
        repo.git(&["rev-parse", "origin/main"]).unwrap()
    );
    assert!(repo
        .git(&["rev-parse", "--abbrev-ref", "task@{upstream}"])
        .is_err());
}

#[test]
fn merged_branches_including_squash_merges() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n")], "base");
    for name in ["merged", "squashed", "open"] {
        repo.git(&["switch", "-q", "-c", name, "main"]).unwrap();
        repo.commit(&[(&format!("{name}.txt"), "work\n")], name);
        repo.commit(
            &[(&format!("{name}.txt"), "more work\n")],
            &format!("{name} 2"),
        );
    }
    repo.git(&["switch", "-q", "main"]).unwrap();
    repo.git(&["merge", "-q", "--no-ff", "-m", "Merge merged", "merged"])
        .unwrap();
    repo.git(&["merge", "-q", "--squash", "squashed"]).unwrap();
    repo.git(&["commit", "-q", "-m", "Squash squashed"])
        .unwrap();
    assert!(operations::is_merged("merged", "main", &repo.root, false));
    assert!(
        operations::is_merged("squashed", "main", &repo.root, false),
        "absorbed: merging it changes nothing"
    );
    assert!(!operations::is_merged("open", "main", &repo.root, false));
}

#[test]
fn conflicts_are_predicted_without_touching_a_folder() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("page.tsx", "a\n"), ("link.tsx", "Link\n")], "base");
    let base = repo.head();
    repo.git(&["switch", "-q", "-c", "upgrade"]).unwrap();
    repo.commit(&[("page.tsx", "upgrade\n")], "upgrade");
    repo.git(&["switch", "-q", "main"]).unwrap();
    assert_eq!(
        operations::merge_base("main", "upgrade", &repo.root),
        Some(base.clone())
    );
    assert_eq!(
        operations::predict_conflicts("main", "upgrade", &repo.root),
        Some(vec![]),
        "nothing on main yet"
    );

    // Uncommitted work on main that touches the same lines.
    repo.write("page.tsx", "overhaul\n");
    let working = safety::working_tree_commit(&repo.root, true).unwrap();
    assert_ne!(working, base);
    assert_eq!(
        repo.git(&["status", "--porcelain"]).unwrap(),
        "M page.tsx",
        "the index and files are untouched"
    );
    assert_eq!(
        operations::predict_conflicts(&working, "upgrade", &repo.root),
        Some(vec!["page.tsx".into()])
    );
    repo.git(&["checkout", "--", "page.tsx"]).unwrap();
    assert_eq!(
        safety::working_tree_commit(&repo.root, true),
        Some(base),
        "clean: HEAD itself"
    );
}

#[test]
fn head_moves_are_told_apart_by_their_reflog() {
    let (repo, _origin, seed) = clone_with_origin();
    repo.commit(&[("b.txt", "1\n")], "own");
    assert_eq!(
        operations::head_reflog_subject(&repo.root).map(|s| head_move(&s)),
        Some(Move::OwnCommit)
    );

    seed.commit(&[("composer.lock", "{}\n")], "deps");
    seed.git(&["push", "-q", "origin", "main"]).unwrap();
    let before = repo.head();
    operations::pull(PullMode::Merge, &repo.root, None).unwrap();
    assert_eq!(
        operations::head_reflog_subject(&repo.root).map(|s| head_move(&s)),
        Some(Move::Merge)
    );
    let after = repo.head();
    assert_eq!(
        operations::changed_paths(&before, &after, &repo.root),
        ["composer.lock"]
    );

    let last = operations::last_head_move(&repo.root).unwrap();
    assert_eq!(
        (last.from.as_str(), last.to.as_str()),
        (before.as_str(), after.as_str())
    );
    assert_eq!(head_move(&last.subject), Move::Merge);
    // The reflog is stamped with the committer date, which the tests pin.
    assert_eq!(last.time, UNIX_EPOCH + Duration::from_secs(1_767_225_600));
    assert_eq!(
        operations::ahead_behind("main", "origin/main", &repo.root),
        Some((2, 0))
    );

    repo.git(&["switch", "-q", "-c", "other"]).unwrap();
    repo.git(&["switch", "-q", "main"]).unwrap();
    assert_eq!(
        operations::head_reflog_subject(&repo.root).map(|s| head_move(&s)),
        Some(Move::Checkout)
    );
}

#[test]
fn commits_ahead_are_newest_first() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n")], "base");
    repo.git(&["switch", "-q", "-c", "task"]).unwrap();
    repo.commit(&[("a.txt", "2\n")], "first");
    repo.commit(&[("a.txt", "3\n")], "second\twith a tab");
    let commits = operations::commits_ahead("task", "main", &repo.root, 50);
    let subjects: Vec<&str> = commits.iter().map(|(_, s)| s.as_str()).collect();
    assert_eq!(subjects, ["second\twith a tab", "first"]);
    assert_eq!(commits[0].0, repo.git(&["rev-parse", "task"]).unwrap());
    assert_eq!(
        operations::commits_ahead("task", "main", &repo.root, 1).len(),
        1
    );
    assert!(operations::commits_ahead("main", "task", &repo.root, 50).is_empty());
}

/// A repository whose own config shows signatures runs its `gpg.program`
/// from `git log`, `show` and `reflog` on a signed commit: no read of
/// Impulse's may, and their output stays parseable.
#[test]
fn untrusted_snapshots_run_no_filter() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n")], "base");
    let marker = repo.path("filter-ran");
    let program = repo.path("fake-filter");
    repo.write(
        "fake-filter",
        &format!("#!/bin/sh\ntouch '{}'\ncat\n", marker.display()),
    );
    set_executable(&program);
    repo.git(&["config", "filter.evil.clean", &program.to_string_lossy()])
        .unwrap();
    repo.write(".git/info/attributes", "a.txt filter=evil\n");
    repo.write("a.txt", "2\n");
    // The setup does what it should: a trusted snapshot runs the filter.
    assert!(safety::working_tree_commit(&repo.root, true).is_some());
    assert!(marker.exists());
    std::fs::remove_file(&marker).unwrap();
    assert!(safety::working_tree_commit(&repo.root, false).is_some());
    assert!(safety::create_with("turn", &repo.root, safety::OPLOG_PREFIX, false).is_ok());
    assert!(
        !marker.exists(),
        "an untrusted snapshot ran the repository's filter"
    );
}

#[test]
fn no_read_runs_the_repositorys_gpg_program() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n")], "base");
    let tree = repo.git(&["write-tree"]).unwrap();
    let parent = repo.head();
    let signed = repo
        .git_stdin(
            &["hash-object", "-t", "commit", "-w", "--stdin"],
            &format!(
                "tree {tree}\nparent {parent}\nauthor Impulse Test <test@impulse.invalid> 1767225600 +0000\ncommitter Impulse Test <test@impulse.invalid> 1767225600 +0000\ngpgsig -----BEGIN PGP SIGNATURE-----\n \n fake\n -----END PGP SIGNATURE-----\n\nsigned\n"
            ),
        )
        .unwrap();
    repo.git(&["update-ref", "-m", "commit: signed", "HEAD", &signed])
        .unwrap();
    let marker = repo.path("gpg-ran");
    let program = repo.path("fake-gpg");
    repo.write(
        "fake-gpg",
        &format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display()),
    );
    set_executable(&program);
    repo.git(&["config", "log.showSignature", "true"]).unwrap();
    repo.git(&["config", "gpg.program", &program.to_string_lossy()])
        .unwrap();
    // The setup does what it should: a plain `git log` runs the program.
    let _ = repo.git(&["log", "-1", "--format=%s"]);
    assert!(marker.exists());
    std::fs::remove_file(&marker).unwrap();

    assert_eq!(
        operations::head_message(&repo.root).as_deref(),
        Some("signed")
    );
    assert!(operations::commit_date("HEAD", &repo.root).is_some());
    let ahead: Vec<String> = operations::commits_ahead("HEAD", &parent, &repo.root, 5)
        .into_iter()
        .map(|(_, s)| s)
        .collect();
    assert_eq!(ahead, ["signed"]);
    assert_eq!(
        operations::head_reflog_subject(&repo.root).as_deref(),
        Some("commit: signed")
    );
    assert_eq!(
        operations::last_head_move(&repo.root).map(|m| m.to),
        Some(signed.clone())
    );
    let entries = log::entries(
        &repo.root,
        &log::LogScope::Head,
        None,
        &HistoryQuery::default(),
        0,
        300,
    )
    .unwrap();
    assert_eq!(entries[0].subject, "signed");
    assert!(log::decorations(&repo.root).unwrap().contains_key(&signed));
    assert!(log::details(&repo.root, &signed)
        .unwrap()
        .message
        .starts_with("signed"));
    assert!(!marker.exists(), "a read ran the repository's gpg.program");
}

#[test]
fn commit_date_is_the_committers_date() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n")], "first");
    let expected: u64 = repo
        .git(&["log", "-1", "--format=%ct", "HEAD"])
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        operations::commit_date("HEAD", &repo.root),
        Some(UNIX_EPOCH + Duration::from_secs(expected))
    );
    assert_eq!(operations::commit_date("no-such-branch", &repo.root), None);
}

#[test]
fn a_remote_branch_opened_as_a_task_tracks_it() {
    let (repo, _origin, seed) = clone_with_origin();
    let path = format!("{}-wt-shared", repo.root);
    let _cleanup = RemoveOnDrop(path.clone());
    seed.git(&["switch", "-q", "-c", "shared"]).unwrap();
    seed.commit(&[("s.txt", "1\n")], "shared");
    seed.git(&["push", "-q", "origin", "shared"]).unwrap();
    repo.git(&["fetch", "-q", "origin"]).unwrap();
    operations::add_worktree(
        &path,
        "shared",
        true,
        Some("origin/shared"),
        true,
        &repo.root,
    )
    .unwrap();
    assert_eq!(
        repo.git(&["rev-parse", "--abbrev-ref", "shared@{upstream}"])
            .unwrap(),
        "origin/shared"
    );
}

#[test]
fn lightweight_and_annotated_tags() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n")], "first");
    let first = repo.head();
    repo.commit(&[("a.txt", "2\n")], "second");

    operations::create_tag("v1.0", &first, None, false, &repo.root).unwrap();
    operations::create_tag(
        "v2.0",
        "HEAD",
        Some("Second release\n\nNotes."),
        false,
        &repo.root,
    )
    .unwrap();
    assert_eq!(repo.git(&["cat-file", "-t", "v1.0"]).unwrap(), "commit");
    assert_eq!(repo.git(&["cat-file", "-t", "v2.0"]).unwrap(), "tag");
    assert_eq!(
        repo.git(&["tag", "-l", "--format=%(contents)", "v2.0"])
            .unwrap(),
        "Second release\n\nNotes."
    );

    let tags = operations::tag_details(&repo.root);
    let by_name: HashMap<&str, &operations::TagInfo> =
        tags.iter().map(|t| (t.name.as_str(), t)).collect();
    assert_eq!(by_name.len(), 2);
    let v1 = by_name["v1.0"];
    assert!(v1.commit == first && !v1.is_annotated && v1.subject == "first");
    let v2 = by_name["v2.0"];
    assert!(v2.commit == repo.head() && v2.is_annotated && v2.subject == "Second release");

    // Existing names fail unless forced; bad names never reach git.
    assert!(operations::create_tag("v1.0", "HEAD", None, false, &repo.root).is_err());
    operations::create_tag("v1.0", "HEAD", None, true, &repo.root).unwrap();
    assert_eq!(
        repo.git(&["rev-parse", "v1.0^{commit}"]).unwrap(),
        repo.head()
    );
    assert!(matches!(
        operations::create_tag("bad name", "HEAD", None, false, &repo.root),
        Err(OperationError::Invalid(_))
    ));

    operations::delete_tag("v1.0", &repo.root).unwrap();
    assert_eq!(repo.git(&["tag"]).unwrap(), "v2.0");
}

#[test]
fn push_and_delete_tags_on_remote() {
    let (repo, origin, _seed) = clone_with_origin();
    assert_eq!(
        operations::default_remote(&repo.root, None).as_deref(),
        Some("origin")
    );
    assert_eq!(
        operations::remote_url("origin", &repo.root),
        Some(format!("{}/.git", origin.root))
    );

    operations::create_tag("v1", "HEAD", None, false, &repo.root).unwrap();
    operations::create_tag("v2", "HEAD", Some("two"), false, &repo.root).unwrap();
    operations::push_tag("v1", "origin", false, &repo.root, None).unwrap();
    assert_eq!(origin.git(&["tag"]).unwrap(), "v1");
    operations::push_all_tags("origin", &repo.root, None).unwrap();
    assert_eq!(origin.git(&["tag"]).unwrap(), "v1\nv2");
    operations::delete_remote_tag("v1", "origin", &repo.root).unwrap();
    assert_eq!(origin.git(&["tag"]).unwrap(), "v2");
    assert_eq!(repo.git(&["tag"]).unwrap(), "v1\nv2", "local tags stay");
}

#[test]
fn push_follows_annotated_tags() {
    let (repo, origin, _seed) = clone_with_origin();
    repo.commit(&[("b.txt", "b\n")], "second");
    operations::create_tag("light", "HEAD", None, false, &repo.root).unwrap();
    operations::create_tag("release", "HEAD", Some("release"), false, &repo.root).unwrap();
    operations::push(
        &PushOptions {
            follow_tags: true,
            ..Default::default()
        },
        &repo.root,
        None,
    )
    .unwrap();
    assert_eq!(origin.git(&["rev-parse", "main"]).unwrap(), repo.head());
    assert_eq!(
        origin.git(&["tag"]).unwrap(),
        "release",
        "only annotated tags follow"
    );
}

#[test]
fn pull_modes() {
    let (repo, _origin, seed) = clone_with_origin();
    // Upstream moves on.
    seed.commit(&[("a.txt", "1\nupstream\n")], "upstream");
    seed.git(&["push", "-q", "origin", "main"]).unwrap();

    operations::fetch(true, None, &repo.root, Duration::from_secs(600), None).unwrap();
    assert_eq!(
        repo.git(&["rev-list", "--count", "main..origin/main"])
            .unwrap(),
        "1"
    );
    operations::pull(PullMode::FastForwardOnly, &repo.root, None).unwrap();
    assert_eq!(repo.head(), seed.head());

    // Diverged: fast-forward only refuses; rebase replays; merge merges.
    seed.commit(&[("c.txt", "c\n")], "upstream 2");
    seed.git(&["push", "-q", "origin", "main"]).unwrap();
    repo.commit(&[("d.txt", "d\n")], "local");
    assert!(
        operations::pull(PullMode::FastForwardOnly, &repo.root, None).is_err(),
        "ff-only refuses a diverged branch"
    );
    operations::pull(PullMode::Rebase, &repo.root, None).unwrap();
    assert_eq!(
        repo.git(&["log", "--format=%s", "-3"]).unwrap(),
        "local\nupstream 2\nupstream"
    );
    assert_eq!(
        repo.git(&["rev-list", "--merges", "--count", "HEAD"])
            .unwrap(),
        "0"
    );

    seed.commit(&[("e.txt", "e\n")], "upstream 3");
    seed.git(&["push", "-q", "origin", "main"]).unwrap();
    operations::pull(PullMode::Merge, &repo.root, None).unwrap();
    assert_eq!(
        repo.git(&["rev-list", "--merges", "--count", "HEAD"])
            .unwrap(),
        "1"
    );
    assert!(repo.exists("e.txt") && repo.exists("d.txt"));
}

#[test]
fn merge_and_rebase_branches() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n")], "base");
    repo.git(&["switch", "-q", "-c", "topic"]).unwrap();
    repo.commit(&[("t.txt", "t\n")], "topic work");
    repo.git(&["switch", "-q", "main"]).unwrap();
    repo.commit(&[("m.txt", "m\n")], "main work");

    // Rebase topic onto main: its commit now sits on main's.
    repo.git(&["switch", "-q", "topic"]).unwrap();
    operations::rebase("main", &repo.root).unwrap();
    assert_eq!(
        repo.git(&["log", "--format=%s", "-3"]).unwrap(),
        "topic work\nmain work\nbase"
    );

    // Merge it back: a fast-forward, or a merge commit with --no-ff.
    repo.git(&["switch", "-q", "main"]).unwrap();
    repo.git(&["branch", "-q", "topic-copy", "topic"]).unwrap();
    operations::merge("topic", false, &repo.root).unwrap();
    assert_eq!(repo.head(), repo.git(&["rev-parse", "topic"]).unwrap());
    repo.git(&["reset", "-q", "--hard", "HEAD~1"]).unwrap();
    operations::merge("topic-copy", true, &repo.root).unwrap();
    assert_eq!(
        repo.git(&["rev-list", "--merges", "--count", "HEAD"])
            .unwrap(),
        "1"
    );

    // A conflicting merge stops with the merge open.
    repo.git(&["switch", "-q", "-c", "clash", "HEAD~1"])
        .unwrap();
    repo.commit(&[("m.txt", "clash\n")], "clash");
    repo.git(&["switch", "-q", "main"]).unwrap();
    repo.commit(&[("m.txt", "mine\n")], "mine");
    match operations::merge("clash", false, &repo.root) {
        Err(OperationError::Cli(error)) => assert_eq!(error.kind, ErrorKind::MergeConflict),
        other => panic!("expected a conflict, got {other:?}"),
    }
    assert_eq!(
        snapshot(&repo.root, true).unwrap().operation,
        Some(RepoOperation::Merge)
    );
}

#[test]
fn taking_a_side_can_be_reopened() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("m.txt", "base\n")], "base");
    repo.git(&["switch", "-q", "-c", "clash"]).unwrap();
    repo.commit(&[("m.txt", "theirs\n")], "theirs");
    repo.git(&["switch", "-q", "main"]).unwrap();
    repo.commit(&[("m.txt", "ours\n")], "ours");
    assert!(
        operations::merge("clash", false, &repo.root).is_err(),
        "expected a conflict"
    );
    let stages = repo.git(&["ls-files", "-u", "--", "m.txt"]).unwrap();
    // Half-resolved by hand before taking a side.
    let edited = repo.read("m.txt").replace("ours\n", "ours, edited\n");
    repo.write("m.txt", &edited);
    let snapshot_before = safety::create("take incoming side", &repo.root, OPLOG_PREFIX).unwrap();
    assert_eq!(
        snapshot_before.index_tree, None,
        "no index tree while files are in conflict"
    );

    let point = operations::conflict_point(&repo.root);
    assert_eq!(point.operation, Some(RepoOperation::Merge));
    operations::resolve_conflicts(&["m.txt"], false, &repo.root).unwrap();
    assert_eq!(repo.read("m.txt"), "theirs\n");
    assert!(
        repo.git(&["ls-files", "-u"]).unwrap().is_empty(),
        "resolved and staged"
    );

    // Undo: the conflict is back in the index, markers in the file.
    operations::reopen_conflicts(&["m.txt"], &point, &repo.root).unwrap();
    assert_eq!(
        repo.git(&["ls-files", "-u", "--", "m.txt"]).unwrap(),
        stages
    );
    assert!(repo.read("m.txt").contains("<<<<<<<"));
    let conflicted: Vec<String> = snapshot(&repo.root, true)
        .unwrap()
        .conflicted
        .into_iter()
        .map(|c| c.path)
        .collect();
    assert_eq!(conflicted, ["m.txt"]);
    // And the snapshot brings back the hand edits.
    safety::restore(&snapshot_before, &["m.txt"], &repo.root).unwrap();
    assert_eq!(repo.read("m.txt"), edited);
    assert_eq!(
        repo.git(&["ls-files", "-u", "--", "m.txt"]).unwrap(),
        stages
    );
}

#[test]
fn reopening_is_refused_once_the_operation_moved_on() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("m.txt", "base\n"), ("n.txt", "base\n")], "base");
    repo.git(&["switch", "-q", "-c", "clash"]).unwrap();
    repo.commit(&[("m.txt", "theirs\n")], "theirs m");
    repo.commit(&[("n.txt", "theirs\n")], "theirs n");
    repo.git(&["switch", "-q", "main"]).unwrap();
    repo.commit(&[("m.txt", "ours\n"), ("n.txt", "ours\n")], "ours");
    let resolve = |paths: &[&str], start: &dyn Fn() -> operations::GitResult| -> ConflictPoint {
        assert!(start().is_err(), "expected a conflict");
        let point = operations::conflict_point(&repo.root);
        operations::resolve_conflicts(paths, false, &repo.root).unwrap();
        point
    };
    let expect_refused = |paths: &[&str], point: &ConflictPoint, phrase: &str| {
        match operations::reopen_conflicts(paths, point, &repo.root) {
            Err(OperationError::Stale(message)) => assert!(message.contains(phrase), "{message}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
        let mut args = vec!["ls-files", "-u", "--"];
        args.extend(paths);
        assert!(
            repo.git(&args).unwrap().is_empty(),
            "the conflict stayed resolved"
        );
    };

    // The merge was committed.
    let point = resolve(&["m.txt", "n.txt"], &|| {
        operations::merge("clash", false, &repo.root)
    });
    assert_eq!(point.operation, Some(RepoOperation::Merge));
    repo.git(&["commit", "-q", "--no-edit"]).unwrap();
    expect_refused(
        &["m.txt", "n.txt"],
        &point,
        "merge is no longer in progress",
    );

    // The merge was aborted (HEAD didn't move).
    repo.git(&["reset", "-q", "--hard", "HEAD~1"]).unwrap();
    let point = resolve(&["m.txt", "n.txt"], &|| {
        operations::merge("clash", false, &repo.root)
    });
    repo.git(&["merge", "--abort"]).unwrap();
    expect_refused(
        &["m.txt", "n.txt"],
        &point,
        "merge is no longer in progress",
    );

    // The rebase went on to its next step, which stopped on a conflict too.
    repo.git(&["switch", "-q", "clash"]).unwrap();
    let point = resolve(&["m.txt"], &|| operations::rebase("main", &repo.root));
    let _ = repo.git(&["-c", "core.editor=true", "rebase", "--continue"]);
    assert_eq!(
        current_operation(&repo.root),
        Some(RepoOperation::Rebase {
            step: Some(2),
            total: Some(2)
        })
    );
    expect_refused(&["m.txt"], &point, "rebase has moved on");
}

#[test]
fn default_remote_follows_the_branch() {
    let (repo, origin, _seed) = clone_with_origin();
    repo.git(&["remote", "add", "fork", &format!("{}/.git", origin.root)])
        .unwrap();
    repo.git(&["branch", "-q", "topic"]).unwrap();
    repo.git(&["config", "branch.topic.remote", "fork"])
        .unwrap();
    repo.git(&["branch", "-q", "loose"]).unwrap();

    let default_remote = |branch: Option<&str>| operations::default_remote(&repo.root, branch);
    assert_eq!(
        default_remote(None).as_deref(),
        Some("origin"),
        "main tracks origin"
    );
    assert_eq!(default_remote(Some("topic")).as_deref(), Some("fork"));
    assert_eq!(
        default_remote(Some("loose")).as_deref(),
        Some("origin"),
        "no remote of its own"
    );
    repo.git(&["switch", "-q", "topic"]).unwrap();
    assert_eq!(
        default_remote(None).as_deref(),
        Some("fork"),
        "the checked-out branch's"
    );

    // All at once, the same answers.
    repo.git(&["branch", "-q", "release.1.2"]).unwrap();
    repo.git(&["config", "branch.release.1.2.remote", "fork"])
        .unwrap();
    repo.git(&["branch", "-q", "orphaned"]).unwrap();
    repo.git(&["config", "branch.orphaned.remote", "gone"])
        .unwrap();
    let names = ["main", "topic", "loose", "release.1.2", "orphaned"];
    let all = operations::default_remotes(&repo.root, &names);
    let expected: HashMap<String, String> = [
        ("main", "origin"),
        ("topic", "fork"),
        ("loose", "origin"),
        ("release.1.2", "fork"),
        ("orphaned", "origin"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    assert_eq!(all, expected);
    for name in names {
        assert_eq!(all.get(name).cloned(), default_remote(Some(name)), "{name}");
    }
}
