//! The live state of one repository (the Swift `GitRepositoryState` and
//! `GitRepositoryStore`): its snapshot (branch, upstream, ahead and behind,
//! the operation in progress, staged, unstaged, untracked and conflicted
//! files) and the file tree's status letters, re-read off the main thread
//! whenever `impulse_git::RepoWatcher` sees the working tree, the index or a
//! ref change, and after every git action Impulse runs. One state per
//! repository root, shared by every window that shows it.
//!
//! Both reads go through libgit2, which never runs a program the
//! repository's configuration names (no fsmonitor hook, filter or merge
//! driver), so they're safe in folders that aren't trusted. The git CLI
//! reads Impulse makes on its own (`read_git`) run with
//! `cli::restricted_environment` there.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::channel::oneshot;
use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global, Task, WeakEntity};
use impulse_git::cli::{self, RunOptions};
use impulse_git::operation_abort;
use impulse_git::operations::{self, GitResult, OperationError};
use impulse_git::safety::{self, SafetySnapshot};
use impulse_git::{RepoSnapshot, RepoWatcher};

/// The file tree's status letters: folder path, then name, then letter
/// (`impulse_git::all_statuses`).
pub type StatusMap = HashMap<String, HashMap<String, String>>;

pub enum RepositoryEvent {
    /// HEAD moved (a commit, pull, merge or checkout, from Impulse or a
    /// terminal) while no operation was in progress, from one commit to
    /// another.
    HeadMoved { from: String, to: String },
}

pub struct RepositoryState {
    pub root: String,
    pub snapshot: Option<RepoSnapshot>,
    pub statuses: StatusMap,
    /// A refresh has finished at least once.
    pub loaded: bool,
    /// A long-running operation ("Pushing…") and its latest progress line.
    pub activity: Option<String>,
    pub activity_detail: Option<String>,
    /// Whether the folder is trusted, as the window last said.
    pub trusted: bool,
    refresh_in_flight: bool,
    refresh_queued: bool,
    waiting: Vec<oneshot::Sender<()>>,
    watcher: Option<RepoWatcher>,
    _watching: Option<Task<()>>,
    /// Operations run one at a time, as on the Swift app's serial queue.
    lock: Arc<Mutex<()>>,
    /// When the background fetch last started (`git_auto_fetch_minutes`).
    last_fetch: Option<std::time::Instant>,
    quiet_fetch_in_flight: bool,
}

impl EventEmitter<RepositoryEvent> for RepositoryState {}

impl RepositoryState {
    fn new(root: String, cx: &mut Context<Self>) -> Self {
        let mut state = RepositoryState {
            root,
            snapshot: None,
            statuses: HashMap::new(),
            loaded: false,
            activity: None,
            activity_detail: None,
            trusted: false,
            refresh_in_flight: false,
            refresh_queued: false,
            waiting: Vec::new(),
            watcher: None,
            _watching: None,
            lock: Arc::new(Mutex::new(())),
            last_fetch: None,
            quiet_fetch_in_flight: false,
        };
        state.refresh(cx);
        state
    }

    /// Read the snapshot again (bursts coalesce; never on the main thread).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.refresh_in_flight {
            self.refresh_queued = true;
            return;
        }
        self.refresh_in_flight = true;
        let root = self.root.clone();
        let trusted = self.trusted;
        cx.spawn(async move |this, cx| {
            let (snapshot, statuses) = cx
                .background_spawn(async move {
                    let snapshot = impulse_git::snapshot(&root, true);
                    // A merge, cherry-pick or revert just started (an agent's,
                    // say): note the work from before it, so Abort can put
                    // exactly that back. Only in a trusted folder: recording
                    // runs git, which can run programs the repository's
                    // configuration names.
                    let started = snapshot.as_ref().and_then(|s| s.operation.as_ref());
                    if trusted && started.is_some_and(operation_abort::applies) {
                        operation_abort::record_start_if_needed(
                            &root,
                            operation_abort::DEFAULT_RECORD_AGE,
                        );
                    }
                    let statuses = impulse_git::all_statuses(&root, false).unwrap_or_default();
                    (snapshot, statuses)
                })
                .await;
            let _ = this.update(cx, |this, cx| this.refreshed(snapshot, statuses, cx));
        })
        .detach();
    }

    fn refreshed(
        &mut self,
        snapshot: Option<RepoSnapshot>,
        statuses: StatusMap,
        cx: &mut Context<Self>,
    ) {
        self.refresh_in_flight = false;
        self.loaded = true;
        if snapshot != self.snapshot {
            let moved = match (&self.snapshot, &snapshot) {
                (Some(before), Some(after)) if after.operation.is_none() => {
                    match (&before.head_oid, &after.head_oid) {
                        (Some(from), Some(to)) if from != to => Some((from.clone(), to.clone())),
                        _ => None,
                    }
                }
                _ => None,
            };
            self.snapshot = snapshot;
            if let Some((from, to)) = moved {
                cx.emit(RepositoryEvent::HeadMoved { from, to });
            }
        }
        self.statuses = statuses;
        self.start_watching(cx);
        if self.refresh_queued {
            self.refresh_queued = false;
            self.refresh(cx);
        } else {
            for waiter in self.waiting.drain(..) {
                let _ = waiter.send(());
            }
        }
        cx.notify();
    }

    /// Resolves once the refresh in progress (or the next one) is done.
    pub fn next_refresh(&mut self, cx: &mut Context<Self>) -> oneshot::Receiver<()> {
        let (sender, receiver) = oneshot::channel();
        self.waiting.push(sender);
        if !self.refresh_in_flight {
            self.refresh(cx);
        }
        receiver
    }

    fn start_watching(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        if self.watcher.is_some() {
            return;
        }
        let (sender, receiver) = async_channel::unbounded();
        let mut watcher = RepoWatcher::new(
            &snapshot.root,
            &snapshot.git_dir,
            &snapshot.common_dir,
            Duration::from_millis(250),
            move |change| {
                let _ = sender.try_send(change);
            },
        );
        if let Err(error) = watcher.start() {
            log::warn!("couldn't watch {}: {error}", self.root);
            return;
        }
        self.watcher = Some(watcher);
        self._watching = Some(cx.spawn(async move |this, cx| {
            while receiver.recv().await.is_ok() {
                if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                    break;
                }
            }
        }));
    }

    /// Run a git operation off the main thread (one at a time per
    /// repository), with a safety snapshot first when `snapshot_reason` is
    /// given, then refresh. With `require_snapshot` the operation doesn't
    /// run at all when the snapshot can't be made: it throws work away, and
    /// the promised Undo would be missing. `label` shows in the Changes
    /// panel's activity bar, with git's progress lines under it. The task
    /// resolves after the refresh, so the snapshot is current.
    pub fn run<T: Send + 'static>(
        &mut self,
        label: Option<String>,
        snapshot_reason: Option<String>,
        require_snapshot: bool,
        operation: impl FnOnce(&str, &mut dyn FnMut(&str)) -> GitResult<T> + Send + 'static,
        cx: &mut Context<Self>,
    ) -> Task<(GitResult<T>, Option<SafetySnapshot>)> {
        let labelled = label.is_some();
        if let Some(label) = label {
            self.activity = Some(label);
            self.activity_detail = None;
            cx.notify();
        }
        let root = self.root.clone();
        let lock = self.lock.clone();
        let (lines, progress) = async_channel::unbounded::<String>();
        cx.spawn(async move |this, cx| {
            // git's progress lines, as they arrive.
            let listener = {
                let this = this.clone();
                cx.spawn(async move |cx| {
                    while let Ok(line) = progress.recv().await {
                        let _ = this.update(cx, |this, cx| {
                            if this.activity.is_some() {
                                this.activity_detail = Some(line);
                                cx.notify();
                            }
                        });
                    }
                })
            };
            let outcome = cx
                .background_spawn(async move {
                    let _turn = lock.lock().unwrap_or_else(|e| e.into_inner());
                    let mut safety_snapshot = None;
                    let mut failure = None;
                    if let Some(reason) = snapshot_reason {
                        match safety::create(&reason, &root, safety::OPLOG_PREFIX) {
                            Ok(created) => safety_snapshot = Some(created),
                            Err(error) => failure = Some(error),
                        }
                    }
                    let result = match failure {
                        Some(error) if require_snapshot => Err(OperationError::Invalid(format!(
                            "Nothing was changed: Impulse couldn't save a safety snapshot to undo \
                             it with. {}",
                            error.message()
                        ))),
                        _ => {
                            let mut report = |line: &str| {
                                let _ = lines.try_send(line.to_string());
                            };
                            operation(&root, &mut report)
                        }
                    };
                    drop(lines);
                    // Older than two weeks, or beyond the newest 200, go.
                    if safety_snapshot.is_some() {
                        safety::prune(&root, safety::OPLOG_PREFIX, 200, safety::DEFAULT_MAX_AGE);
                    }
                    (result, safety_snapshot)
                })
                .await;
            let _ = listener.await;
            let refreshed = this.update(cx, |this, cx| {
                if labelled {
                    this.activity = None;
                    this.activity_detail = None;
                }
                this.next_refresh(cx)
            });
            if let Ok(refreshed) = refreshed {
                let _ = refreshed.await;
            }
            outcome
        })
    }

    /// A git read Impulse makes on its own: in a folder that isn't trusted,
    /// with `cli::restricted_environment`, so it runs no program the
    /// repository's configuration names (and not at all when that
    /// environment can't be made). Blocking: call it off the main thread.
    pub fn read_git(root: &str, trusted: bool, arguments: &[&str]) -> Option<String> {
        let base = operations::environment();
        let environment = if trusted {
            base
        } else {
            cli::restricted_environment(root, &base)?
        };
        cli::run(
            arguments,
            root,
            RunOptions {
                environment: Some(&environment),
                timeout: Some(Duration::from_secs(30)),
                ..Default::default()
            },
        )
        .ok()
        .map(|output| output.stdout)
    }

    /// The status letter for `path`: its own, or a folder's (the
    /// highest-priority one inside it). Ignored files are "I".
    pub fn status_of(&self, path: &Path) -> Option<&str> {
        let parent = path.parent()?.to_string_lossy();
        let name = path.file_name()?.to_string_lossy();
        self.statuses
            .get(parent.as_ref())
            .and_then(|names| names.get(name.as_ref()))
            .map(String::as_str)
    }

    /// The titlebar's and status bar's branch: the branch, or the commit a
    /// detached HEAD is at.
    pub fn branch_label(&self) -> Option<String> {
        let snapshot = self.snapshot.as_ref()?;
        snapshot
            .branch
            .clone()
            .or_else(|| {
                snapshot
                    .head_oid
                    .as_ref()
                    .map(|h| h[..h.len().min(7)].to_string())
            })
            .filter(|b| !b.is_empty())
    }
}

/// One `RepositoryState` per repository root, app-wide (the windows hold
/// them; a state nothing holds any more goes, with its watcher).
impl RepositoryState {
    /// Fetch in the background, without progress or a failure toast (the
    /// Swift `fetchQuietly`): ahead/behind counts and History's incoming
    /// commits stay current.
    pub fn fetch_quietly(&mut self, cx: &mut Context<Self>) {
        let has_upstream = self.snapshot.as_ref().is_some_and(|s| s.upstream.is_some());
        if self.activity.is_some() || self.quiet_fetch_in_flight || !has_upstream {
            return;
        }
        self.quiet_fetch_in_flight = true;
        self.last_fetch = Some(std::time::Instant::now());
        let root = self.root.clone();
        let task = cx.background_spawn(async move {
            impulse_git::operations::fetch(false, None, &root, Duration::from_secs(120), None)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.quiet_fetch_in_flight = false;
                match result {
                    Ok(_) => this.refresh(cx),
                    Err(error) => log::info!("background fetch of {} failed: {error:?}", this.root),
                }
            });
        })
        .detach();
    }
}

/// Fetch every trusted repository open in a window every
/// `git_auto_fetch_minutes` (off at 0, the default), checking once a
/// minute (the Swift `AutoFetch`).
pub fn start_auto_fetch(cx: &mut App) {
    cx.spawn(async move |cx| loop {
        cx.background_executor()
            .timer(Duration::from_secs(60))
            .await;
        cx.update(|cx| {
            let minutes = crate::settings_store::SettingsStore::get(cx).git_auto_fetch_minutes;
            if minutes <= 0 || !cx.has_global::<RepositoryStore>() {
                return;
            }
            let interval = Duration::from_secs(minutes as u64 * 60);
            let states: Vec<_> = cx
                .global::<RepositoryStore>()
                .states
                .values()
                .filter_map(WeakEntity::upgrade)
                .collect();
            for state in states {
                state.update(cx, |state, cx| {
                    // Fetch follows the repository's own configuration
                    // (ssh commands, credential helpers): trusted only.
                    let due = state.last_fetch.is_none_or(|t| t.elapsed() >= interval);
                    if state.trusted && due {
                        state.fetch_quietly(cx);
                    }
                });
            }
        });
    })
    .detach();
}

#[derive(Default)]
pub struct RepositoryStore {
    states: HashMap<String, WeakEntity<RepositoryState>>,
    /// Folder to repository root (`None`: not in a repository).
    roots: HashMap<PathBuf, Option<String>>,
}

impl Global for RepositoryStore {}

impl RepositoryStore {
    /// The repository `dir` is in, if any (cached: libgit2's discovery is a
    /// few `stat`s, but render asks often).
    pub fn root_for(dir: &Path, cx: &mut App) -> Option<String> {
        if !cx.has_global::<RepositoryStore>() {
            cx.set_global(RepositoryStore::default());
        }
        let store = cx.global_mut::<RepositoryStore>();
        if let Some(root) = store.roots.get(dir) {
            return root.clone();
        }
        let root = impulse_git::repo_root(&dir.to_string_lossy())
            .map(|r| r.trim_end_matches('/').to_string());
        if store.roots.len() > 256 {
            store.roots.clear();
        }
        store.roots.insert(dir.to_path_buf(), root.clone());
        root
    }

    /// The state for a repository root, created on first use.
    pub fn state(root: &str, cx: &mut App) -> Entity<RepositoryState> {
        if !cx.has_global::<RepositoryStore>() {
            cx.set_global(RepositoryStore::default());
        }
        if let Some(existing) = cx
            .global::<RepositoryStore>()
            .states
            .get(root)
            .and_then(WeakEntity::upgrade)
        {
            return existing;
        }
        let root = root.to_string();
        let state = cx.new(|cx| RepositoryState::new(root.clone(), cx));
        let store = cx.global_mut::<RepositoryStore>();
        store.states.retain(|_, weak| weak.upgrade().is_some());
        store.states.insert(root, state.downgrade());
        state
    }
}
