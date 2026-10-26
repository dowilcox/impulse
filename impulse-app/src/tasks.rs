//! Tasks: a branch checked out in its own folder beside the repository
//! (`<repo>.worktrees/<branch>`) and opened as a workspace, as the Swift
//! `TaskWorkflows`, `TaskRegistryStore`, `OverlapMonitor` and the task parts
//! of `TabManager` and `ProjectActions` do it. This file holds what the
//! task features share: the repository's task list (`.git/impulse/tasks.json`),
//! a terminal's task identity, the project settings and their trust, a new
//! task's preparation (copies, values, setup), New Task from Branch, the
//! sidebar's task states, and the toasts and confirmations the flows show.
//! The New Task sheet is in `task_sheet`, archiving in `task_cleanup`,
//! Finish Task in `task_finish`, the sidebar's task rows in `task_sidebar`.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::{prelude::*, App, Context, Window};
use impulse_git::{branches, operations, scoped_diff, snapshot};
use impulse_kit::child_process::ChildProcess;
use impulse_kit::git::task_overlap::{self, Pair};
use impulse_kit::git::task_registry::{self, TaskRecord, TaskRegistry};
use impulse_kit::git::worktree_tasks;
use impulse_kit::project::config::{Loaded, ProjectConfig, ProjectTrust};
use impulse_kit::project::{compose_file, folder_clone, task_environment};
use impulse_kit::trust::WorkspaceTrust;

use crate::workbench::Workbench;
use crate::workbench_chrome::abbreviate_home;
use crate::workspace::{Kind, Workspace};

// MARK: Paths

fn last_component(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

fn parent_of(path: &str) -> String {
    Path::new(path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The main checkout of the repository `root` is in: for a task (a linked
/// worktree), the folder its repository's `.git` is in; otherwise `root`.
pub(crate) fn main_checkout_root(root: &str) -> String {
    match (
        branches::git_directory(root),
        branches::common_git_directory(root),
    ) {
        (Some(git_dir), Some(common)) if git_dir != common && last_component(&common) == ".git" => {
            parent_of(&common)
        }
        _ => root.to_string(),
    }
}

/// Impulse's own files for a task, outside its folder:
/// `.git/impulse/tasks/<task>`.
pub(crate) fn task_folder(task: &str, common: &str) -> String {
    Path::new(&TaskRegistry::directory(common))
        .join("tasks")
        .join(task)
        .to_string_lossy()
        .into_owned()
}

/// A relative path that stays inside the folder it's relative to.
fn is_inside(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.starts_with('~')
        && !path.split('/').any(|p| p == "..")
}

// MARK: The task list (TaskRegistryStore)

/// The registry of the repository `root` is in, brought in line with its
/// worktrees first (tasks removed by hand dropped, older tasks in
/// `<repo>.worktrees/` adopted). `None` outside a repository. When git can't
/// list the worktrees, the registry is left as it is.
pub(crate) fn registry(root: &str) -> Option<TaskRegistry> {
    let common = branches::common_git_directory(root)?;
    let main = main_checkout_root(root);
    let registry = TaskRegistry::load(&common);
    let Some(listed) = operations::worktree_list(&main) else {
        return Some(registry);
    };
    let worktrees: Vec<(String, Option<String>)> = listed
        .into_iter()
        .filter(|w| !w.is_bare)
        .map(|w| (w.path, w.branch))
        .collect();
    let now = chrono::Utc::now();
    let mut reconciled = registry.clone();
    if !reconciled.reconcile(&worktrees, &main, now) {
        return Some(registry);
    }
    Some(
        TaskRegistry::update(&common, |stored| {
            stored.reconcile(&worktrees, &main, now);
            stored.clone()
        })
        .unwrap_or(reconciled),
    )
}

/// The record of the task at `path`, if Impulse made it.
pub(crate) fn record_for_path(path: &str) -> Option<TaskRecord> {
    registry(path)?.record_for_path(path).cloned()
}

/// Who asked for a task: "user", or "agent" with who asked and from where.
pub(crate) struct Provenance {
    pub created_by: &'static str,
    pub requested_by: Option<String>,
    pub requested_from: Option<String>,
}

impl Default for Provenance {
    fn default() -> Self {
        Provenance {
            created_by: "user",
            requested_by: None,
            requested_from: None,
        }
    }
}

/// Record a new task, giving it the lowest free slot `available` accepts.
/// `base` is what the sheet's From held: `origin/main` is split into the
/// remote and its branch, anything else is kept as typed (empty: the main
/// checkout's branch).
pub(crate) fn record_created(
    path: &str,
    branch: &str,
    base: &str,
    root: &str,
    from_remote: bool,
    provenance: Provenance,
    available: impl FnMut(i64) -> bool,
) -> Option<TaskRecord> {
    let common = branches::common_git_directory(root)?;
    let main = main_checkout_root(root);
    let mut base_branch = if base.is_empty() {
        operations::current_branch(&main)
    } else {
        Some(base.to_string())
    };
    let mut remote = None;
    if let Some(typed) = base_branch.clone() {
        if let Some(found) = operations::remotes(&main)
            .into_iter()
            .find(|r| typed.starts_with(&format!("{r}/")))
        {
            base_branch = Some(typed[found.len() + 1..].to_string());
            remote = Some(found);
        }
    }
    let start = scoped_diff::resolve_commit(path, "HEAD");
    TaskRegistry::update(&common, move |registry| {
        let record = TaskRecord {
            base: base_branch,
            remote,
            slot: registry.next_slot(available),
            start,
            from_remote: from_remote.then_some(true),
            created_by: Some(provenance.created_by.to_string()),
            requested_by: provenance.requested_by,
            requested_from: provenance
                .requested_from
                .map(|p| task_registry::canonical(&p)),
            ..TaskRecord::new(path, branch)
        };
        registry.add(record.clone());
        record
    })
    .ok()
}

/// Whether every port of `slot` is free on this computer (false when one is
/// past 65535).
pub(crate) fn ports_are_free(config: Option<&ProjectConfig>, slot: i64) -> bool {
    let Some(config) = config.filter(|c| !c.ports.is_empty()) else {
        return true;
    };
    match task_environment::ports(&config.ports, slot, config.port_offset) {
        Some(ports) => ports.values().all(|p| task_environment::port_is_free(*p)),
        None => false,
    }
}

/// Who a terminal in `directory` belongs to, for its environment
/// (`IMPULSE_TASK`, `IMPULSE_TASK_SLOT`, `IMPULSE_REPO_ROOT`): cheap enough
/// for each new terminal (no git processes, nothing reconciled).
pub(crate) fn identity(directory: &str) -> Vec<(String, String)> {
    let Some(common) = branches::common_git_directory(directory) else {
        return Vec::new();
    };
    if last_component(&common) != ".git" {
        return Vec::new();
    }
    let mut env = vec![("IMPULSE_REPO_ROOT".to_string(), parent_of(&common))];
    if let Some(worktree) = branches::repo_root(directory) {
        if let Some(record) = TaskRegistry::load(&common).record_for_path(&worktree) {
            env.push(("IMPULSE_TASK".into(), last_component(&record.path)));
            if let Some(slot) = record.slot {
                env.push(("IMPULSE_TASK_SLOT".into(), slot.to_string()));
            }
        }
    }
    env
}

/// Note that Finish Task pushed the task at `path` for review.
pub(crate) fn mark_pushed_for_review(path: &str, root: &str) {
    let Some(common) = branches::common_git_directory(root) else {
        return;
    };
    let canonical = task_registry::canonical(path);
    let _ = TaskRegistry::update(&common, |registry| {
        if let Some(task) = registry
            .tasks
            .iter_mut()
            .find(|t| task_registry::canonical(&t.path) == canonical)
        {
            task.pushed_for_review = Some(chrono::Utc::now());
        }
    });
}

/// Forget the task at `path` (archived); the record, for Undo.
pub(crate) fn remove_record(path: &str, root: &str) -> Option<TaskRecord> {
    let common = branches::common_git_directory(root)?;
    TaskRegistry::update(&common, |registry| registry.remove(path))
        .ok()
        .flatten()
}

/// Put an archived task's record back (Undo), with its slot when no task
/// took it meanwhile.
pub(crate) fn restore_record(record: &TaskRecord, root: &str) {
    let Some(common) = branches::common_git_directory(root) else {
        return;
    };
    let mut record = record.clone();
    let _ = TaskRegistry::update(&common, move |registry| {
        if let Some(slot) = record.slot {
            if registry.tasks.iter().any(|t| t.slot == Some(slot)) {
                record.slot = registry.next_slot(|_| true);
            }
        }
        registry.add(record);
    });
}

/// "Task slot 1 · from origin/main · APP_PORT 8100, VITE_PORT 5273" for a
/// task Impulse made; `None` for other worktrees. Reads the list as it is
/// (no git), so it can run as a workspace opens.
pub(crate) fn task_summary(root: &str) -> Option<String> {
    let common = branches::common_git_directory(root)?;
    let record = TaskRegistry::load(&common).record_for_path(root)?.clone();
    let mut parts = vec![format!(
        "Task{}",
        record
            .slot
            .map(|s| format!(" slot {s}"))
            .unwrap_or_default()
    )];
    if let Some(base) = record.base_ref() {
        parts.push(format!("from {base}"));
    }
    if let Some(requested_by) = &record.requested_by {
        parts.push(format!("started for {requested_by}"));
    }
    if let Some(slot) = record.slot {
        if let Some(config) = project_config(root) {
            if let Some(ports) = (!config.ports.is_empty())
                .then(|| task_environment::ports(&config.ports, slot, config.port_offset))
                .flatten()
            {
                parts.push(
                    ports
                        .iter()
                        .map(|(k, v)| format!("{k} {v}"))
                        .collect::<Vec<_>>()
                        .join(", "),
                );
            }
        }
    }
    Some(parts.join(" · "))
}

// MARK: A workspace's task state

/// What the sidebar knows about a folder workspace's repository and task:
/// the Swift `Workspace.isTask`/`taskSummary` and the `OverlapMonitor`'s
/// answers for it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskInfo {
    /// The repository's shared git folder: workspaces with the same one are
    /// grouped under the repository's name.
    pub common_dir: Option<String>,
    /// A linked worktree of its repository.
    pub is_task: bool,
    pub summary: Option<String>,
    /// Its branch is merged into its base.
    pub merged: bool,
    /// Its branch's upstream was deleted on the remote.
    pub upstream_gone: bool,
    /// Where it changes the same files as another workspace of the repository.
    pub overlaps: Vec<Pair>,
    /// Some task of its repository is merged (Archive Merged Tasks...).
    pub repository_has_merged_tasks: bool,
}

impl TaskInfo {
    /// The cheap part, as a workspace opens: its repository and whether
    /// it's a task (libgit2 and a file read, no git processes).
    pub fn read(kind: Kind, root: &Path) -> TaskInfo {
        if kind != Kind::Folder {
            return TaskInfo::default();
        }
        let root = root.to_string_lossy();
        let common = branches::common_git_directory(&root);
        let is_task = common.is_some() && branches::git_directory(&root) != common;
        TaskInfo {
            summary: is_task.then(|| task_summary(&root)).flatten(),
            common_dir: common,
            is_task,
            ..Default::default()
        }
    }

    /// The repository's name for the sidebar's group header: the folder
    /// its `.git` is in.
    pub fn repository_name(&self) -> Option<String> {
        let common = self.common_dir.as_ref()?;
        Some(last_component(&parent_of(common)))
    }
}

/// The sidebar's group key: the repository's shared git folder, or the
/// workspace's own id outside a repository.
pub(crate) fn group_key(workspace: &Workspace) -> String {
    workspace
        .task
        .common_dir
        .clone()
        .unwrap_or_else(|| format!("workspace-{}", workspace.id))
}

// MARK: The overlap monitor

/// One repository's tasks as the monitor last worked them out.
#[derive(Default)]
struct RepositoryTasks {
    common: String,
    pairs: Vec<Pair>,
    merged: HashSet<String>,
    /// Merged tasks that Finish pushed for review (merged on the server).
    reviewed_merged: HashSet<String>,
    upstream_gone: HashSet<String>,
    summaries: HashMap<String, Option<String>>,
}

thread_local! {
    /// Tasks whose "was merged" toast went out, in any window (Swift's
    /// `OverlapMonitor.claimCleanUpOffer`): offered once per run.
    static OFFERED_CLEAN_UP: std::cell::RefCell<HashSet<String>> = Default::default();
}

/// Answers by (base, tip) commit ids: what two commits say about each other
/// never changes, so a recompute only asks git about branches that moved.
static MERGED_ANSWERS: LazyLock<Mutex<HashMap<String, bool>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static COMMITTED_ANSWERS: LazyLock<Mutex<HashMap<String, Vec<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn cached<V: Clone>(
    cache: &Mutex<HashMap<String, V>>,
    base: &str,
    tip: &str,
    compute: impl FnOnce() -> V,
) -> V {
    let key = format!("{base}..{tip}");
    if let Some(known) = cache.lock().unwrap().get(&key) {
        return known.clone();
    }
    let value = compute();
    let mut cache = cache.lock().unwrap();
    if cache.len() >= 2_000 {
        cache.clear();
    }
    cache.insert(key, value.clone());
    value
}

/// Every changed path of a snapshot: staged, unstaged, untracked, conflicted.
pub(crate) fn uncommitted(snapshot: Option<&snapshot::RepoSnapshot>) -> BTreeSet<String> {
    let Some(s) = snapshot else {
        return BTreeSet::new();
    };
    [&s.staged, &s.unstaged, &s.untracked, &s.conflicted]
        .into_iter()
        .flatten()
        .map(|c| c.path.clone())
        .collect()
}

/// The repository's tasks, without any git work when it has no linked
/// worktrees (and so no tasks).
fn repository_tasks(main: &str, common: &str) -> Vec<TaskRecord> {
    let linked = std::fs::read_dir(Path::new(common).join("worktrees"))
        .map(|d| d.count())
        .unwrap_or(0);
    if linked == 0 {
        return Vec::new();
    }
    registry(main).map(|r| r.tasks).unwrap_or_default()
}

/// What each workspace of the repository changes: the main checkout's
/// uncommitted files and commits not pushed, and each task's uncommitted
/// files and commits since it left its base.
pub(crate) fn changes(main: &str, tasks: &[TaskRecord]) -> Vec<task_overlap::Changes> {
    if tasks.is_empty() {
        return Vec::new();
    }
    let main_snapshot = snapshot::snapshot(main, false);
    let fallback = scoped_diff::default_base_branch(main);
    let mut revisions: Vec<String> = tasks
        .iter()
        .filter_map(|t| t.base_ref().or_else(|| fallback.clone()))
        .collect();
    revisions.extend(main_snapshot.as_ref().and_then(|s| s.upstream.clone()));
    revisions.extend(tasks.iter().map(|t| format!("refs/heads/{}", t.branch)));
    let refs: Vec<&str> = revisions.iter().map(String::as_str).collect();
    let commits = scoped_diff::resolve_commits(main, &refs);
    let mut list = Vec::new();
    let mut main_files = uncommitted(main_snapshot.as_ref());
    if let Some(snapshot) = &main_snapshot {
        if let (Some(upstream), Some(head)) = (
            snapshot.upstream.as_ref().and_then(|u| commits.get(u)),
            snapshot.head_oid.as_ref(),
        ) {
            main_files.extend(committed_paths(upstream, head, main));
        }
    }
    list.push(task_overlap::Changes::new(
        task_registry::canonical(main),
        last_component(main),
        main_files,
    ));
    for task in tasks {
        let mut files = uncommitted(snapshot::snapshot(&task.path, false).as_ref());
        let base = task
            .base_ref()
            .or_else(|| fallback.clone())
            .and_then(|b| commits.get(&b).cloned());
        if let (Some(base), Some(tip)) = (base, commits.get(&format!("refs/heads/{}", task.branch)))
        {
            files.extend(committed_paths(&base, tip, main));
        }
        list.push(task_overlap::Changes::new(
            task_registry::canonical(&task.path),
            last_component(&task.path),
            files,
        ));
    }
    list
}

/// The files `tip` changed since it left `base` (commit ids).
fn committed_paths(base: &str, tip: &str, root: &str) -> Vec<String> {
    cached(&COMMITTED_ANSWERS, base, tip, || {
        operations::merge_base(base, tip, root)
            .map(|start| operations::changed_paths(&start, tip, root))
            .unwrap_or_default()
    })
}

/// Tasks whose branch has work of its own (it moved past where it started)
/// and is merged into its base, squash merges included.
pub(crate) fn merged_records(main: &str, tasks: &[TaskRecord], trusted: bool) -> Vec<TaskRecord> {
    let candidates: Vec<&TaskRecord> = tasks
        .iter()
        .filter(|t| t.start.is_some() && t.base.is_some())
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }
    let mut revisions: Vec<String> = candidates.iter().filter_map(|t| t.base_ref()).collect();
    revisions.extend(
        candidates
            .iter()
            .map(|t| format!("refs/heads/{}", t.branch)),
    );
    let refs: Vec<&str> = revisions.iter().map(String::as_str).collect();
    let commits = scoped_diff::resolve_commits(main, &refs);
    candidates
        .into_iter()
        .filter(|task| {
            let (Some(base), Some(tip)) = (
                task.base_ref().and_then(|b| commits.get(&b)),
                commits.get(&format!("refs/heads/{}", task.branch)),
            ) else {
                return false;
            };
            if Some(tip) == task.start.as_ref() {
                return false;
            }
            cached(&MERGED_ANSWERS, base, tip, || {
                operations::is_merged(tip, base, main, trusted)
            })
        })
        .cloned()
        .collect()
}

/// The files that would conflict if two workspaces' work met, uncommitted
/// work included (off the main thread). `None` when git can't tell.
pub(crate) fn conflicts(pair: &Pair) -> Option<Vec<String>> {
    let trusted = |path: &str| crate::workbench_trust::is_trusted(path);
    let a = impulse_git::safety::working_tree_commit(&pair.a.path, trusted(&pair.a.path))?;
    let b = impulse_git::safety::working_tree_commit(&pair.b.path, trusted(&pair.b.path))?;
    operations::predict_conflicts(&a, &b, &pair.a.path)
}

fn compute_repository(main: &str, common: &str, trust: &WorkspaceTrust) -> RepositoryTasks {
    let tasks = repository_tasks(main, common);
    let changes = changes(main, &tasks);
    let mut pairs = Vec::new();
    if changes.len() >= 2 {
        // Each task's env file is its own by design: never an overlap.
        let config = project_config(main).unwrap_or_default();
        let mut ignoring = config.overlap_ignore.clone();
        ignoring.push(config.env_file.clone());
        pairs = task_overlap::pairs(&changes, &ignoring);
    }
    let trusted = trust.is_trusted(main);
    let merged_records = merged_records(main, &tasks, trusted);
    let merged = merged_records
        .iter()
        .map(|t| task_registry::canonical(&t.path))
        .collect();
    let reviewed_merged = merged_records
        .iter()
        .filter(|t| t.pushed_for_review.is_some())
        .map(|t| task_registry::canonical(&t.path))
        .collect();
    let upstream_gone = if tasks.is_empty() {
        HashSet::new()
    } else {
        let gone: HashSet<String> = operations::branch_details(main, None)
            .into_iter()
            .filter(|b| b.upstream_gone)
            .map(|b| b.name)
            .collect();
        tasks
            .iter()
            .filter(|t| gone.contains(&t.branch))
            .map(|t| task_registry::canonical(&t.path))
            .collect()
    };
    let summaries = tasks
        .iter()
        .map(|t| (task_registry::canonical(&t.path), task_summary(&t.path)))
        .collect();
    RepositoryTasks {
        common: common.to_string(),
        pairs,
        merged,
        reviewed_merged,
        upstream_gone,
        summaries,
    }
}

impl Workbench {
    /// Keep the sidebar's task states current: when a repository the
    /// window holds changes (`schedule_task_refresh`), when a task flow asks
    /// (`refresh_task_states`), and once a minute for what no watcher sees.
    pub(crate) fn start_task_monitor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |workbench, cx| loop {
            cx.background_executor()
                .timer(Duration::from_secs(60))
                .await;
            if workbench
                .update(cx, |workbench, cx| workbench.refresh_task_states(cx))
                .is_err()
            {
                break;
            }
        })
        .detach();
        self.refresh_task_states(cx);
    }

    /// A repository changed: refresh the task states once changes settle
    /// (Swift's OverlapMonitor follows the repositories the same way).
    pub(crate) fn schedule_task_refresh(&mut self, cx: &mut Context<Self>) {
        if !self.workspaces.iter().any(|w| w.task.common_dir.is_some()) {
            return;
        }
        self.task_refresh = Some(cx.spawn(async move |workbench, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            let _ = workbench.update(cx, |workbench, cx| workbench.refresh_task_states(cx));
        }));
    }

    /// Work out again where each repository's workspaces change the same
    /// files, which tasks are merged, and their summaries (off the main
    /// thread). Only for repositories with tasks.
    pub(crate) fn refresh_task_states(&mut self, cx: &mut Context<Self>) {
        let mut repositories: Vec<(String, String)> = Vec::new();
        for workspace in &self.workspaces {
            if let Some(common) = &workspace.task.common_dir {
                if !repositories.iter().any(|(c, _)| c == common) {
                    repositories.push((common.clone(), workspace.root.to_string_lossy().into()));
                }
            }
        }
        if repositories.is_empty() {
            return;
        }
        // One at a time in each window; one asked for meanwhile follows.
        if let Some(again) = self.task_states_running.as_mut() {
            *again = true;
            return;
        }
        self.task_states_running = Some(false);
        let trust = self.trust.clone();
        let work = cx.background_executor().spawn(async move {
            repositories
                .iter()
                .map(|(common, root)| compute_repository(&main_checkout_root(root), common, &trust))
                .collect::<Vec<RepositoryTasks>>()
        });
        cx.spawn(async move |workbench, cx| {
            let found = work.await;
            let _ = workbench.update(cx, |workbench, cx| {
                let again = workbench.task_states_running.take() == Some(true);
                workbench.apply_task_states(found, cx);
                if again {
                    workbench.refresh_task_states(cx);
                }
            });
        })
        .detach();
    }

    fn apply_task_states(&mut self, found: Vec<RepositoryTasks>, cx: &mut Context<Self>) {
        self.note_new_overlaps(&found, cx);
        self.offer_clean_up(&found, cx);
        let mut changed = false;
        for workspace in &mut self.workspaces {
            let Some(repository) = found
                .iter()
                .find(|r| Some(&r.common) == workspace.task.common_dir.as_ref())
            else {
                continue;
            };
            let path = task_registry::canonical(&workspace.root.to_string_lossy());
            let mut info = workspace.task.clone();
            info.merged = info.is_task && repository.merged.contains(&path);
            info.upstream_gone = info.is_task && repository.upstream_gone.contains(&path);
            info.overlaps = repository
                .pairs
                .iter()
                .filter(|p| p.contains(&path))
                .cloned()
                .collect();
            info.repository_has_merged_tasks = !repository.merged.is_empty();
            if let Some(summary) = repository.summaries.get(&path) {
                info.summary = summary.clone();
            }
            if info != workspace.task {
                workspace.task = info;
                changed = true;
            }
        }
        if changed {
            cx.notify();
        }
    }

    /// A task Finish pushed for review was merged on the server: the
    /// window that has it open offers to finish the job, once (Swift's
    /// `reviewedTasksMerged`).
    fn offer_clean_up(&mut self, found: &[RepositoryTasks], cx: &mut Context<Self>) {
        let offers: Vec<(u64, String)> = found
            .iter()
            .flat_map(|r| r.reviewed_merged.iter())
            .filter_map(|path| {
                let workspace = self
                    .workspaces
                    .iter()
                    .find(|w| task_registry::canonical(&w.root.to_string_lossy()) == *path)?;
                OFFERED_CLEAN_UP
                    .with_borrow_mut(|offered| offered.insert(path.clone()))
                    .then(|| (workspace.id, workspace.name()))
            })
            .collect();
        if offers.is_empty() {
            return;
        }
        let Some((_, handle)) = crate::session::workbenches(cx)
            .into_iter()
            .find(|(w, _)| w.entity_id() == cx.entity_id())
        else {
            return;
        };
        let workbench = cx.entity().downgrade();
        let _ = handle.update(cx, move |_, window, cx| {
            for (id, name) in offers {
                let workbench = workbench.clone();
                let clean_up: Run = Rc::new(move |window, cx| {
                    let _ = workbench.update(cx, |w, cx| {
                        let index = w.workspaces.iter().position(|ws| ws.id == id);
                        if index.is_some() {
                            w.open_finish_task(index, window, cx);
                        }
                    });
                });
                crate::toasts::show(
                    window,
                    cx,
                    crate::toasts::Toast::success(
                        "Finish can update the main checkout and archive the task.",
                    )
                    .title(format!("{name} was merged."))
                    .action("Clean Up…", clean_up)
                    .lifetime(Some(Duration::from_secs(30))),
                );
            }
        });
    }

    /// Pairs of workspaces that just started changing the same files: a
    /// toast when this window is in front, a desktop notification when it
    /// isn't (`task_overlap_notify`), once per pair.
    fn note_new_overlaps(&mut self, found: &[RepositoryTasks], cx: &mut Context<Self>) {
        let mut started = Vec::new();
        for repository in found {
            let now: std::collections::HashSet<String> =
                repository.pairs.iter().map(|p| p.key()).collect();
            let known = self
                .known_overlaps
                .insert(repository.common.clone(), now)
                .unwrap_or_default();
            started.extend(
                repository
                    .pairs
                    .iter()
                    .filter(|p| !known.contains(&p.key()))
                    .cloned(),
            );
        }
        if started.is_empty() || !crate::settings_store::SettingsStore::get(cx).task_overlap_notify
        {
            return;
        }
        let roots: Vec<String> = self
            .workspaces
            .iter()
            .map(|w| task_registry::canonical(&w.root.to_string_lossy()))
            .collect();
        let Some((_, handle)) = crate::session::workbenches(cx)
            .into_iter()
            .find(|(w, _)| w.entity_id() == cx.entity_id())
        else {
            return;
        };
        let _ = handle.update(cx, move |_, window, cx| {
            for pair in started
                .iter()
                .filter(|p| roots.contains(&p.a.path) || roots.contains(&p.b.path))
            {
                let count = pair.files.len();
                let sentence = format!(
                    "{} and {} now change the same {}",
                    pair.a.name,
                    pair.b.name,
                    if count == 1 {
                        "file".to_string()
                    } else {
                        format!("{count} files")
                    }
                );
                if window.is_window_active() {
                    let mut detail = pair
                        .files
                        .iter()
                        .take(3)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ");
                    if count > 3 {
                        detail.push_str(", …");
                    }
                    toast(window, cx, ToastKind::Warning, sentence, Some(detail), None);
                } else {
                    let _ = crate::agent_notify::post(
                        "Tasks change the same files",
                        "",
                        &sentence,
                        None,
                    );
                }
            }
        });
    }

    /// The repository tasks are made from: the workspace at `index`'s, else
    /// the active one's. Its main checkout, as tasks always branch from there.
    pub(crate) fn task_repository(&self, index: Option<usize>) -> Option<String> {
        let workspace = self.workspaces.get(index.unwrap_or(self.active))?;
        if workspace.kind != Kind::Folder {
            return None;
        }
        let root = workspace.root.to_string_lossy().into_owned();
        branches::repo_root(&root).map(|r| main_checkout_root(&r))
    }
}

// MARK: Project settings and their trust

/// Trusted project.toml files, kept in the data directory as
/// `project-trust.json` (`{"trusted": {root: digest}}`, as the Swift app
/// keeps them in its defaults).
fn project_trust_file() -> PathBuf {
    impulse_kit::paths::data_dir(cfg!(debug_assertions)).join("project-trust.json")
}

pub(crate) fn project_trust() -> ProjectTrust {
    std::fs::read(project_trust_file())
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

pub(crate) fn save_project_trust(trust: &ProjectTrust) {
    if let Ok(data) = serde_json::to_vec_pretty(trust) {
        let _ = std::fs::write(project_trust_file(), data);
    }
}

/// The settings for the checkout at `root`, which are its repository's: the
/// main checkout's `.impulse/project.toml`, committed or not, so every task
/// gets what Project Setup saved, then the local `.git/impulse/project.toml`.
pub(crate) fn load_project_config(root: &str) -> Option<Loaded> {
    ProjectConfig::load(
        &main_checkout_root(root),
        branches::common_git_directory(root).as_deref(),
    )
}

/// The project's settings when they parse (no trust needed to read them).
pub(crate) fn project_config(root: &str) -> Option<ProjectConfig> {
    load_project_config(root)?.config.ok()
}

/// What a settings file is trusted under: the committed file per main
/// repository, the local file by its own path.
pub(crate) fn trust_key(source_path: &str, loaded: &Loaded, root: &str) -> String {
    if loaded.local.as_ref().map(|l| l.path.as_str()) == Some(source_path) {
        return source_path.to_string();
    }
    repository_trust_key(root)
}

/// The main repository a committed settings file at `root` is trusted
/// under (Swift's `ProjectActions.trustKey(for:)`).
pub(crate) fn repository_trust_key(root: &str) -> String {
    branches::common_git_directory(root)
        .map(|c| parent_of(&c))
        .unwrap_or_else(|| root.to_string())
}

/// The project's settings when every file that needs trusting is trusted
/// already; `None` otherwise. Never asks.
pub(crate) fn already_trusted_project_config(root: &str) -> Option<ProjectConfig> {
    let loaded = load_project_config(root)?;
    let config = loaded.config.clone().ok()?;
    let store = project_trust();
    loaded
        .sources()
        .iter()
        .all(|s| store.is_trusted(&trust_key(&s.path, &loaded, root), &s.digest))
        .then_some(config)
}

/// A command as the trust prompt lists it: `$ ` before its first line, its
/// other lines indented, and characters that would hide what it does
/// (control characters, direction overrides, zero-width ones) written out
/// as `\u{202E}`.
pub(crate) fn listed_command(command: &str) -> String {
    let hidden = |c: char| {
        c != '\t'
            && (c.is_control()
                || matches!(c as u32,
                    0xAD | 0x61C | 0x180E | 0x200B..=0x200F | 0x202A..=0x202E
                    | 0x2060..=0x2064 | 0x2066..=0x206F | 0xFEFF | 0xFFF9..=0xFFFB))
    };
    command
        .replace("\r\n", "\n")
        .split(['\n', '\r'])
        .enumerate()
        .map(|(index, line)| {
            let mut shown = String::from(if index == 0 { "$ " } else { "  " });
            for c in line.chars() {
                if hidden(c) {
                    shown.push_str(&format!("\\u{{{:X}}}", c as u32));
                } else {
                    shown.push(c);
                }
            }
            shown
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A settings file the trust prompt asks about: what it's trusted under,
/// its digest, what it runs and sets, and whether it's the local file.
struct Untrusted {
    key: String,
    digest: String,
    commands: Vec<String>,
    values: Vec<String>,
    local: bool,
}

/// What the flows continue with on the workbench.
pub(crate) type Then<T> = Box<dyn FnOnce(&mut Workbench, T, &mut Window, &mut Context<Workbench>)>;

impl Workbench {
    /// The project's settings once trusted; `None` when there are none,
    /// they don't parse, or the user declined. Each settings file with
    /// commands is trusted on its own, for exactly its content.
    pub(crate) fn trust_project_config(
        &mut self,
        root: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: Then<Option<ProjectConfig>>,
    ) {
        let Some(loaded) = load_project_config(root) else {
            return then(self, None, window, cx);
        };
        let config = match loaded.config.clone() {
            Ok(config) => config,
            Err(error) => {
                toast(
                    window,
                    cx,
                    ToastKind::Warning,
                    error.to_string(),
                    None,
                    None,
                );
                return then(self, None, window, cx);
            }
        };
        let store = project_trust();
        let untrusted: Vec<Untrusted> = loaded
            .sources()
            .into_iter()
            .filter(|s| !store.is_trusted(&trust_key(&s.path, &loaded, root), &s.digest))
            .map(|s| Untrusted {
                key: trust_key(&s.path, &loaded, root),
                digest: s.digest.clone(),
                commands: s.commands.clone(),
                values: s.values.clone(),
                local: loaded.local.as_ref().map(|l| &l.path) == Some(&s.path),
            })
            .collect();
        if untrusted.is_empty() {
            return then(self, Some(config), window, cx);
        }
        let commands: Vec<String> = untrusted.iter().flat_map(|u| u.commands.clone()).collect();
        let values: Vec<String> = untrusted
            .iter()
            .flat_map(|u| u.values.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let name = last_component(root);
        let title = if untrusted.len() == 1 {
            let file = if untrusted[0].local {
                ".git/impulse/project.toml"
            } else {
                ".impulse/project.toml"
            };
            format!("Trust {file} in {name}?")
        } else {
            format!("Trust the project settings in {name}?")
        };
        let what = if untrusted.len() == 1 {
            "It".to_string()
        } else {
            ".impulse/project.toml and .git/impulse/project.toml".to_string()
        };
        let mut message = String::new();
        if !commands.is_empty() {
            message = if commands.len() == 1 {
                format!("{what} can run the command below on {}. ", this_computer())
            } else {
                format!(
                    "{what} can run all {} commands below on {}. ",
                    commands.len(),
                    this_computer()
                )
            };
        }
        if !values.is_empty() {
            message += &format!(
                "{} sets {} in each new task's env file. ",
                if commands.is_empty() {
                    what.as_str()
                } else {
                    "It also"
                },
                values.join(", ")
            );
        }
        message += if untrusted.len() == 1 {
            "You'll be asked again if the file changes."
        } else {
            "You'll be asked again if the files change."
        };
        let listed = commands
            .iter()
            .map(|c| listed_command(c))
            .collect::<Vec<_>>()
            .join("\n");
        let then = Rc::new(RefCell::new(Some(then)));
        let workbench = cx.entity().downgrade();
        let config = Rc::new(config);
        let untrusted = Rc::new(untrusted);
        let answer = {
            let (then, workbench, config, untrusted) = (
                then.clone(),
                workbench.clone(),
                config.clone(),
                untrusted.clone(),
            );
            move |trusted: bool, window: &mut Window, cx: &mut App| {
                window.close_dialog(cx);
                let Some(then) = then.borrow_mut().take() else {
                    return;
                };
                if trusted {
                    let mut store = project_trust();
                    for file in untrusted.iter() {
                        store.trust(&file.key, &file.digest);
                    }
                    save_project_trust(&store);
                }
                let config = trusted.then(|| (*config).clone());
                let _ = workbench.update(cx, |workbench, cx| then(workbench, config, window, cx));
            }
        };
        let answer = Rc::new(answer);
        let (ok, cancel) = (answer.clone(), answer);
        let lines = listed.lines().count();
        let mut alert = crate::alert::Alert::new(title).message(message);
        if !listed.is_empty() {
            let palette = crate::alert::palette(cx);
            alert = alert.accessory(Some(440.), move |_, _| {
                crate::alert::command_list(listed.clone(), lines, &palette).into_any_element()
            });
        }
        alert
            .button("Trust and Run", move |window, cx| ok(true, window, cx))
            .button("Cancel", move |window, cx| cancel(false, window, cx))
            .show(window, cx);
    }
}

/// Where project commands run, as the trust prompt says it.
fn this_computer() -> &'static str {
    if cfg!(target_os = "macos") {
        "your Mac"
    } else {
        "this computer"
    }
}

/// Run a project script in `directory` (off the main thread); false when
/// it fails or takes over two minutes. `extra` goes into its environment
/// (a task's `IMPULSE_TASK`, say).
pub(crate) fn run_script(script: &str, directory: &str, extra: &[(String, String)]) -> bool {
    ChildProcess::new("/bin/sh")
        .args(["-c", script])
        .dir(directory)
        .envs(extra.iter().cloned())
        .env("PATH", impulse_kit::shell::login::login_path())
        .timeout(Duration::from_secs(120))
        .run()
        .map(|o| !o.timed_out && o.status == 0)
        .unwrap_or(false)
}

// MARK: Preparing a new task

/// The untracked files a new task gets from `root`: `.worktreeinclude`'s
/// patterns (or the defaults) plus the project settings' `[worktrees] copy`.
pub(crate) fn task_copies(root: &str) -> Vec<String> {
    let include = std::fs::read_to_string(Path::new(root).join(".worktreeinclude")).ok();
    let mut patterns = worktree_tasks::include_patterns(include.as_deref());
    patterns.extend(
        project_config(root)
            .map(|c| c.worktree_copy)
            .unwrap_or_default(),
    );
    worktree_tasks::matching_files(&patterns, root)
}

/// The env file, when the settings give tasks values of their own and the
/// main checkout has one that `copies` doesn't already include.
pub(crate) fn env_file_copy(
    config: Option<&ProjectConfig>,
    root: &str,
    copies: &[String],
) -> Vec<String> {
    match config {
        Some(c)
            if c.has_task_values()
                && is_inside(&c.env_file)
                && !copies.contains(&c.env_file)
                && Path::new(root).join(&c.env_file).exists() =>
        {
            vec![c.env_file.clone()]
        }
        _ => Vec::new(),
    }
}

/// Copy `files` into a new task. A link is copied as a link, as Swift's
/// `FileManager.copyItem` does: followed, an untracked `.env` linking to a
/// key elsewhere would become a real copy of the key inside the task.
pub(crate) fn copy_files(files: &[String], root: &str, destination: &str) {
    for relative in files {
        let target = Path::new(destination).join(relative);
        if std::fs::symlink_metadata(&target).is_ok() {
            continue;
        }
        if let Some(parent) = target.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ =
            impulse_kit::project::folder_clone::copy_item(&Path::new(root).join(relative), &target);
    }
}

/// What a new task needs from trusted settings before its first terminal
/// opens: the database's data folder cloned (its service stopped in the
/// main checkout meanwhile), then its own values written into its env
/// file. Off the main thread.
pub(crate) fn prepare_task(
    config: Option<&ProjectConfig>,
    root: &str,
    path: &str,
    slot: Option<i64>,
) {
    let Some(config) = config else {
        return;
    };
    if let Some(folder) = &config.database_folder {
        clone_database(folder, config.database_service.as_deref(), root, path);
    }
    if let Some(slot) = slot {
        write_task_values(config, path, slot);
    }
}

/// Write a task's own values into its env file: its ports and
/// `[worktrees.env]`, and with `compose_override`, `COMPOSE_FILE` naming the
/// override written for it. False when there was nothing to write.
pub(crate) fn write_task_values(config: &ProjectConfig, path: &str, slot: i64) -> bool {
    if !is_inside(&config.env_file) {
        return false;
    }
    let task = last_component(path);
    let mut values = if config.has_task_values() {
        task_environment::values(config, &task, slot)
    } else {
        Vec::new()
    };
    if config.compose_override {
        if let Some(files) = write_compose_override(config, path, &task, slot) {
            values.push(task_environment::EnvValue::new("COMPOSE_FILE", files));
        }
    }
    if values.is_empty() {
        return false;
    }
    let file = Path::new(path).join(&config.env_file);
    let current = std::fs::read_to_string(&file).unwrap_or_default();
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let text = task_environment::applying(&values, &current, &format!("Impulse task {task}"));
    std::fs::write(&file, text).is_ok()
}

/// The task's Compose override, in `.git/impulse/tasks/<task>/` so the
/// project's own files don't change; `COMPOSE_FILE` for it.
fn write_compose_override(
    config: &ProjectConfig,
    path: &str,
    task: &str,
    slot: i64,
) -> Option<String> {
    let (name, text) = compose_file::find(path)?;
    let common = branches::common_git_directory(path)?;
    let text =
        compose_file::ComposeFile::parse(&text).task_override(task, slot, config.port_offset)?;
    let folder = task_folder(task, &common);
    let file = Path::new(&folder).join("compose.override.yml");
    std::fs::create_dir_all(&folder).ok()?;
    std::fs::write(&file, text).ok()?;
    let own = [
        "compose.override.yaml",
        "compose.override.yml",
        "docker-compose.override.yaml",
        "docker-compose.override.yml",
    ]
    .into_iter()
    .find(|f| Path::new(path).join(f).exists());
    let mut files = vec![name];
    files.extend(own.map(String::from));
    files.push(file.to_string_lossy().into_owned());
    Some(files.join(":"))
}

/// Clone a database's data folder into a task, its Compose service in the
/// main checkout stopped for the copy and started again.
fn clone_database(folder: &str, service: Option<&str>, root: &str, path: &str) {
    let mut restart = false;
    if let Some(service) = service {
        let running =
            compose(&["ps", "--status", "running", "--services"], root).unwrap_or_default();
        if running.lines().any(|l| l == service) {
            restart = compose(&["stop", service], root).is_some();
        }
    }
    folder_clone::clone(folder, root, path);
    if restart {
        if let Some(service) = service {
            let _ = compose(&["start", service], root);
        }
    }
}

/// `docker compose <arguments>` in `directory` with the login shell's
/// PATH; its output, or `None` when it fails or takes over a minute.
fn compose(arguments: &[&str], directory: &str) -> Option<String> {
    let output = ChildProcess::new("/usr/bin/env")
        .arg("docker")
        .arg("compose")
        .args(arguments)
        .dir(directory)
        .env("PATH", impulse_kit::shell::login::login_path())
        .timeout(Duration::from_secs(60))
        .run()
        .ok()?;
    (!output.timed_out && output.status == 0).then(|| output.stdout_text())
}

// MARK: Opening a task's workspace

impl Workbench {
    /// Open `folder` as a workspace whose first terminal runs `command`
    /// (the setup script, the agent), or show it if it's open already.
    pub(crate) fn open_task_workspace(
        &mut self,
        folder: &Path,
        command: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> usize {
        let folder = crate::workspace::normalize(folder);
        if let Some(index) = self
            .workspaces
            .iter()
            .position(|w| w.kind == Kind::Folder && w.root == folder)
        {
            self.activate_workspace(index, window, cx);
            return index;
        }
        let id = self.next_id();
        // Before Scratch, which stays last; the sidebar groups it with its
        // repository.
        let at = self
            .workspaces
            .iter()
            .position(|w| w.kind == Kind::Scratch)
            .unwrap_or(self.workspaces.len());
        self.workspaces
            .insert(at, Workspace::new(id, Kind::Folder, folder.clone()));
        let terminal = self.new_terminal(folder, command.into_iter().collect(), window, cx);
        self.add_tab(at, terminal, window, cx);
        self.activate_workspace(at, window, cx);
        self.refresh_task_states(cx);
        at
    }

    /// The index of the folder workspace at `path`, if one is open.
    pub(crate) fn workspace_at(&self, path: &str) -> Option<usize> {
        let wanted = task_registry::canonical(path);
        self.workspaces.iter().position(|w| {
            w.kind == Kind::Folder && task_registry::canonical(&w.root.to_string_lossy()) == wanted
        })
    }

    /// Close the workspace with `id` for a task flow: one with unsaved files
    /// stays open (the notice says so) and `false` comes back.
    pub(crate) fn close_workspace_for_task(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(index) = self.workspaces.iter().position(|w| w.id == id) else {
            return true;
        };
        let dirty = self.workspaces[index]
            .all_surfaces()
            .any(|s| s.is_dirty(cx));
        if dirty {
            let name = self.workspaces[index].name();
            self.set_notice(
                format!("{name} has unsaved files. Save or close them first."),
                cx,
            );
            return false;
        }
        // The task flow has its own Undo, so there's no Undo Close.
        self.remove_workspace(index, false, window, cx);
        true
    }

    // MARK: New Task from Branch

    /// "New Task from Branch...": an existing branch opened as a task, in a
    /// folder beside the repository like New Task (copies, the task list,
    /// setup). A remote branch gets a local branch of the same name that
    /// tracks it, so pushing goes back to it.
    pub(crate) fn open_branch_as_task(
        &mut self,
        name: String,
        is_remote: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.task_repository(None) else {
            return self.set_notice("Open a folder in a git repository first.".into(), cx);
        };
        let branch = if is_remote {
            name.split_once('/')
                .map(|(_, b)| b.to_string())
                .unwrap_or(name.clone())
        } else {
            name.clone()
        };
        let path = worktree_tasks::worktree_path(&root, &branch);
        let trust = self.trust.clone();
        let work = {
            let (root, path, branch, name) =
                (root.clone(), path.clone(), branch.clone(), name.clone());
            cx.background_executor().spawn(async move {
                let copies = task_copies(&root);
                if Path::new(&path).exists() {
                    return Err(format!("{} already exists.", abbreviate_home(&path)));
                }
                let _ = std::fs::create_dir_all(parent_of(&path));
                let added = if is_remote {
                    operations::add_worktree(&path, &branch, true, Some(&name), true, &root)
                } else {
                    operations::add_worktree(&path, &name, false, None, false, &root)
                };
                added.map_err(|e| e.message())?;
                // Someone else's branch stays restricted even inside a
                // trusted folder, until it's trusted itself.
                if is_remote {
                    trust.restrict(&path);
                }
                let config = project_config(&path);
                let mut files = copies.clone();
                files.extend(env_file_copy(config.as_ref(), &root, &copies));
                copy_files(&files, &root, &path);
                for folder in config
                    .as_ref()
                    .map(|c| c.worktree_clone.clone())
                    .unwrap_or_default()
                {
                    folder_clone::clone(&folder, &root, &path);
                }
                // The base it will be merged into: what New Task would start from.
                let main = snapshot::snapshot(&root, false);
                let base = main
                    .map(|s| {
                        worktree_tasks::default_base(
                            s.branch.as_deref(),
                            s.upstream.as_deref(),
                            s.ahead,
                        )
                        .0
                    })
                    .unwrap_or_default();
                let record = record_created(
                    &path,
                    &branch,
                    &base,
                    &root,
                    is_remote,
                    Provenance::default(),
                    |slot| ports_are_free(config.as_ref(), slot),
                );
                Ok(record.and_then(|r| r.slot))
            })
        };
        cx.spawn_in(window, async move |workbench, cx| {
            let result = work.await;
            let _ = workbench.update_in(cx, |workbench, window, cx| {
                let slot = match result {
                    Ok(slot) => slot,
                    Err(failure) => {
                        return toast(window, cx, ToastKind::Warning, failure, None, None);
                    }
                };
                let opened: Then<Option<String>> = {
                    let (path, branch) = (path.clone(), branch.clone());
                    Box::new(move |workbench, setup, window, cx| {
                        workbench.open_task_workspace(Path::new(&path), setup, window, cx);
                        toast(
                            window,
                            cx,
                            ToastKind::Success,
                            format!("Opened {branch} as a task."),
                            None,
                            None,
                        );
                    })
                };
                if is_remote {
                    // Someone else's branch: its values come only from
                    // settings trusted already, and its setup script is
                    // asked about every time.
                    let config = already_trusted_project_config(&path);
                    let (root2, path2) = (root.clone(), path.clone());
                    workbench.after_preparing(
                        config,
                        root2,
                        path2,
                        slot,
                        window,
                        cx,
                        Box::new(move |workbench, _, window, cx| {
                            workbench
                                .confirm_branch_script(&name, &path, "setup", window, cx, opened)
                        }),
                    );
                } else {
                    if workbench.trust.is_trusted(&root) {
                        workbench.trust.trust(&path);
                    }
                    let (root2, path2) = (root.clone(), path.clone());
                    workbench.trust_project_config(
                        &path,
                        window,
                        cx,
                        Box::new(move |workbench, config, window, cx| {
                            let setup = config.as_ref().and_then(|c| c.setup_script.clone());
                            workbench.after_preparing(
                                config,
                                root2,
                                path2,
                                slot,
                                window,
                                cx,
                                Box::new(move |workbench, _, window, cx| {
                                    opened(workbench, setup, window, cx)
                                }),
                            );
                        }),
                    );
                }
            });
        })
        .detach();
    }

    /// `prepare_task` off the main thread, then `then`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn after_preparing(
        &mut self,
        config: Option<ProjectConfig>,
        root: String,
        path: String,
        slot: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: Then<()>,
    ) {
        let needed = config.as_ref().is_some_and(|c| {
            c.database_folder.is_some() || c.has_task_values() || c.compose_override
        });
        if !needed {
            return then(self, (), window, cx);
        }
        let work = cx
            .background_executor()
            .spawn(async move { prepare_task(config.as_ref(), &root, &path, slot) });
        cx.spawn_in(window, async move |workbench, cx| {
            work.await;
            let _ =
                workbench.update_in(cx, |workbench, window, cx| then(workbench, (), window, cx));
        })
        .detach();
    }

    /// A branch's setup or archive script from settings that may come from
    /// someone else's branch: run only when the user says so, every time
    /// (the branch decides what its commands do). `then` gets the script to
    /// run, or `None`.
    pub(crate) fn confirm_branch_script(
        &mut self,
        branch: &str,
        root: &str,
        which: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: Then<Option<String>>,
    ) {
        let config = project_config(root);
        let script = match which {
            "setup" => config.and_then(|c| c.setup_script),
            _ => config.and_then(|c| c.archive_script),
        };
        let Some(script) = script else {
            return then(self, None, window, cx);
        };
        let (title, message, run) = if which == "setup" {
            (
                format!("Run the setup script for {branch}?"),
                format!(
                    "The task for {branch} is set up with this script:\n\n{}\n\nIt runs in the branch's checkout, where the branch decides what these commands do (a changed package.json script, for example). Run it only if you trust the branch's changes. The task opens either way.",
                    listed_command(&script)
                ),
                "Run Setup Script",
            )
        } else {
            (
                format!("Run the archive script for {branch}?"),
                format!(
                    "Archiving {branch} runs this script first:\n\n{}\n\nIt runs in the branch's checkout, which isn't a trusted folder, and the branch decides what these commands do (a changed package.json script, for example). Run it only if you trust the branch's changes. The task is archived either way.",
                    listed_command(&script)
                ),
                "Run Archive Script",
            )
        };
        let workbench = cx.entity().downgrade();
        confirm(
            window,
            cx,
            title,
            message,
            run,
            "Don't Run",
            false,
            move |yes, window, cx| {
                let _ = workbench.update(cx, |workbench, cx| {
                    then(workbench, yes.then_some(script), window, cx)
                });
            },
        );
    }
}

// MARK: Toasts and confirmations

/// A toast's look.
#[derive(Clone, Copy)]
pub(crate) enum ToastKind {
    Info,
    Success,
    Warning,
}

/// Something to run later with the window (a toast's Undo, say).
pub(crate) type Run = Rc<dyn Fn(&mut Window, &mut App)>;

/// What a toast's button does (Undo, Restore Task).
pub(crate) type ToastAction = (&'static str, Run);

/// A dialog's answer, given once.
type Answer = Rc<RefCell<Option<Box<dyn FnOnce(bool, &mut Window, &mut App)>>>>;

/// A toast, as the Swift app's `Toast`: a message, an optional detail and
/// an optional action (`toasts`).
pub(crate) fn toast(
    window: &mut Window,
    cx: &mut App,
    kind: ToastKind,
    message: String,
    detail: Option<String>,
    action: Option<ToastAction>,
) {
    let kind = match kind {
        ToastKind::Info => crate::toasts::Kind::Info,
        ToastKind::Success => crate::toasts::Kind::Success,
        ToastKind::Warning => crate::toasts::Kind::Warning,
    };
    let mut note = match detail {
        Some(detail) => crate::toasts::Toast::new(kind, detail).title(message),
        None => crate::toasts::Toast::new(kind, message),
    };
    if let Some((label, run)) = action {
        note = note.action(label, run);
    }
    crate::toasts::show(window, cx, note);
}

/// Ask a yes-or-no question in a dialog; `answer` hears it once.
#[allow(clippy::too_many_arguments)]
pub(crate) fn confirm(
    window: &mut Window,
    cx: &mut App,
    title: String,
    message: String,
    ok: &'static str,
    cancel: &'static str,
    destructive: bool,
    answer: impl FnOnce(bool, &mut Window, &mut App) + 'static,
) {
    let answer: Answer = Rc::new(RefCell::new(Some(Box::new(answer))));
    #[cfg(debug_assertions)]
    {
        let press = |yes: bool| -> Run {
            let answer = answer.clone();
            Rc::new(move |window, cx| {
                window.close_dialog(cx);
                if let Some(answer) = answer.borrow_mut().take() {
                    answer(yes, window, cx)
                }
            })
        };
        crate::debug_snapshot::note_sheet(
            vec![title.clone(), message.clone()],
            vec![(cancel, press(false)), (ok, press(true))],
        );
    }
    let reply = |yes: bool| {
        let answer = answer.clone();
        move |window: &mut Window, cx: &mut App| {
            window.close_dialog(cx);
            if let Some(answer) = answer.borrow_mut().take() {
                answer(yes, window, cx)
            }
        }
    };
    let alert = crate::alert::Alert::new(title)
        .message(message)
        .button(ok, reply(true))
        .button(cancel, reply(false))
        // Escape answers no, whatever the button says.
        .escape(1);
    if destructive {
        alert.destructive()
    } else {
        alert
    }
    .show(window, cx);
}
