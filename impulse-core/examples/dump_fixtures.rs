//! Golden-fixture dumper for the Swift rewrite.
//!
//! Captures the current Rust implementation's outputs as JSON fixtures so the
//! Swift ports (ImpulseKit / ImpulseGit) can assert behavior parity. Run on
//! macOS from the workspace root:
//!
//! ```sh
//! cargo run -p impulse-core --example dump_fixtures -- /tmp/fixtures
//! ```
//!
//! Writes two subdirectories:
//! - `kit/` → impulse-macos/Tests/ImpulseKitTests/Fixtures/
//! - `git/` → impulse-macos/Tests/ImpulseGitTests/Fixtures/
//!
//! Determinism: git fixtures use pinned author/committer dates and an isolated
//! config, machine-specific paths are replaced with `$ROOT`, and file mtimes
//! are stripped from file-tree fixtures.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let out = std::env::args()
        .nth(1)
        .expect("usage: dump_fixtures <output-dir>");
    let out = PathBuf::from(out);
    let kit = out.join("kit");
    let git = out.join("git");
    fs::create_dir_all(&kit).expect("create kit dir");
    fs::create_dir_all(&git).expect("create git dir");

    dump_shell_parser(&kit);
    // dump_close_risk / dump_palette / dump_glob were removed along with the
    // Rust modules they exercised (ported to ImpulseKit in Phase 1); their
    // fixtures remain committed under Tests/ImpulseKitTests/Fixtures.
    dump_util(&kit);
    dump_file_tree(&kit);
    dump_git(&git);

    println!("Fixtures written to {}", out.display());
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create fixture dir");
    }
    let mut text = serde_json::to_string_pretty(value).expect("serialize fixture");
    text.push('\n');
    fs::write(path, text).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

fn to_value<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).expect("to_value")
}

/// Replace the scratch root with `$ROOT` in every string in the JSON tree.
fn replace_root(v: &mut Value, root: &str) {
    match v {
        Value::String(s) => {
            if s.contains(root) {
                *s = s.replace(root, "$ROOT");
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|x| replace_root(x, root)),
        Value::Object(map) => map.values_mut().for_each(|x| replace_root(x, root)),
        _ => {}
    }
}

/// Remove machine-dependent `modified` (mtime) keys from FileEntry-shaped objects.
fn strip_mtimes(v: &mut Value) {
    match v {
        Value::Array(items) => items.iter_mut().for_each(strip_mtimes),
        Value::Object(map) => {
            map.remove("modified");
            map.values_mut().for_each(strip_mtimes);
        }
        _ => {}
    }
}

/// A scratch directory that is removed on drop. Canonicalized so paths embedded
/// in outputs match what the library observes (`/var` vs `/private/var`).
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "impulse-fixture-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create scratch dir");
        let path = path.canonicalize().expect("canonicalize scratch dir");
        Scratch { path }
    }

    fn root(&self) -> &str {
        self.path.to_str().expect("scratch path is UTF-8")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

// ---------------------------------------------------------------------------
// Shell parser (Phase 5 parity)
// ---------------------------------------------------------------------------

fn dump_shell_parser(out: &Path) {
    // (input, cursor). usize::MAX means "cursor at end of input".
    let corpus: Vec<(&str, usize)> = vec![
        ("", 0),
        ("ls", usize::MAX),
        ("ls -la ./src", usize::MAX),
        ("git commit -m 'hello world'", usize::MAX),
        ("FOO=bar BAZ=qux cargo build --release", usize::MAX),
        ("echo \"a b\" | grep a > out.txt 2>&1", usize::MAX),
        ("cat < in.txt >> out.log", usize::MAX),
        ("cd ~/pro", usize::MAX),
        ("echo 'unclosed", usize::MAX),
        ("echo \"unclosed double", usize::MAX),
        ("a | b | c", 6),
        ("git checkout main", 7),
        ("git checkout main", 3),
        ("echo héllo wörld", usize::MAX),
        ("./run.sh --flag=value", usize::MAX),
        ("VAR=1", usize::MAX),
        ("ls  ", usize::MAX),
        ("grep -r \"needle\" src/", usize::MAX),
    ];

    let results: Vec<Value> = corpus
        .iter()
        .map(|(input, cursor)| {
            let cursor = if *cursor == usize::MAX {
                input.len()
            } else {
                *cursor
            };
            to_value(&impulse_core::shell_parser::parse_shell_input(input, cursor))
        })
        .collect();
    write_json(&out.join("shell_parser.json"), &Value::Array(results));
}

// ---------------------------------------------------------------------------
// URI / language-id utilities (Phase 6 parity)
// ---------------------------------------------------------------------------

fn dump_util(out: &Path) {
    let paths = [
        "/Users/dev/project/src/main.rs",
        "/tmp/with space/file.txt",
        "/tmp/unicode/héllo.md",
        "/tmp/percent%file.js",
    ];
    let path_to_uri: Vec<Value> = paths
        .iter()
        .map(|p| {
            json!({
                "path": p,
                "uri": impulse_core::util::file_path_to_uri(Path::new(p)),
            })
        })
        .collect();
    write_json(&out.join("path_to_uri.json"), &Value::Array(path_to_uri));

    let uris = [
        "file:///Users/dev/project/src/main.rs",
        "file:///tmp/with%20space/file.txt",
        "file:///tmp/unicode/h%C3%A9llo.md",
        "untitled:Untitled-1",
    ];
    let uri_to_path: Vec<Value> = uris
        .iter()
        .map(|u| {
            json!({
                "uri": u,
                "path": impulse_core::util::uri_to_file_path(u),
            })
        })
        .collect();
    write_json(&out.join("uri_to_path.json"), &Value::Array(uri_to_path));

    let lang_uris = [
        "file:///a/b.rs",
        "file:///a/b.ts",
        "file:///a/b.tsx",
        "file:///a/b.py",
        "file:///a/b.swift",
        "file:///a/b.md",
        "file:///a/b.json",
        "file:///a/b.yml",
        "file:///a/b.sh",
        "file:///a/b.c",
        "file:///a/b.cpp",
        "file:///a/b.go",
        "file:///a/Makefile",
        "file:///a/Dockerfile",
        "file:///a/b.unknownext",
        "file:///a/b",
    ];
    let languages: Vec<Value> = lang_uris
        .iter()
        .map(|u| {
            json!({
                "uri": u,
                "language": impulse_core::util::language_from_uri(u),
            })
        })
        .collect();
    write_json(&out.join("language_from_uri.json"), &Value::Array(languages));
}

// ---------------------------------------------------------------------------
// File tree patches (Phase 3 parity)
// ---------------------------------------------------------------------------

fn dump_file_tree(out: &Path) {
    let scratch = Scratch::new("tree");
    let root = scratch.root().to_string();
    let rp = |rel: &str| scratch.path.join(rel);

    fs::create_dir_all(rp("alpha")).unwrap();
    fs::create_dir_all(rp("zeta")).unwrap();
    fs::write(rp("alpha/a1.txt"), "alpha one\n").unwrap();
    fs::write(rp("beta.txt"), "beta\n").unwrap();
    fs::write(rp("zeta/z1.txt"), "zeta one\n").unwrap();
    fs::write(rp(".hidden"), "hidden\n").unwrap();

    let alpha = rp("alpha").to_str().unwrap().to_string();
    let mut before: HashMap<String, Vec<impulse_core::filesystem::FileEntry>> = HashMap::new();
    before.insert(
        root.clone(),
        impulse_core::filesystem::read_directory_entries(&root, false).unwrap(),
    );
    before.insert(
        alpha.clone(),
        impulse_core::filesystem::read_directory_entries(&alpha, false).unwrap(),
    );

    let mut before_json = json!({
        "root": to_value(before.get(&root).unwrap()),
        "alpha": to_value(before.get(&alpha).unwrap()),
    });

    // Mutations observed by the (simulated) filesystem watcher.
    fs::write(rp("gamma.txt"), "gamma\n").unwrap();
    fs::remove_file(rp("beta.txt")).unwrap();
    fs::write(rp("alpha/a2.txt"), "alpha two\n").unwrap();
    fs::write(rp("alpha/a1.txt"), "alpha one, edited\n").unwrap();

    use impulse_core::file_tree::{FileTreeWatchEvent, FileTreeWatchEventKind};
    let events = vec![
        FileTreeWatchEvent {
            kind: FileTreeWatchEventKind::Create,
            paths: vec![rp("gamma.txt").to_str().unwrap().to_string()],
        },
        FileTreeWatchEvent {
            kind: FileTreeWatchEventKind::Remove,
            paths: vec![rp("beta.txt").to_str().unwrap().to_string()],
        },
        FileTreeWatchEvent {
            kind: FileTreeWatchEventKind::Create,
            paths: vec![rp("alpha/a2.txt").to_str().unwrap().to_string()],
        },
        FileTreeWatchEvent {
            kind: FileTreeWatchEventKind::Modify,
            paths: vec![rp("alpha/a1.txt").to_str().unwrap().to_string()],
        },
    ];

    let batch =
        impulse_core::file_tree::build_patch_batch_from_filesystem(&root, &events, &before, false)
            .expect("patch batch");

    let mut events_json = to_value(&events);
    let mut batch_json = to_value(&batch);
    for v in [&mut before_json, &mut events_json, &mut batch_json] {
        replace_root(v, &root);
        strip_mtimes(v);
    }

    write_json(
        &out.join("file_tree_patch.json"),
        &json!({
            "before_by_parent": before_json,
            "events": events_json,
            "batch": batch_json,
        }),
    );

    // Stable node id normalization corpus.
    let ids: Vec<Value> = ["/a/b/", "/a/b", "/a/b\\", "/", ""]
        .iter()
        .map(|p| json!({ "path": p, "id": impulse_core::file_tree::stable_node_id(p) }))
        .collect();
    write_json(&out.join("stable_node_id.json"), &Value::Array(ids));
}

// ---------------------------------------------------------------------------
// Git (Phase 3 parity)
// ---------------------------------------------------------------------------

fn run_git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@impulse.dev")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@impulse.dev")
        .env("GIT_AUTHOR_DATE", "2024-01-01T00:00:00 +0000")
        .env("GIT_COMMITTER_DATE", "2024-01-01T00:00:00 +0000")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed");
}

const SAMPLE_BASE: &str = "fn main() {
    let message = \"hello world\";
    println!(\"{}\", message);
    let total = compute(1, 2);
    println!(\"total = {}\", total);
}

fn compute(a: i32, b: i32) -> i32 {
    a + b
}
";

const SAMPLE_MODIFIED: &str = "fn main() {
    let message = \"hello swift world\";
    println!(\"{}\", message);
    let sum = compute(1, 2, 3);
    println!(\"sum = {}\", sum);
    log_result(sum);
}

fn compute(a: i32, b: i32, c: i32) -> i32 {
    a + b + c
}

fn log_result(value: i32) {
    eprintln!(\"result: {}\", value);
}
";

fn dump_git(out: &Path) {
    let scratch = Scratch::new("git");
    let root = scratch.root().to_string();
    let rp = |rel: &str| scratch.path.join(rel);

    run_git(&scratch.path, &["init", "-q", "-b", "main"]);
    fs::create_dir_all(rp("src")).unwrap();
    fs::write(rp("src/sample.rs"), SAMPLE_BASE).unwrap();
    fs::write(rp("notes.txt"), "alpha\nbeta\ngamma\n").unwrap();
    fs::write(rp("keep.md"), "# Keep\n\nUnchanged file.\n").unwrap();
    run_git(&scratch.path, &["add", "-A"]);
    run_git(&scratch.path, &["commit", "-q", "-m", "base"]);

    // Working-tree changes: modify (with intra-line word changes), delete, add.
    fs::write(rp("src/sample.rs"), SAMPLE_MODIFIED).unwrap();
    fs::remove_file(rp("notes.txt")).unwrap();
    fs::write(rp("extra.txt"), "brand new file\n").unwrap();

    // Scenario description so the Swift tests can reproduce the exact setup.
    write_json(
        &out.join("scenario.json"),
        &json!({
            "description": "git init -b main; commit src/sample.rs + notes.txt + keep.md; then modify src/sample.rs (word-level line edits + added fn), delete notes.txt, add untracked extra.txt",
            "author": { "name": "Fixture", "email": "fixture@impulse.dev", "date": "2024-01-01T00:00:00 +0000" },
            "base_files": {
                "src/sample.rs": SAMPLE_BASE,
                "notes.txt": "alpha\nbeta\ngamma\n",
                "keep.md": "# Keep\n\nUnchanged file.\n"
            },
            "worktree_changes": {
                "src/sample.rs": SAMPLE_MODIFIED,
                "notes.txt": null,
                "extra.txt": "brand new file\n"
            }
        }),
    );

    let change_set = impulse_core::git::list_changed_files(&root).expect("list_changed_files");
    let mut change_set_json = to_value(&change_set);
    replace_root(&mut change_set_json, &root);
    write_json(&out.join("changeset.json"), &change_set_json);

    for file in &change_set.files {
        let hunks = match impulse_core::git::file_hunks(&root, &file.path) {
            Ok(h) => to_value(&h),
            Err(e) => json!({ "error": e }),
        };
        let mut hunks_json = hunks;
        replace_root(&mut hunks_json, &root);
        let safe_name = file.path.replace('/', "__");
        write_json(&out.join(format!("hunks_{safe_name}.json")), &hunks_json);
    }

    // Diff markers for the modified file, mirroring impulse_git_diff_markers
    // (sorted for determinism — the FFI iterates a HashMap).
    let sample_abs = rp("src/sample.rs").to_str().unwrap().to_string();
    let diff = impulse_core::git::get_file_diff(&sample_abs).expect("get_file_diff");
    let mut markers: Vec<(u32, &str)> = diff
        .changed_lines
        .iter()
        .filter_map(|(&line, status)| match status {
            impulse_core::git::DiffLineStatus::Added => Some((line, "added")),
            impulse_core::git::DiffLineStatus::Modified => Some((line, "modified")),
            impulse_core::git::DiffLineStatus::Unchanged => None,
        })
        .collect();
    markers.extend(diff.deleted_lines.iter().map(|&line| (line, "deleted")));
    markers.sort();
    let markers_json: Vec<Value> = markers
        .iter()
        .map(|(line, status)| json!({ "line": line, "status": status }))
        .collect();
    write_json(
        &out.join("markers_src__sample.rs.json"),
        &Value::Array(markers_json),
    );

    // Branch queries.
    write_json(
        &out.join("branches.json"),
        &json!({
            "branch": impulse_core::git::get_git_branch(&root).ok().flatten(),
            "branches": impulse_core::git::list_git_branches(&root).unwrap_or_default(),
        }),
    );

    // Blame on the unchanged file (pinned dates make the commit hash stable).
    let keep_abs = rp("keep.md").to_str().unwrap().to_string();
    match impulse_core::git::get_line_blame(&keep_abs, 1) {
        Ok(blame) => write_json(
            &out.join("blame_keep.md.json"),
            &json!({
                "author": blame.author,
                "date": blame.date,
                "commitHash": blame.commit_hash,
                "summary": blame.summary,
            }),
        ),
        Err(e) => write_json(&out.join("blame_keep.md.json"), &json!({ "error": e })),
    }

    // Per-directory and repo-wide status maps (sorted via BTreeMap for
    // determinism — the FFI serializes HashMaps).
    let dir_status = impulse_core::filesystem::get_git_status_for_directory(&root)
        .unwrap_or_default()
        .into_iter()
        .collect::<std::collections::BTreeMap<_, _>>();
    write_json(&out.join("status_root_dir.json"), &to_value(&dir_status));

    let all_statuses = impulse_core::filesystem::get_all_git_statuses(&root)
        .unwrap_or_default()
        .into_iter()
        .map(|(dir, files)| {
            (
                dir.replace(&root, "$ROOT"),
                files
                    .into_iter()
                    .collect::<std::collections::BTreeMap<_, _>>(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    write_json(&out.join("status_all.json"), &to_value(&all_statuses));
}
