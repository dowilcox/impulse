//! Watching a repository's working tree and git folders (FSEvents on macOS,
//! inotify on Linux, through `notify`) and reporting debounced changes: edits
//! in the working tree (from TUIs and agents too), index writes, HEAD and ref
//! moves, and operation state files, in normal checkouts and linked worktrees
//! alike (the Swift `RepoWatcher`).
//!
//! Events that can't change what this checkout shows are dropped here, so
//! they never cost a status refresh: anything under a gitignored folder
//! (node_modules, build output), and in a linked worktree everything in the
//! shared git folder but the refs (the main checkout's index and HEAD, other
//! worktrees' own files).

use std::collections::HashMap;
use std::ops::{BitOr, BitOrAssign};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use git2::Repository;
use notify::event::{AccessKind, AccessMode};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// What changed, as a set of flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Change(u8);

impl Change {
    pub const NONE: Change = Change(0);
    /// A file in the working tree changed.
    pub const WORKING_TREE: Change = Change(1 << 0);
    /// The index changed (stage, unstage, commit, checkout).
    pub const INDEX: Change = Change(1 << 1);
    /// HEAD, a branch, tag, remote ref or stash moved.
    pub const REFS: Change = Change(1 << 2);
    /// An operation started, advanced or finished (MERGE_HEAD,
    /// rebase-merge/...).
    pub const OPERATION: Change = Change(1 << 3);

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn contains(self, other: Change) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for Change {
    type Output = Change;
    fn bitor(self, other: Change) -> Change {
        Change(self.0 | other.0)
    }
}

impl BitOrAssign for Change {
    fn bitor_assign(&mut self, other: Change) {
        self.0 |= other.0;
    }
}

type Handler = Box<dyn Fn(Change) + Send + Sync>;

struct Shared {
    root: String,
    git_dir: String,
    common_dir: String,
    debounce: Duration,
    handler: Handler,
    state: Mutex<State>,
}

struct State {
    pending: Change,
    flush_scheduled: bool,
    /// Gone with the watcher: a flush on its way delivers nothing.
    alive: bool,
    ignores: IgnoreFilter,
}

pub struct RepoWatcher {
    shared: Arc<Shared>,
    watcher: Option<RecommendedWatcher>,
}

impl RepoWatcher {
    /// `handler` is called with the accumulated changes on a background
    /// thread, at most once per `debounce`.
    pub fn new(
        root: &str,
        git_dir: &str,
        common_dir: &str,
        debounce: Duration,
        handler: impl Fn(Change) + Send + Sync + 'static,
    ) -> Self {
        let root = canonical(root);
        RepoWatcher {
            shared: Arc::new(Shared {
                git_dir: canonical(git_dir),
                common_dir: canonical(common_dir),
                debounce,
                handler: Box::new(handler),
                state: Mutex::new(State {
                    pending: Change::NONE,
                    flush_scheduled: false,
                    alive: true,
                    ignores: IgnoreFilter::new(&root),
                }),
                root,
            }),
            watcher: None,
        }
    }

    pub fn start(&mut self) -> notify::Result<()> {
        if self.watcher.is_some() {
            return Ok(());
        }
        let shared = &self.shared;
        let mut paths = vec![shared.root.clone()];
        // Linked worktrees keep their git folders outside the working tree
        // (their own one inside the shared one).
        for dir in [&shared.common_dir, &shared.git_dir] {
            if !paths
                .iter()
                .any(|p| dir == p || dir.starts_with(&format!("{p}/")))
            {
                paths.push(dir.clone());
            }
        }
        let receiver = Arc::downgrade(&self.shared);
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                let (Ok(event), Some(shared)) = (event, receiver.upgrade()) else {
                    return;
                };
                // Reads aren't changes; a write is seen as its modify events.
                if let EventKind::Access(kind) = event.kind {
                    if kind != AccessKind::Close(AccessMode::Write) {
                        return;
                    }
                }
                let paths: Vec<String> = event
                    .paths
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                receive(&shared, &paths);
            })?;
        for path in &paths {
            watcher.watch(Path::new(path), RecursiveMode::Recursive)?;
        }
        self.watcher = Some(watcher);
        Ok(())
    }

    pub fn stop(&mut self) {
        self.watcher = None;
    }

    /// What a change at `path` means for the checkout at `root`, whose own
    /// git files are in `git_dir` and shared ones in `common_dir` (all
    /// canonical). `is_ignored` gets working-tree paths relative to `root`,
    /// a folder with a trailing "/".
    pub fn classify(
        path: &str,
        root: &str,
        git_dir: &str,
        common_dir: &str,
        is_ignored: &mut dyn FnMut(&str) -> bool,
    ) -> Change {
        // A linked worktree's own git folder is inside the shared one.
        if let Some(relative) = relative(path, git_dir) {
            return Self::classify_git_path(relative);
        }
        if let Some(relative) = relative(path, common_dir) {
            return Self::classify_shared_git_path(relative);
        }
        if let Some(relative) = relative(path, root) {
            // The `.git` folder of a normal checkout is under the root.
            if relative == ".git" || relative.starts_with(".git/") {
                return Change::NONE;
            }
            return Self::classify_working_tree_path(relative, is_ignored);
        }
        Change::NONE
    }

    /// A path in the working tree matters unless it's ignored or in an
    /// ignored folder (checked from the top, so `node_modules/` is one
    /// lookup for everything in it).
    pub fn classify_working_tree_path(
        relative: &str,
        is_ignored: &mut dyn FnMut(&str) -> bool,
    ) -> Change {
        let components: Vec<&str> = relative.split('/').filter(|c| !c.is_empty()).collect();
        let Some((name, folders)) = components.split_last() else {
            return Change::WORKING_TREE;
        };
        let mut folder = String::new();
        for component in folders {
            folder.push_str(component);
            folder.push('/');
            if is_ignored(&folder) {
                return Change::NONE;
            }
        }
        // New ignore rules change what status lists, whatever they match.
        if *name == ".gitignore" {
            return Change::WORKING_TREE;
        }
        if is_ignored(relative) {
            Change::NONE
        } else {
            Change::WORKING_TREE
        }
    }

    /// Whether a change at `path` can change what's ignored.
    pub fn changes_ignore_rules(path: &str) -> bool {
        path.ends_with("/.gitignore") || path.ends_with("/info/exclude")
    }

    /// A path in this checkout's own git folder (all of `.git` in a normal
    /// checkout).
    pub fn classify_git_path(relative: &str) -> Change {
        // Object writes and lock files are noise; the meaningful change
        // follows.
        if relative.starts_with("objects/")
            || relative.ends_with(".lock")
            || relative.starts_with("logs/")
            || relative.contains("impulse-snapshot-")
        {
            return Change::NONE;
        }
        if relative == "index" {
            return Change::INDEX;
        }
        if matches!(
            relative,
            "HEAD" | "ORIG_HEAD" | "FETCH_HEAD" | "packed-refs"
        ) || relative.starts_with("refs/")
        {
            return Change::REFS;
        }
        if relative.starts_with("rebase-merge")
            || relative.starts_with("rebase-apply")
            || [
                "MERGE_HEAD",
                "CHERRY_PICK_HEAD",
                "REVERT_HEAD",
                "BISECT_LOG",
                "sequencer",
            ]
            .iter()
            .any(|name| relative == *name || relative.starts_with(&format!("{name}/")))
        {
            return Change::OPERATION;
        }
        if relative == "info/exclude" {
            return Change::WORKING_TREE;
        }
        // Everything else, `worktrees/` included: a linked worktree's index
        // and HEAD are its own (its watcher sees them), not this checkout's.
        Change::NONE
    }

    /// A path in the git folder a linked worktree shares with the main
    /// checkout: only the refs (and the ignore rules) are shared. The index,
    /// HEAD and operation files there are the main checkout's.
    pub fn classify_shared_git_path(relative: &str) -> Change {
        if relative.ends_with(".lock") {
            return Change::NONE;
        }
        if relative == "packed-refs" || relative.starts_with("refs/") {
            return Change::REFS;
        }
        if relative == "info/exclude" {
            return Change::WORKING_TREE;
        }
        Change::NONE
    }
}

impl Drop for RepoWatcher {
    fn drop(&mut self) {
        self.watcher = None;
        self.shared.state.lock().unwrap().alive = false;
    }
}

fn receive(shared: &Arc<Shared>, paths: &[String]) {
    let mut state = shared.state.lock().unwrap();
    let mut change = Change::NONE;
    for path in paths {
        let found = RepoWatcher::classify(
            path,
            &shared.root,
            &shared.git_dir,
            &shared.common_dir,
            &mut |p| state.ignores.is_ignored(p),
        );
        // The index decides which ignored files are tracked anyway.
        if !found.is_empty()
            && (found.contains(Change::INDEX) || RepoWatcher::changes_ignore_rules(path))
        {
            state.ignores.invalidate();
        }
        change |= found;
    }
    if change.is_empty() {
        return;
    }
    state.pending |= change;
    if state.flush_scheduled {
        return;
    }
    state.flush_scheduled = true;
    let shared = Arc::clone(shared);
    std::thread::spawn(move || {
        std::thread::sleep(shared.debounce);
        let changes = {
            let mut state = shared.state.lock().unwrap();
            state.flush_scheduled = false;
            if !state.alive {
                return;
            }
            std::mem::take(&mut state.pending)
        };
        (shared.handler)(changes);
    });
}

fn relative<'a>(path: &'a str, base: &str) -> Option<&'a str> {
    if path == base {
        return Some("");
    }
    path.strip_prefix(base)?.strip_prefix('/')
}

/// Symlinks resolved, as the event paths come (FSEvents reports
/// /private/var for /var).
fn canonical(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string())
}

/// Which working-tree paths are ignored as far as status goes: matched by an
/// ignore rule, and nothing in them tracked (git still follows a tracked file
/// under an ignored folder). Answers are kept until the ignore rules or the
/// index change, so a burst of events under `node_modules/` costs one lookup.
pub struct IgnoreFilter {
    root: String,
    repo: Option<Repository>,
    /// The repository's index (libgit2 keeps it loaded) matches the file.
    index_is_current: bool,
    known: HashMap<String, bool>,
}

const IGNORE_FILTER_LIMIT: usize = 4_096;

impl IgnoreFilter {
    pub fn new(root: &str) -> Self {
        IgnoreFilter {
            root: root.to_string(),
            repo: None,
            index_is_current: false,
            known: HashMap::new(),
        }
    }

    /// `relative` is relative to the working tree; a folder ends in "/".
    pub fn is_ignored(&mut self, relative: &str) -> bool {
        if let Some(&answer) = self.known.get(relative) {
            return answer;
        }
        // A folder seen ignored still is once it's gone (`rm -rf build`),
        // when libgit2 can no longer tell it's a folder.
        if !relative.ends_with('/') && self.known.get(&format!("{relative}/")) == Some(&true) {
            return true;
        }
        let answer = self.check(relative);
        if self.known.len() >= IGNORE_FILTER_LIMIT {
            self.known.clear();
        }
        self.known.insert(relative.to_string(), answer);
        answer
    }

    /// Forget every answer: the ignore rules or the index changed.
    pub fn invalidate(&mut self) {
        self.known.clear();
        self.index_is_current = false;
    }

    fn check(&mut self, relative: &str) -> bool {
        if self.repo.is_none() {
            self.repo = Repository::open(&self.root).ok();
        }
        let Some(repo) = &self.repo else {
            return false;
        };
        if !repo.is_path_ignored(relative).unwrap_or(false) {
            return false;
        }
        !self.is_tracked(relative)
    }

    /// Whether the index has `relative` or anything under it. True when the
    /// index can't be read, so the change still counts.
    fn is_tracked(&mut self, relative: &str) -> bool {
        let Some(repo) = &self.repo else {
            return true;
        };
        // The repository loads its index once and hands out that one; it's
        // read again only when told the file changed.
        let Ok(mut index) = repo.index() else {
            return true;
        };
        if !self.index_is_current {
            if index.read(false).is_err() {
                return true;
            }
            self.index_is_current = true;
        }
        let name = relative.strip_suffix('/').unwrap_or(relative);
        index.get_path(Path::new(name), 0).is_some()
            || index.find_prefix(format!("{name}/")).is_ok()
    }
}
