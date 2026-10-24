//! Helpers shared by the integration tests (the Swift tests' `TempRepo`,
//! `ScenarioRepo`, `FixtureSupport` and `JSONCompare`).
//!
//! `TempRepo` is a scratchpad: tests build whatever state they need with the
//! git CLI, which also serves as the oracle for expected results. A pinned
//! identity, date and config keep commits deterministic and keep the user's
//! global config (hooks, signing, aliases) out. `ScenarioRepo` rebuilds the
//! golden-fixture scenario exactly, so every run reproduces the commit the
//! fixtures recorded.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use impulse_git::{cli, process};
use serde_json::Value;

/// The golden fixtures, read in place from the Swift test tree so both
/// ports answer to one spec; never regenerate them from either port.
pub fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../impulse-macos/Tests/ImpulseGitTests/Fixtures")
        .join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap()
}

/// Every occurrence of `root` in strings (keys and values) as "$ROOT".
pub fn normalize(value: Value, root: &str) -> Value {
    match value {
        Value::String(s) => Value::String(s.replace(root, "$ROOT")),
        Value::Array(items) => {
            Value::Array(items.into_iter().map(|v| normalize(v, root)).collect())
        }
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k.replace(root, "$ROOT"), normalize(v, root)))
                .collect(),
        ),
        other => other,
    }
}

/// The first difference between two JSON values (objects compare by key,
/// arrays in order), or `None` when they're equal.
pub fn first_difference(actual: &Value, expected: &Value, path: &str) -> Option<String> {
    match (actual, expected) {
        (Value::Object(a), Value::Object(e)) => {
            let mut keys: Vec<&String> = a.keys().chain(e.keys()).collect();
            keys.sort();
            keys.dedup();
            keys.into_iter().find_map(|key| {
                let at = format!("{path}.{key}");
                match (a.get(key), e.get(key)) {
                    (None, Some(e)) => Some(format!("{at}: missing in actual (expected {e})")),
                    (Some(a), None) => Some(format!("{at}: unexpected in actual ({a})")),
                    (Some(a), Some(e)) => first_difference(a, e, &at),
                    (None, None) => None,
                }
            })
        }
        (Value::Array(a), Value::Array(e)) => {
            if a.len() != e.len() {
                return Some(format!("{path}: array count {} != {}", a.len(), e.len()));
            }
            a.iter()
                .zip(e)
                .enumerate()
                .find_map(|(i, (a, e))| first_difference(a, e, &format!("{path}[{i}]")))
        }
        _ if actual == expected => None,
        _ => Some(format!("{path}: actual {actual} != expected {expected}")),
    }
}

pub fn expect_matches_fixture(actual: &impl serde::Serialize, fixture_name: &str, root: &str) {
    let actual = normalize(serde_json::to_value(actual).unwrap(), root);
    let expected = fixture(fixture_name);
    if let Some(difference) = first_difference(&actual, &expected, "$") {
        panic!("{fixture_name}: {difference}");
    }
}

/// A name no other test (or test process) picks.
pub fn unique() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!(
        "{}-{nanos:x}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// A new empty folder in the temporary directory, canonical (symlinks
/// resolved, as libgit2 reports working directories).
pub fn temp_dir(prefix: &str) -> String {
    let dir = std::env::temp_dir().join(format!("{prefix}-{}", unique()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(&dir)
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

/// Pinned identity and config.
pub fn environment() -> HashMap<String, String> {
    let mut env: HashMap<String, String> = std::env::vars().collect();
    for (key, value) in [
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ("GIT_CONFIG_SYSTEM", "/dev/null"),
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_AUTHOR_NAME", "Impulse Test"),
        ("GIT_AUTHOR_EMAIL", "test@impulse.invalid"),
        ("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z"),
        ("GIT_COMMITTER_NAME", "Impulse Test"),
        ("GIT_COMMITTER_EMAIL", "test@impulse.invalid"),
        ("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z"),
    ] {
        env.insert(key.into(), value.into());
    }
    env
}

/// The overrides code under test that shells out to git gets.
pub fn git_overrides() -> HashMap<String, String> {
    environment()
        .into_iter()
        .filter(|(k, _)| k.starts_with("GIT_"))
        .collect()
}

/// Give every git command the crate runs the pinned environment (what the
/// Swift suites do in `init`).
pub fn setup() {
    impulse_git::operations::set_environment(git_overrides());
}

/// Run git in `root` with `env`, returning trimmed stdout.
pub fn run_git(
    root: &str,
    args: &[&str],
    stdin: Option<&str>,
    env: &HashMap<String, String>,
) -> Result<String, String> {
    run_git_raw(root, args, stdin, env).map(|out| out.trim().to_string())
}

/// `run_git`, with stdout as git printed it.
pub fn run_git_raw(
    root: &str,
    args: &[&str],
    stdin: Option<&str>,
    env: &HashMap<String, String>,
) -> Result<String, String> {
    let git = cli::git_path().unwrap_or_else(|| "/usr/bin/git".into());
    let mut full = vec!["-C", root];
    full.extend(args);
    let result = process::run(
        &git,
        &full,
        process::Options {
            environment: Some(env),
            stdin: stdin.map(str::as_bytes),
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;
    if result.status != 0 {
        return Err(format!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&result.stdout).into_owned())
}

pub struct TempRepo {
    /// Canonical absolute path of the repository root.
    pub root: String,
    /// Removed when dropped.
    owned: bool,
}

impl TempRepo {
    pub fn create() -> TempRepo {
        Self::create_on("main")
    }

    pub fn create_on(initial_branch: &str) -> TempRepo {
        let repo = TempRepo {
            root: temp_dir("impulse-git-test"),
            owned: true,
        };
        repo.git(&["init", "-q", "-b", initial_branch]).unwrap();
        repo
    }

    /// A handle on a folder something else owns (a worktree, a nested
    /// repository): never removed by this handle.
    pub fn at(root: impl Into<String>) -> TempRepo {
        TempRepo {
            root: root.into(),
            owned: false,
        }
    }

    /// Run git in the repository, returning trimmed stdout.
    pub fn git(&self, args: &[&str]) -> Result<String, String> {
        run_git(&self.root, args, None, &environment())
    }

    pub fn git_stdin(&self, args: &[&str], stdin: &str) -> Result<String, String> {
        run_git(&self.root, args, Some(stdin), &environment())
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        Path::new(&self.root).join(relative)
    }

    pub fn write(&self, relative: &str, content: &str) {
        self.write_bytes(relative, content.as_bytes());
    }

    pub fn write_bytes(&self, relative: &str, content: &[u8]) {
        let path = self.path(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    pub fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.path(relative)).unwrap()
    }

    pub fn exists(&self, relative: &str) -> bool {
        self.path(relative).exists()
    }

    pub fn remove(&self, relative: &str) {
        let path = self.path(relative);
        if path.is_dir() {
            std::fs::remove_dir_all(path).unwrap();
        } else {
            std::fs::remove_file(path).unwrap();
        }
    }

    /// Write files and commit them.
    pub fn commit(&self, files: &[(&str, &str)], message: &str) {
        for (path, content) in files {
            self.write(path, content);
        }
        self.git(&["add", "-A"]).unwrap();
        self.git(&["commit", "-q", "-m", message]).unwrap();
    }

    pub fn head(&self) -> String {
        self.git(&["rev-parse", "HEAD"]).unwrap()
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        if self.owned {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

/// Removes a folder (a worktree beside a repository) when dropped.
pub struct RemoveOnDrop(pub String);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The fixture scenario (Fixtures/scenario.json) rebuilt in a temporary
/// folder with the git CLI under its pinned author, committer and date.
pub struct ScenarioRepo {
    pub root: String,
}

impl ScenarioRepo {
    pub fn create() -> ScenarioRepo {
        let scenario = fixture("scenario.json");
        let author = &scenario["author"];
        let root = temp_dir("impulse-git-parity");
        let mut env: HashMap<String, String> = std::env::vars().collect();
        for (key, value) in [
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_SYSTEM", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_AUTHOR_NAME", author["name"].as_str().unwrap()),
            ("GIT_AUTHOR_EMAIL", author["email"].as_str().unwrap()),
            ("GIT_AUTHOR_DATE", author["date"].as_str().unwrap()),
            ("GIT_COMMITTER_NAME", author["name"].as_str().unwrap()),
            ("GIT_COMMITTER_EMAIL", author["email"].as_str().unwrap()),
            ("GIT_COMMITTER_DATE", author["date"].as_str().unwrap()),
        ] {
            env.insert(key.into(), value.into());
        }
        let git = |args: &[&str]| run_git(&root, args, None, &env).unwrap();
        let write = |relative: &str, content: &str| {
            let path = Path::new(&root).join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        };

        git(&["init", "-b", "main"]);
        git(&["config", "user.name", author["name"].as_str().unwrap()]);
        git(&["config", "user.email", author["email"].as_str().unwrap()]);
        let mut base: Vec<(&String, &Value)> =
            scenario["base_files"].as_object().unwrap().iter().collect();
        base.sort_by_key(|(k, _)| k.as_str());
        for (path, content) in base {
            write(path, content.as_str().unwrap());
        }
        git(&["add", "-A"]);
        git(&["commit", "-m", "base"]);
        let mut changes: Vec<(&String, &Value)> = scenario["worktree_changes"]
            .as_object()
            .unwrap()
            .iter()
            .collect();
        changes.sort_by_key(|(k, _)| k.as_str());
        for (path, content) in changes {
            match content.as_str() {
                Some(content) => write(path, content),
                None => std::fs::remove_file(Path::new(&root).join(path)).unwrap(),
            }
        }
        ScenarioRepo { root }
    }
}

impl Drop for ScenarioRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
