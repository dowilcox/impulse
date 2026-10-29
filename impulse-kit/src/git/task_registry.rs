//! The tasks Impulse created, kept in the repository's shared git folder
//! (`.git/impulse/tasks.json`) so every worktree, window and launch sees the
//! same list: each task's folder, branch, the base it was made from, and its
//! slot (a small number that per-task ports are worked out from). Worktrees
//! made any other way aren't listed, and task features leave them alone,
//! except the ones Claude Code makes for itself and its subagents
//! (`<repo>/.claude/worktrees/<name>`), which are adopted.
//!
//! The file is the Swift app's: the same keys, dates as ISO 8601 in whole
//! seconds, written byte for byte as Swift's `JSONEncoder` writes it
//! ([`TaskRegistry::to_json`]), so both apps can share a repository.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use super::worktree_tasks;
use crate::agents::foundation_json;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecord {
    /// The task's folder.
    pub path: String,
    pub branch: String,
    /// The branch the task was made from (`main`); `None` when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// The remote `base` is on (`origin`); `None` for a local base.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    /// Unique among the repository's tasks while the task exists; the main
    /// checkout is 0. `None` for tasks adopted from before the registry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<i64>,
    #[serde(with = "iso8601")]
    pub created: DateTime<Utc>,
    /// The commit the task started at: a branch still there has no work of
    /// its own yet (so it isn't "merged"). `None` for adopted tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    /// When Finish Task pushed the branch for review: once it's merged,
    /// Impulse offers to clean up.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "iso8601_option"
    )]
    pub pushed_for_review: Option<DateTime<Utc>>,
    /// Made from a remote branch (New Task from Branch...), possibly someone
    /// else's: its folder stays restricted until it's trusted itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_remote: Option<bool>,
    /// "agent" when an agent asked for it (`impulse tasks new`, the MCP
    /// server's create_task) and you approved, "user" when you made it;
    /// `None` for tasks from before this was kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    /// For an agent's task: which agent, in which workspace ("Claude Code in
    /// trailhead").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_by: Option<String>,
    /// The workspace the request came from, for telling a chain of agent
    /// tasks (an agent in an agent's task asking for another).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_from: Option<String>,
}

impl TaskRecord {
    /// A record made now, with nothing but its folder and branch; set the
    /// rest with struct update syntax.
    pub fn new(path: impl Into<String>, branch: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            branch: branch.into(),
            base: None,
            remote: None,
            slot: None,
            created: Utc::now(),
            start: None,
            pushed_for_review: None,
            from_remote: None,
            created_by: None,
            requested_by: None,
            requested_from: None,
        }
    }

    pub fn is_from_agent(&self) -> bool {
        self.created_by.as_deref() == Some("agent")
    }

    /// `origin/main`, or `main` for a local base; `None` when unknown.
    pub fn base_ref(&self) -> Option<String> {
        let base = self.base.as_ref()?;
        Some(match &self.remote {
            Some(remote) => format!("{remote}/{base}"),
            None => base.clone(),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRegistry {
    #[serde(default)]
    pub tasks: Vec<TaskRecord>,
}

/// The highest slot given. The search for a free one stops there, as ports
/// that are never free (in use for good, or past 65535) would keep it going
/// forever.
pub const MAX_SLOT: i64 = 999;

impl TaskRegistry {
    pub fn new(tasks: Vec<TaskRecord>) -> Self {
        Self { tasks }
    }

    /// `<common git dir>/impulse`: Impulse's folder inside the repository's
    /// shared git folder, never committed and shared by every worktree.
    pub fn directory(common_git_directory: &str) -> String {
        join(common_git_directory, "impulse")
    }

    pub fn file_path(common_git_directory: &str) -> String {
        join(&Self::directory(common_git_directory), "tasks.json")
    }

    /// The file's text, byte for byte as the Swift app writes it
    /// (`JSONEncoder` with `[.prettyPrinted, .sortedKeys]`: `"key" : value`,
    /// keys in code point order, `/` as `\/`, no final newline), so a
    /// repository both apps use doesn't churn. Either layout reads back.
    pub fn to_json(&self) -> io::Result<String> {
        let value = serde_json::to_value(self).map_err(io::Error::other)?;
        Ok(foundation_json::pretty_with(
            &value,
            foundation_json::Format::ENCODER,
        ))
    }

    pub fn record_for_path(&self, path: &str) -> Option<&TaskRecord> {
        let path = canonical(path);
        self.tasks.iter().find(|t| canonical(&t.path) == path)
    }

    /// A task named the way `impulse tasks` and the MCP tools name one: its
    /// folder's name or its branch.
    pub fn record_named(&self, name: &str) -> Option<&TaskRecord> {
        self.tasks
            .iter()
            .find(|t| super::last_path_component(&t.path) == name)
            .or_else(|| self.record_for_branch(name))
    }

    /// Tasks agents asked for that are still there.
    pub fn agent_task_count(&self) -> usize {
        self.tasks.iter().filter(|t| t.is_from_agent()).count()
    }

    /// The task with `branch` checked out, if any.
    pub fn record_for_branch(&self, branch: &str) -> Option<&TaskRecord> {
        self.tasks.iter().find(|t| t.branch == branch)
    }

    /// The lowest slot from 1 to `MAX_SLOT` that no task holds and
    /// `available` accepts; `None` when there's none.
    pub fn next_slot(&self, mut available: impl FnMut(i64) -> bool) -> Option<i64> {
        let taken: Vec<i64> = self.tasks.iter().filter_map(|t| t.slot).collect();
        (1..=MAX_SLOT).find(|slot| !taken.contains(slot) && available(*slot))
    }

    /// Add a task, replacing any record for the same folder.
    pub fn add(&mut self, mut record: TaskRecord) {
        self.remove(&record.path);
        record.path = canonical(&record.path);
        self.tasks.push(record);
    }

    pub fn remove(&mut self, path: &str) -> Option<TaskRecord> {
        let path = canonical(path);
        let index = self.tasks.iter().position(|t| canonical(&t.path) == path)?;
        Some(self.tasks.remove(index))
    }

    /// Bring the list in line with the repository's worktrees (each a folder
    /// and the branch checked out there): drop tasks whose worktree is gone
    /// (removed by hand), and adopt worktrees that aren't listed: in
    /// `<repo>.worktrees/`, which Impulse made before it kept this list, and
    /// in `<repo>/.claude/worktrees/`, which Claude Code made (`claude
    /// --worktree`, a subagent with `isolation: "worktree"`) and which are
    /// marked as started for it. Adopted tasks have no base or slot. True
    /// when anything changed.
    pub fn reconcile(
        &mut self,
        worktrees: &[(String, Option<String>)],
        repo_root: &str,
        now: DateTime<Utc>,
    ) -> bool {
        let before = self.tasks.clone();
        let present: Vec<String> = worktrees.iter().map(|(path, _)| canonical(path)).collect();
        self.tasks.retain(|t| present.contains(&canonical(&t.path)));
        let folder = canonical(&parent(&worktree_tasks::worktree_path(repo_root, "x")));
        let claude_folder = canonical(&join(repo_root, crate::agents::subagents::CLAUDE_WORKTREES));
        for (path, branch) in worktrees {
            let path = canonical(path);
            let Some(branch) = branch else { continue };
            let by_claude = parent(&path) == claude_folder;
            if (parent(&path) != folder && !by_claude) || self.record_for_path(&path).is_some() {
                continue;
            }
            self.tasks.push(TaskRecord {
                created: now,
                requested_by: by_claude.then(|| "Claude Code".to_string()),
                ..TaskRecord::new(path, branch.clone())
            });
        }
        self.tasks != before
    }

    /// The registry in `common_git_directory`; empty when there's none yet
    /// (or it doesn't parse).
    pub fn load(common_git_directory: &str) -> TaskRegistry {
        fs::read(Self::file_path(common_git_directory))
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default()
    }

    /// Read, change and write the registry under a lock, so two windows (or
    /// Impulse and Impulse Dev) don't lose each other's changes. A file that
    /// doesn't parse is kept beside it as `tasks.json.bad` before being
    /// replaced. Written only when `body` changed it.
    pub fn update<T>(
        common_git_directory: &str,
        body: impl FnOnce(&mut TaskRegistry) -> T,
    ) -> io::Result<T> {
        let folder = Self::directory(common_git_directory);
        fs::create_dir_all(&folder)?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(join(&folder, "tasks.lock"))?;
        lock.lock()?;

        let path = Self::file_path(common_git_directory);
        let mut registry = TaskRegistry::default();
        if let Ok(data) = fs::read(&path) {
            match serde_json::from_slice(&data) {
                Ok(decoded) => registry = decoded,
                Err(_) => {
                    let bad = format!("{path}.bad");
                    let _ = fs::remove_file(&bad);
                    let _ = fs::rename(&path, &bad);
                }
            }
        }
        let before = registry.clone();
        let result = body(&mut registry);
        if registry != before {
            let temp = format!("{path}.tmp");
            fs::write(&temp, registry.to_json()?)?;
            fs::rename(&temp, &path)?;
        }
        let _ = lock.unlock();
        Ok(result)
    }
}

/// A path with `.`/`..` and symlinks resolved, also for a folder that no
/// longer exists (an archived task): its nearest existing parent is resolved
/// instead, so `/var/...` and `/private/var/...` compare equal.
pub fn canonical(path: &str) -> String {
    let standard = standardize(path);
    if Path::new(&standard).exists() {
        if let Ok(real) = fs::canonicalize(&standard) {
            return real.to_string_lossy().into_owned();
        }
        return standard;
    }
    let parent = parent(&standard);
    if parent == standard || parent.is_empty() {
        return standard;
    }
    join(&canonical(&parent), super::last_path_component(&standard))
}

/// `path` with `~` expanded and `.`, `..`, doubled and trailing slashes
/// removed, without looking at the disk (`NSString.standardizingPath`).
pub(crate) fn standardize(path: &str) -> String {
    let expanded = match path.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            format!("{}{rest}", dirs::home_dir().unwrap_or_default().display())
        }
        _ => path.to_string(),
    };
    let mut out = PathBuf::new();
    for component in Path::new(&expanded).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir) => {}
                _ => out.push(".."),
            },
            other => out.push(other),
        }
    }
    let text = out.to_string_lossy().into_owned();
    if text.is_empty() && !path.is_empty() {
        ".".to_string()
    } else {
        text
    }
}

/// The folder holding `path` (`NSString.deletingLastPathComponent`).
pub(crate) fn parent(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => "/".to_string(),
        Some(slash) => trimmed[..slash].to_string(),
        None if path.starts_with('/') => "/".to_string(),
        None => String::new(),
    }
}

pub(crate) fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_string()
    } else if folder.ends_with('/') {
        format!("{folder}{name}")
    } else {
        format!("{folder}/{name}")
    }
}

/// Dates as Swift's `.iso8601` strategy writes them: whole seconds, `Z`.
mod iso8601 {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        date: &DateTime<Utc>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&date.to_rfc3339_opts(SecondsFormat::Secs, true))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<DateTime<Utc>, D::Error> {
        let text = String::deserialize(deserializer)?;
        DateTime::parse_from_rfc3339(&text)
            .map(|date| date.with_timezone(&Utc))
            .map_err(serde::de::Error::custom)
    }
}

mod iso8601_option {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        date: &Option<DateTime<Utc>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match date {
            Some(date) => iso8601::serialize(date, serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<DateTime<Utc>>, D::Error> {
        let Some(text) = Option::<String>::deserialize(deserializer)? else {
            return Ok(None);
        };
        DateTime::parse_from_rfc3339(&text)
            .map(|date| Some(date.with_timezone(&Utc)))
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    struct Scratch {
        _dir: tempfile::TempDir,
        git: String,
    }

    fn temporary_git_directory() -> Scratch {
        let dir = tempfile::tempdir().unwrap();
        let git = dir.path().join("repo/.git");
        fs::create_dir_all(&git).unwrap();
        Scratch {
            git: git.to_string_lossy().into_owned(),
            _dir: dir,
        }
    }

    fn record(path: &str, branch: &str, slot: Option<i64>) -> TaskRecord {
        TaskRecord {
            slot,
            ..TaskRecord::new(path, branch)
        }
    }

    #[test]
    fn slots_are_the_lowest_free_number_from_one() {
        let mut registry = TaskRegistry::default();
        assert_eq!(registry.next_slot(|_| true), Some(1));
        registry.add(record("/r.worktrees/a", "a", Some(1)));
        registry.add(record("/r.worktrees/b", "b", Some(2)));
        registry.add(record("/r.worktrees/old", "old", None));
        assert_eq!(
            registry.next_slot(|_| true),
            Some(3),
            "adopted tasks hold no slot"
        );
        registry.remove("/r.worktrees/a");
        assert_eq!(
            registry.next_slot(|_| true),
            Some(1),
            "an archived task's slot is free again"
        );
        assert_eq!(
            registry.next_slot(|s| s != 1),
            Some(3),
            "a slot whose ports are busy is skipped"
        );
    }

    #[test]
    fn the_search_for_a_free_slot_ends() {
        let mut registry = TaskRegistry::default();
        let mut asked = 0;
        assert_eq!(
            registry.next_slot(|_| {
                asked += 1;
                false
            }),
            None,
            "ports that are never free"
        );
        assert_eq!(asked, MAX_SLOT);
        for slot in 1..=MAX_SLOT {
            registry.add(record(
                &format!("/r.worktrees/t{slot}"),
                &format!("t{slot}"),
                Some(slot),
            ));
        }
        assert_eq!(registry.next_slot(|_| true), None, "every slot taken");

        // Under the registry's lock: the update returns, and the lock is free again.
        let s = temporary_git_directory();
        let none = TaskRegistry::update(&s.git, |r| r.next_slot(|_| false)).unwrap();
        assert_eq!(none, None);
        assert_eq!(
            TaskRegistry::update(&s.git, |r| r.next_slot(|_| true)).unwrap(),
            Some(1)
        );
    }

    #[test]
    fn adding_the_same_folder_replaces_its_record() {
        let mut registry = TaskRegistry::default();
        registry.add(record("/r.worktrees/a", "a", Some(1)));
        registry.add(record("/r.worktrees/./a", "a2", Some(4)));
        assert_eq!(registry.tasks.len(), 1);
        assert_eq!(
            registry.record_for_path("/r.worktrees/a").unwrap().branch,
            "a2"
        );
        assert_eq!(registry.record_for_branch("a2").unwrap().slot, Some(4));
        assert_eq!(
            registry.record_named("a").unwrap().branch,
            "a2",
            "by its folder's name"
        );
        assert_eq!(
            registry.record_named("a2").unwrap().branch,
            "a2",
            "or its branch"
        );
    }

    #[test]
    fn base_ref_names_the_remote() {
        let with = |base: Option<&str>, remote: Option<&str>| TaskRecord {
            base: base.map(String::from),
            remote: remote.map(String::from),
            ..TaskRecord::new("/x", "x")
        };
        assert_eq!(
            with(Some("main"), Some("origin")).base_ref().as_deref(),
            Some("origin/main")
        );
        assert_eq!(with(Some("main"), None).base_ref().as_deref(), Some("main"));
        assert_eq!(with(None, None).base_ref(), None);
    }

    #[test]
    fn reconcile_drops_gone_tasks_and_adopts_older_ones() {
        let mut registry = TaskRegistry::new(vec![
            TaskRecord {
                base: Some("main".into()),
                ..record("/code/app.worktrees/kept", "kept", Some(1))
            },
            record("/code/app.worktrees/gone", "gone", Some(2)),
        ]);
        let tree = |path: &str, branch: Option<&str>| (path.to_string(), branch.map(String::from));
        let changed = registry.reconcile(
            &[
                tree("/code/app", Some("main")),
                tree("/code/app.worktrees/kept", Some("kept")),
                tree("/code/app.worktrees/older", Some("older")),
                tree("/code/app.worktrees/detached", None),
                tree("/code/app/.worktrees/nested", Some("nested")),
                tree("/elsewhere/manual", Some("manual")),
                tree(
                    "/code/app/.claude/worktrees/agent-a1",
                    Some("worktree-agent-a1"),
                ),
                tree("/code/app/.claude/worktrees/x/deeper", Some("deeper")),
            ],
            "/code/app",
            Utc::now(),
        );
        assert!(changed);
        let branches: Vec<&str> = registry.tasks.iter().map(|t| t.branch.as_str()).collect();
        assert_eq!(branches, ["kept", "older", "worktree-agent-a1"]);
        assert_eq!(
            registry
                .record_for_branch("worktree-agent-a1")
                .unwrap()
                .requested_by
                .as_deref(),
            Some("Claude Code")
        );
        assert_eq!(
            registry.record_for_branch("older").unwrap().requested_by,
            None
        );
        assert_eq!(
            registry.record_for_branch("kept").unwrap().base.as_deref(),
            Some("main"),
            "existing records are kept as they are"
        );
        assert_eq!(registry.record_for_branch("older").unwrap().slot, None);
        let changed_again = registry.reconcile(
            &[
                tree("/code/app.worktrees/kept", Some("kept")),
                tree("/code/app.worktrees/older", Some("older")),
                tree(
                    "/code/app/.claude/worktrees/agent-a1",
                    Some("worktree-agent-a1"),
                ),
            ],
            "/code/app",
            Utc::now(),
        );
        assert!(!changed_again);
    }

    #[test]
    fn update_writes_and_load_reads() {
        let s = temporary_git_directory();
        assert_eq!(TaskRegistry::load(&s.git), TaskRegistry::default());
        let created = Utc.timestamp_opt(1_760_000_000, 0).unwrap();
        let expected = TaskRecord {
            base: Some("main".into()),
            remote: Some("origin".into()),
            slot: Some(1),
            created,
            ..TaskRecord::new("/r.worktrees/a", "a")
        };
        let slot = TaskRegistry::update(&s.git, |registry| {
            let slot = registry.next_slot(|_| true);
            registry.add(TaskRecord {
                slot,
                ..expected.clone()
            });
            slot
        })
        .unwrap();
        assert_eq!(slot, Some(1));
        assert_eq!(
            TaskRegistry::file_path(&s.git),
            format!("{}/impulse/tasks.json", s.git)
        );
        assert_eq!(TaskRegistry::load(&s.git).tasks, [expected]);
        let text = fs::read_to_string(TaskRegistry::file_path(&s.git)).unwrap();
        assert!(
            text.contains(r#""created" : "2025-10-09T08:53:20Z""#),
            "{text}"
        );
        assert!(
            !text.contains("pushedForReview"),
            "unset values are left out"
        );
    }

    #[test]
    fn reads_the_swift_apps_file() {
        let s = temporary_git_directory();
        let path = TaskRegistry::file_path(&s.git);
        fs::create_dir_all(parent(&path)).unwrap();
        fs::write(
            &path,
            r#"{
  "tasks" : [
    {
      "branch" : "fix",
      "created" : "2025-10-09T08:53:20Z",
      "createdBy" : "agent",
      "path" : "/r.worktrees/fix",
      "pushedForReview" : "2025-10-10T08:00:00Z",
      "requestedBy" : "Claude Code in r",
      "slot" : 2
    }
  ]
}"#,
        )
        .unwrap();
        let registry = TaskRegistry::load(&s.git);
        let task = &registry.tasks[0];
        assert!(task.is_from_agent());
        assert_eq!(task.slot, Some(2));
        assert_eq!(task.requested_by.as_deref(), Some("Claude Code in r"));
        assert_eq!(
            task.pushed_for_review,
            Some(Utc.with_ymd_and_hms(2025, 10, 10, 8, 0, 0).unwrap())
        );
        assert_eq!(registry.agent_task_count(), 1);
    }

    // The expected texts below are what the Swift app's encoder
    // (`JSONEncoder`, `[.prettyPrinted, .sortedKeys]`, `.iso8601` dates)
    // wrote for the same registries: a Swift probe with TaskRecord's
    // fields, run on macOS 27 with Swift 6.4 when this was written.

    #[test]
    fn writes_the_file_as_the_swift_app_does() {
        let full = TaskRecord {
            base: Some("main".into()),
            remote: Some("origin".into()),
            slot: Some(3),
            // Fractions of a second are dropped, as Swift's `.iso8601`.
            created: Utc.timestamp_opt(1_760_000_000, 750_000_000).unwrap(),
            start: Some("0123456789abcdef0123456789abcdef01234567".into()),
            pushed_for_review: Some(Utc.timestamp_opt(1_760_086_400, 0).unwrap()),
            from_remote: Some(true),
            created_by: Some("agent".into()),
            requested_by: Some("Claude Code in café 🚀".into()),
            requested_from: Some("/Users/me/code/app".into()),
            ..TaskRecord::new("/Users/me/code/app.worktrees/feature-x", "feature/x")
        };
        let minimal = TaskRecord {
            slot: Some(0),
            created: Utc.timestamp_opt(0, 0).unwrap(),
            from_remote: Some(false),
            created_by: Some("user".into()),
            ..TaskRecord::new("/r.worktrees/a b", r#"a"b\c</d>"#)
        };
        let registry = TaskRegistry::new(vec![full, minimal]);
        let swift = r#"{
  "tasks" : [
    {
      "base" : "main",
      "branch" : "feature\/x",
      "created" : "2025-10-09T08:53:20Z",
      "createdBy" : "agent",
      "fromRemote" : true,
      "path" : "\/Users\/me\/code\/app.worktrees\/feature-x",
      "pushedForReview" : "2025-10-10T08:53:20Z",
      "remote" : "origin",
      "requestedBy" : "Claude Code in café 🚀",
      "requestedFrom" : "\/Users\/me\/code\/app",
      "slot" : 3,
      "start" : "0123456789abcdef0123456789abcdef01234567"
    },
    {
      "branch" : "a\"b\\c<\/d>",
      "created" : "1970-01-01T00:00:00Z",
      "createdBy" : "user",
      "fromRemote" : false,
      "path" : "\/r.worktrees\/a b",
      "slot" : 0
    }
  ]
}"#;
        assert_eq!(registry.to_json().unwrap(), swift);
        let read: TaskRegistry = serde_json::from_str(swift).unwrap();
        assert_eq!(read.tasks[0].branch, "feature/x");
        assert_eq!(read.tasks[1].branch, r#"a"b\c</d>"#);

        assert_eq!(
            TaskRegistry::default().to_json().unwrap(),
            "{\n  \"tasks\" : [\n\n  ]\n}"
        );

        let controls: String = (0u32..0x20)
            .chain([0x7f, 0x2028, 0x2029, 0xfeff])
            .map(|c| char::from_u32(c).unwrap())
            .collect();
        let odd = TaskRegistry::new(vec![TaskRecord {
            created: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            requested_by: Some(controls),
            ..TaskRecord::new("/x", "x")
        }]);
        assert_eq!(
            odd.to_json().unwrap(),
            "{\n  \"tasks\" : [\n    {\n      \"branch\" : \"x\",\n      \"created\" : \"2023-11-14T22:13:20Z\",\n      \"path\" : \"\\/x\",\n      \"requestedBy\" : \"\\u0000\\u0001\\u0002\\u0003\\u0004\\u0005\\u0006\\u0007\\b\\t\\n\\u000b\\f\\r\\u000e\\u000f\\u0010\\u0011\\u0012\\u0013\\u0014\\u0015\\u0016\\u0017\\u0018\\u0019\\u001a\\u001b\\u001c\\u001d\\u001e\\u001f\u{7f}\u{2028}\u{2029}\u{feff}\"\n    }\n  ]\n}"
        );
    }

    #[test]
    fn update_writes_swifts_layout_and_reads_the_older_one() {
        let s = temporary_git_directory();
        let path = TaskRegistry::file_path(&s.git);
        fs::create_dir_all(parent(&path)).unwrap();
        // What this app wrote before: serde_json's layout, a final newline.
        fs::write(
            &path,
            "{\n  \"tasks\": [\n    {\n      \"branch\": \"a\",\n      \"created\": \"2025-10-09T08:53:20Z\",\n      \"path\": \"/r.worktrees/a\",\n      \"slot\": 1\n    }\n  ]\n}\n",
        )
        .unwrap();
        assert_eq!(TaskRegistry::load(&s.git).tasks[0].slot, Some(1));
        TaskRegistry::update(&s.git, |r| {
            r.add(record("/r.worktrees/b", "b", Some(2)));
        })
        .unwrap();
        let written = fs::read_to_string(&path).unwrap();
        assert_eq!(written, TaskRegistry::load(&s.git).to_json().unwrap());
        assert!(
            written.contains(r#""path" : "\/r.worktrees\/a""#),
            "{written}"
        );
        assert!(!written.ends_with('\n'), "no final newline, as Swift");
        assert_eq!(TaskRegistry::load(&s.git).tasks.len(), 2);
    }

    #[test]
    fn a_broken_file_is_kept_aside_and_replaced() {
        let s = temporary_git_directory();
        let path = TaskRegistry::file_path(&s.git);
        fs::create_dir_all(parent(&path)).unwrap();
        fs::write(&path, "{ not json").unwrap();
        assert_eq!(TaskRegistry::load(&s.git), TaskRegistry::default());
        TaskRegistry::update(&s.git, |r| r.add(record("/r.worktrees/a", "a", Some(1)))).unwrap();
        assert_eq!(TaskRegistry::load(&s.git).tasks.len(), 1);
        assert_eq!(
            fs::read_to_string(format!("{path}.bad")).unwrap(),
            "{ not json"
        );
    }

    #[test]
    fn canonical_paths_resolve_symlinks_even_for_missing_folders() {
        let s = temporary_git_directory();
        let missing = format!("{}/gone/folder", s.git);
        assert!(canonical(&missing).ends_with("/repo/.git/gone/folder"));
        assert_eq!(
            canonical(&missing),
            canonical(&missing.replace("/.git/", "/.git/./"))
        );
        assert_eq!(
            canonical(&s.git),
            fs::canonicalize(&s.git).unwrap().to_string_lossy()
        );

        #[cfg(unix)]
        {
            let link = format!("{}/../../link", s.git);
            std::os::unix::fs::symlink(format!("{}/..", s.git), &link).unwrap();
            assert_eq!(
                canonical(&format!("{link}/.git/gone")),
                canonical(&format!("{}/gone", s.git))
            );
        }
    }
}
