//! What `impulse tasks` says, and the hooks' replies (the Swift
//! `TaskListingTests`).

use impulse_protocol::agent_tools::{hook_reply, TaskListing, Workspace};
use serde_json::{json, Value};

fn listing() -> TaskListing {
    TaskListing {
        repository: "pulseboard".into(),
        workspaces: vec![
            Workspace {
                name: "pulseboard".into(),
                path: "/code/pulseboard".into(),
                branch: Some("main".into()),
                head: Some("7e7a97a1".into()),
                is_main_checkout: true,
                agent: Some("Claude Code".into()),
                agent_state: Some("working".into()),
                uncommitted: 156,
                shared_with_caller: vec![
                    "Pages/Teams/Show.tsx".into(),
                    "Pages/Boards/Show.tsx".into(),
                ],
                ..Default::default()
            },
            Workspace {
                name: "interia-upgrade".into(),
                path: "/code/pulseboard.worktrees/interia-upgrade".into(),
                branch: Some("interia-upgrade".into()),
                base: Some("origin/main".into()),
                head: Some("c5aa8141".into()),
                is_caller: true,
                uncommitted: 2,
                ahead: Some(4),
                behind: Some(1),
                ..Default::default()
            },
        ],
    }
}

#[test]
fn text_lists_each_workspace() {
    let text = listing().text();
    assert!(text.starts_with("pulseboard: 2 workspaces"));
    assert!(text.contains("pulseboard (main checkout)\n  branch main at 7e7a97a"));
    assert!(text.contains("  Claude Code: working\n  156 uncommitted files"));
    assert!(
        text.contains("also changes files you change: Pages/Teams/Show.tsx, Pages/Boards/Show.tsx")
    );
    assert!(text.contains(
        "interia-upgrade (task) ← you\n  branch interia-upgrade from origin/main, 4 ahead, 1 behind"
    ));
}

#[test]
fn json_round_trips() {
    let listing = listing();
    let decoded: TaskListing = serde_json::from_str(&listing.json()).unwrap();
    assert_eq!(decoded, listing);
}

#[test]
fn a_summary_for_the_agent_starting_up() {
    let listing = listing();
    let summary = listing.session_summary().unwrap();
    assert!(summary.starts_with("Impulse: you're in the task interia-upgrade of pulseboard: a git worktree on branch interia-upgrade, made from origin/main."));
    assert!(summary.contains(
        "pulseboard (main checkout) — Claude Code working, 156 uncommitted, changes 2 of your files"
    ));
    assert!(summary.ends_with("to see what they're changing."));
    let alone = TaskListing {
        repository: "x".into(),
        workspaces: vec![listing.workspaces[1].clone()],
    };
    assert_eq!(alone.session_summary(), None, "alone: nothing to say");
}

#[test]
fn hook_replies_are_claude_code_json() {
    let context: Value =
        serde_json::from_str(&hook_reply::context("a \"note\"", "PostToolUse")).unwrap();
    assert_eq!(
        context["hookSpecificOutput"],
        json!({"hookEventName": "PostToolUse", "additionalContext": "a \"note\""})
    );
    let claude_ask: Value = serde_json::from_str(&hook_reply::ask("still moving")).unwrap();
    assert_eq!(
        claude_ask["hookSpecificOutput"]["permissionDecision"],
        "ask"
    );
    // Impulse's opencode plugin reads plain keys.
    let note: Value = serde_json::from_str(&hook_reply::opencode_context("a note")).unwrap();
    assert_eq!(note, json!({"context": "a note"}));
    let ask: Value = serde_json::from_str(&hook_reply::opencode_ask("still moving")).unwrap();
    assert_eq!(
        ask,
        json!({"ask": "still moving"}),
        "opencode asks you, with Impulse's reason"
    );
    let deny: Value = serde_json::from_str(&hook_reply::deny("read it again")).unwrap();
    assert_eq!(
        deny["hookSpecificOutput"],
        json!({
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": "read it again",
        })
    );
    let stop: Value = serde_json::from_str(&hook_reply::opencode_deny("read it again")).unwrap();
    assert_eq!(stop, json!({"deny": "read it again"}));
}

// Beyond the Swift tests: the JSON is the Swift app's.

#[test]
fn json_is_foundations_pretty_sorted_json() {
    let listing = TaskListing {
        repository: "r".into(),
        workspaces: vec![Workspace {
            name: "r".into(),
            path: "/code/r".into(),
            is_main_checkout: true,
            is_caller: true,
            ..Default::default()
        }],
    };
    assert_eq!(
        listing.json(),
        "{\n  \"repository\" : \"r\",\n  \"workspaces\" : [\n    {\n      \"isCaller\" : true,\n      \"isMainCheckout\" : true,\n      \"name\" : \"r\",\n      \"path\" : \"/code/r\",\n      \"sharedWithCaller\" : [\n\n      ],\n      \"uncommitted\" : 0\n    }\n  ]\n}"
    );
    assert_eq!(
        hook_reply::ask("a/b"),
        r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"ask","permissionDecisionReason":"a/b"}}"#
    );
}
