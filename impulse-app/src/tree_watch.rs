//! The file tree follows the disk (the Swift `FileTreeDataController`'s
//! watchers): every folder the tree has listed is watched (FSEvents on
//! macOS, inotify on Linux, each folder on its own so a large repository
//! doesn't take a watch per directory), and a folder whose entries change
//! (an agent's new file, a checkout, a `rm` in the terminal) is listed
//! again.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::{Context, Task, Window};
use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::settings_store::SettingsStore;
use crate::workbench::Workbench;

/// Changes come in bursts (a checkout, a build): wait for the rest.
const DEBOUNCE: Duration = Duration::from_millis(150);

/// An order-independent key for a set of folders (the sum of their
/// hashes, and how many), so a frame can tell nothing changed without
/// building the set.
pub(crate) fn paths_key<'a>(paths: impl Iterator<Item = &'a PathBuf>) -> (u64, usize) {
    use std::hash::{BuildHasher, BuildHasherDefault};
    let hasher = BuildHasherDefault::<std::collections::hash_map::DefaultHasher>::default();
    paths.fold((0u64, 0usize), |(sum, count), path| {
        (sum.wrapping_add(hasher.hash_one(path)), count + 1)
    })
}

pub(crate) struct TreeWatch {
    watcher: Option<RecommendedWatcher>,
    dirs: HashSet<PathBuf>,
    /// `paths_key` of the folders last asked for.
    key: (u64, usize),
    receiver: Option<async_channel::Receiver<PathBuf>>,
    _task: Option<Task<()>>,
}

impl TreeWatch {
    pub(crate) fn new() -> Self {
        let (sender, receiver) = async_channel::unbounded();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else {
                return;
            };
            // Only entries coming and going (and renames) change a listing.
            if !matches!(
                event.kind,
                notify::EventKind::Create(_)
                    | notify::EventKind::Remove(_)
                    | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
                    | notify::EventKind::Any
                    | notify::EventKind::Other
            ) {
                return;
            }
            for path in event.paths {
                let _ = sender.try_send(path);
            }
        })
        .map_err(|error| log::warn!("couldn't watch the file tree: {error}"))
        .ok();
        TreeWatch {
            watcher,
            dirs: HashSet::new(),
            key: (0, 0),
            receiver: Some(receiver),
            _task: None,
        }
    }
}

impl Workbench {
    /// Start listing folders again when their entries change.
    pub(crate) fn start_tree_watch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(receiver) = self.tree_watch.receiver.take() else {
            return;
        };
        let task = cx.spawn_in(window, async move |this, cx| {
            while let Ok(first) = receiver.recv().await {
                cx.background_executor().timer(DEBOUNCE).await;
                let mut changed = HashSet::from([first]);
                while let Ok(path) = receiver.try_recv() {
                    changed.insert(path);
                }
                let gone = this
                    .update(cx, |w, cx| w.tree_entries_changed(&changed, cx))
                    .is_err();
                if gone {
                    break;
                }
            }
        });
        self.tree_watch._task = Some(task);
    }

    fn tree_entries_changed(&mut self, changed: &HashSet<PathBuf>, cx: &mut Context<Self>) {
        let show_hidden = SettingsStore::get(cx).sidebar_show_hidden;
        let dirs: HashSet<PathBuf> = changed
            .iter()
            .filter_map(|p| p.parent().map(|d| d.to_path_buf()))
            .collect();
        let mut any = false;
        for workspace in &mut self.workspaces {
            // A folder that's gone takes its listing with it.
            workspace.children.retain(|dir, _| dir.is_dir());
            workspace.expanded_dirs.retain(|dir| dir.is_dir());
            for dir in &dirs {
                if workspace.children.contains_key(dir) {
                    let entries = crate::workspace::list_dir(dir, show_hidden);
                    workspace.children.insert(dir.clone(), entries);
                    any = true;
                }
            }
        }
        if any {
            cx.notify();
        }
    }

    /// Watch exactly the folders the tree has listed.
    pub(crate) fn sync_tree_watch(&mut self) {
        // Every frame: compare a key before building any set.
        let key = paths_key(self.workspaces.iter().flat_map(|w| w.children.keys()));
        if key == self.tree_watch.key && self.tree_watch.dirs.len() == key.1 {
            return;
        }
        self.tree_watch.key = key;
        let wanted: HashSet<PathBuf> = self
            .workspaces
            .iter()
            .flat_map(|w| w.children.keys().cloned())
            .collect();
        let watch = &mut self.tree_watch;
        if wanted == watch.dirs {
            return;
        }
        let Some(watcher) = watch.watcher.as_mut() else {
            return;
        };
        for gone in watch.dirs.difference(&wanted) {
            let _ = watcher.unwatch(gone);
        }
        let mut watching = HashSet::new();
        for dir in &wanted {
            if watch.dirs.contains(dir) || watcher.watch(dir, RecursiveMode::NonRecursive).is_ok() {
                watching.insert(dir.clone());
            }
        }
        watch.dirs = watching;
    }
}
