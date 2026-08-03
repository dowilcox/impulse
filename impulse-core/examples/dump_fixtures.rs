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
use std::fs;
use std::path::{Path, PathBuf};

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


