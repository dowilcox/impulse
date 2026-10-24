//! What Impulse answers agents with (the MCP tools and their `impulse`
//! commands): the answer half of the Swift `AgentToolTests`.

use std::collections::BTreeMap;
use std::time::{Duration, UNIX_EPOCH};

use impulse_protocol::agent_tools::{
    self, BlockList, BlockOutput, CommentEntry, CommentList, Commit, Detected, Problem,
    ProblemList, ProjectScripts, ProjectSummary, Resolution, ReviewComment, Rule, Severity, Side,
    TaskDetail, TaskListing, Turn, TurnList, UnsavedFile, UnsavedList, Workspace,
};
use impulse_protocol::mcp::tools;

fn task() -> Workspace {
    Workspace {
        name: "fix-elevation".into(),
        path: "/code/trailhead.worktrees/fix-elevation".into(),
        branch: Some("fix-elevation".into()),
        base: Some("origin/main".into()),
        head: Some("9b1d4e07".into()),
        agent: Some("Claude Code".into()),
        agent_state: Some("working".into()),
        uncommitted: 1,
        ahead: Some(3),
        behind: Some(0),
        created_by: Some("agent".into()),
        requested_by: Some("Codex in trailhead (main checkout)".into()),
        brief: Some("Fix elevation units".into()),
        ..Default::default()
    }
}

#[test]
fn turn_list_shows_the_latest_and_counts_the_rest() {
    let all: Vec<Turn> = (1..=12)
        .map(|n| Turn {
            agent: "Claude Code".into(),
            started: UNIX_EPOCH + Duration::from_secs(n * 60),
            ended: None,
            start: format!("s{n}"),
            end: Some(format!("e{n}")),
        })
        .collect();
    let list = TurnList::latest(all.clone(), 3);
    let starts: Vec<&str> = list.turns.iter().map(|t| t.start.as_str()).collect();
    assert_eq!(starts, ["s10", "s11", "s12"], "the latest, oldest first");
    assert_eq!(list.older, 9);
    let text = list.text();
    assert!(
        text.starts_with("(9 earlier turns not shown)\n10. Claude Code"),
        "numbered as the terminal's turns: {text}"
    );
    assert!(text.contains("10. Claude Code, 1970-01-01T00:10:00Z: s10 e10"));
    assert_eq!(TurnList::latest(all, 50).older, 0);
    assert_eq!(
        TurnList::latest(Vec::new(), 10).text(),
        "No agent turns recorded in this terminal yet."
    );
}

#[test]
fn task_detail_text() {
    let detail = TaskDetail {
        changed_files: vec!["src/lib/units.ts".into()],
        uncommitted_files: vec!["notes.md".into()],
        commits: vec![Commit {
            id: "9b1d4e07aa".into(),
            subject: "Use feet".into(),
        }],
        more_commits: 2,
        merged: Some(false),
        slot: Some(1),
        ports: BTreeMap::from([("APP_PORT".to_string(), 8100)]),
        brief: Some("Fix elevation units\nOn the trail page.".into()),
        ..TaskDetail::new(task())
    };
    let text = detail.text();
    assert!(text.starts_with("fix-elevation (task)"));
    assert!(text.contains("  started at the request of Codex in trailhead (main checkout)"));
    assert!(text.contains("  not merged into its base yet"));
    assert!(text.contains("  slot 1: APP_PORT 8100"));
    assert!(text.contains("  9b1d4e0 Use feet\n  …and 2 more"));
    assert!(text.contains("Changed against its base:\n  src/lib/units.ts"));
    assert!(text.contains("Brief:\n  Fix elevation units\n  On the trail page."));
    let decoded: TaskDetail = serde_json::from_slice(&agent_tools::data(&detail)).unwrap();
    assert_eq!(decoded, detail);
}

#[test]
fn listings_carry_provenance() {
    let listing = TaskListing {
        repository: "trailhead".into(),
        workspaces: vec![task()],
    };
    assert!(listing.text().contains("  brief: Fix elevation units"));
    let decoded: TaskListing = serde_json::from_str(&listing.json()).unwrap();
    assert_eq!(decoded.workspaces[0].created_by.as_deref(), Some("agent"));
    // A listing from before provenance reads as before.
    let old = r#"{"repository":"r","workspaces":[{"name":"r","path":"/r","isMainCheckout":true,"isCaller":true,"uncommitted":0,"sharedWithCaller":[]}]}"#;
    let old: TaskListing = serde_json::from_str(old).unwrap();
    assert_eq!(old.workspaces[0].created_by, None);
}

#[test]
fn comments_list_with_resolution_and_freshness() {
    let mut comment = ReviewComment::new("c1", "src/a.ts", Side::New, 2, 2, "let x = 1", "Why 1?");
    assert_eq!(comment.freshness(Some("a\nlet x = 1\nb")), Some(false));
    assert_eq!(comment.freshness(Some("a\nlet x = 2\nb")), Some(true));
    assert_eq!(comment.freshness(None), Some(true));
    let removed = ReviewComment::new("c2", "src/a.ts", Side::Old, 1, 1, "y", "Gone?");
    assert_eq!(removed.freshness(Some("z")), None);

    comment.resolution = Some(Resolution {
        by: "Claude Code".into(),
        note: "Made it a constant.".into(),
        at: UNIX_EPOCH,
    });
    let list = CommentList {
        repository: "trailhead".into(),
        comments: vec![CommentEntry::new(&comment, Some(false))],
    };
    let text = list.text();
    assert!(text.contains("[c1] src/a.ts:2 (addressed by Claude Code)"));
    assert!(text.contains("```\nlet x = 1\n```\nWhy 1?\n"));
    assert!(text.contains("Addressed: Made it a constant."));
    // Comments saved before resolutions existed.
    let saved = r#"[{"id":"x","path":"p","side":"new","line":1,"endLine":1,"snippet":"","text":"t","createdAt":"2026-10-01T00:00:00Z"}]"#;
    let saved: Vec<ReviewComment> = serde_json::from_str(saved).unwrap();
    assert_eq!(saved[0].resolution, None);
    assert_eq!(
        saved[0].created_at,
        UNIX_EPOCH + Duration::from_secs(1_790_812_800)
    );
    // A fence the snippet can't close.
    assert_eq!(agent_tools::code_fence("a ```` b"), "`````");
}

#[test]
fn problems_are_scoped_sorted_and_limited() {
    let mut problems = vec![
        Problem::new("/w/b.ts", 3, 1, Severity::Warning, "unused"),
        Problem {
            source: Some("ts".into()),
            code: Some("2322".into()),
            ..Problem::new("/w/a.ts", 9, 2, Severity::Error, "type")
        },
        Problem::new("/other/c.ts", 1, 1, Severity::Error, "elsewhere"),
    ];
    let checked = vec!["/w/a.ts".to_string(), "/w/b.ts".to_string()];
    let list = ProblemList::new(&problems, "/w", None, None, &checked, None);
    let paths: Vec<&str> = list.problems.iter().map(|p| p.path.as_str()).collect();
    assert_eq!(paths, ["a.ts", "b.ts"]);
    assert!(list.counts.errors == 1 && list.counts.warnings == 1);
    assert!(list.text().contains("a.ts:9:2 error [ts 2322]: type"));
    assert_eq!(list.checked_files, ["a.ts", "b.ts"]);
    let list = ProblemList::new(&problems, "/w", None, Some(Severity::Error), &[], None);
    assert_eq!(list.problems.len(), 1);
    for index in 0..(ProblemList::LIMIT as i64 + 5) {
        problems.push(Problem::new("/w/many.ts", index, 1, Severity::Hint, "h"));
    }
    let list = ProblemList::new(&problems, "/w", None, None, &[], None);
    assert_eq!(list.problems.len(), ProblemList::LIMIT);
    assert_eq!(list.truncated, 7);
    assert!(list.text().ends_with("…and 7 more.\n"));
    let empty = ProblemList::new(
        &[],
        "/w",
        None,
        None,
        &[],
        Some("Language servers don't run here.".into()),
    );
    assert_eq!(empty.text(), "Language servers don't run here.");
}

#[test]
fn block_output_keeps_the_tail() {
    let output = (1..=300)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let block = BlockOutput::new(
        "ab:1",
        Some("npm test".into()),
        Some("/w".into()),
        Some(1),
        false,
        &output,
        200,
    );
    assert_eq!(block.total_lines, 300);
    assert_eq!(block.shown_lines, 200);
    assert!(block.output.starts_with("line 101\n"));
    assert!(block
        .text()
        .starts_with("`npm test` in /w (exit status 1), last 200 of 300 lines:"));
    assert_eq!(
        BlockOutput::new("x", None, None, Some(0), false, "", 5).text(),
        "`a command` (succeeded): no output."
    );
    assert_eq!(
        BlockList::parse("ab12cd34:1042"),
        Some(("ab12cd34".to_string(), 1042))
    );
    assert_eq!(BlockList::parse("1042"), None);
}

#[test]
fn project_summary_lists_commands_only_when_trusted() {
    let config = ProjectScripts {
        check_script: Some("npm test".into()),
        on_change: BTreeMap::from([("package-lock.json".to_string(), "npm ci".to_string())]),
        ports: BTreeMap::from([("APP_PORT".to_string(), 8000)]),
        ..Default::default()
    };
    let detected = Detected {
        lock_files: vec!["package-lock.json".into()],
        install: vec!["npm ci".into()],
        compose_file: None,
    };
    let files = vec![".impulse/project.toml".to_string()];
    let trusted = ProjectSummary::new(
        Some(&config),
        true,
        files.clone(),
        Some(2),
        detected.clone(),
        None,
    );
    assert_eq!(trusted.check.as_deref(), Some("npm test"));
    assert_eq!(
        trusted.on_change,
        [Rule {
            file: "package-lock.json".into(),
            command: "npm ci".into()
        }]
    );
    assert_eq!(trusted.ports["APP_PORT"], 8200);
    assert!(trusted
        .text()
        .contains("this task's slot: 2 (APP_PORT 8200)"));
    let untrusted = ProjectSummary::new(Some(&config), false, files, Some(2), detected, None);
    assert!(
        untrusted.check.is_none() && untrusted.on_change.is_empty() && untrusted.ports.is_empty()
    );
    assert_eq!(untrusted.env_file.as_deref(), Some(".env"));
    assert!(untrusted
        .text()
        .contains("lock files: package-lock.json (install: npm ci)"));
}

#[test]
fn create_task_names_every_known_agent() {
    // The Swift `KnownAgents.builtIn` ids.
    let known = [
        "claude", "codex", "gemini", "aider", "opencode", "amp", "copilot", "cursor", "goose",
        "qwen", "crush",
    ];
    let tool = tools::named("create_task").unwrap();
    let description = tool.input_schema["properties"]["agent"]["description"]
        .as_str()
        .unwrap();
    for agent in known {
        assert!(description.contains(agent), "{agent}");
    }
}

// Beyond the Swift tests: the JSON is the Swift app's.

#[test]
fn answers_encode_as_the_swift_app_encodes_them() {
    let list = UnsavedList::new(vec![
        UnsavedFile {
            path: "src/z.ts".into(),
            changed_on_disk: false,
        },
        UnsavedFile {
            path: "src/a.ts".into(),
            changed_on_disk: true,
        },
    ]);
    assert_eq!(
        String::from_utf8(agent_tools::data(&list)).unwrap(),
        r#"{"files":[{"changedOnDisk":true,"path":"src/a.ts"},{"changedOnDisk":false,"path":"src/z.ts"}]}"#
    );
    assert!(list
        .text()
        .contains("  src/a.ts (also changed on disk)\n  src/z.ts"));
    let turns = TurnList {
        turns: vec![Turn {
            agent: "Codex".into(),
            started: UNIX_EPOCH + Duration::from_secs(1_790_812_800),
            ended: None,
            start: "refs/impulse/checkpoints/a".into(),
            end: None,
        }],
        older: 0,
    };
    assert_eq!(
        String::from_utf8(agent_tools::data(&turns)).unwrap(),
        r#"{"older":0,"turns":[{"agent":"Codex","start":"refs/impulse/checkpoints/a","started":"2026-10-01T00:00:00Z"}]}"#
    );
    let decoded: TurnList = serde_json::from_slice(&agent_tools::data(&turns)).unwrap();
    assert_eq!(decoded, turns);
    assert_eq!(
        agent_tools::pretty(&BTreeMap::from([("disabled", Vec::<String>::new())])),
        "{\n  \"disabled\" : [\n\n  ]\n}"
    );
}
