//! Editors follow their files on disk (the Swift `EditorTab` file
//! watcher): the folders of open files are watched (FSEvents on macOS,
//! inotify on Linux), and when one of their files changes, by an agent,
//! git or another app, a clean editor shows it and one with unsaved edits
//! says so. Folders rather than files, so an atomic write (a new file
//! renamed over the old) is seen too.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::{Context, Task, Window};
use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::workbench::Workbench;
use crate::workspace::Surface;

/// Writes come in bursts (a formatter, a checkout): wait this long for the
/// rest before reloading.
const DEBOUNCE: Duration = Duration::from_millis(100);

pub(crate) struct EditorWatch {
    watcher: Option<RecommendedWatcher>,
    dirs: HashSet<PathBuf>,
    /// `tree_watch::paths_key` of the open files' paths last seen.
    key: (u64, usize),
    failed: bool,
    receiver: Option<async_channel::Receiver<PathBuf>>,
    _task: Option<Task<()>>,
}

impl EditorWatch {
    pub(crate) fn new() -> Self {
        let (sender, receiver) = async_channel::unbounded();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else {
                return;
            };
            if matches!(event.kind, notify::EventKind::Access(_)) {
                return;
            }
            for path in event.paths {
                let _ = sender.try_send(path);
            }
        })
        .map_err(|error| log::warn!("couldn't watch open files: {error}"))
        .ok();
        EditorWatch {
            watcher,
            dirs: HashSet::new(),
            key: (0, 0),
            failed: false,
            receiver: Some(receiver),
            _task: None,
        }
    }
}

impl EditorWatch {
    /// Every wanted folder is watched (none waits to be tried again).
    fn retry_done(&self) -> bool {
        !self.failed
    }
}

impl Workbench {
    /// Start handing file changes to the editors that show them.
    pub(crate) fn start_editor_watch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(receiver) = self.editor_watch.receiver.take() else {
            return;
        };
        let task = cx.spawn_in(window, async move |this, cx| {
            while let Ok(first) = receiver.recv().await {
                cx.background_executor().timer(DEBOUNCE).await;
                let mut changed = HashSet::from([first]);
                while let Ok(path) = receiver.try_recv() {
                    changed.insert(path);
                }
                let changed: HashSet<PathBuf> = changed
                    .into_iter()
                    .flat_map(|p| {
                        let canonical = p.canonicalize().ok();
                        std::iter::once(p).chain(canonical)
                    })
                    .collect();
                let gone = this
                    .update_in(cx, |w, window, cx| w.files_changed(&changed, window, cx))
                    .is_err();
                if gone {
                    break;
                }
            }
        });
        self.editor_watch._task = Some(task);
    }

    fn files_changed(
        &mut self,
        changed: &HashSet<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editors: Vec<_> = self
            .workspaces
            .iter()
            .flat_map(|w| w.tabs.iter())
            .flat_map(|t| t.panes.values())
            .filter_map(Surface::editor)
            .cloned()
            .collect();
        for editor in editors {
            let path = editor.read(cx).path.clone();
            let canonical = path.canonicalize().ok();
            if changed.contains(&path) || canonical.is_some_and(|c| changed.contains(&c)) {
                editor.update(cx, |e, cx| e.disk_changed(window, cx));
            }
        }
    }

    /// Watch exactly the folders of the files open in editors.
    pub(crate) fn sync_editor_watch(&mut self, cx: &Context<Self>) {
        // Every frame: compare the open files' key before building a set.
        let paths: Vec<&PathBuf> = self
            .workspaces
            .iter()
            .flat_map(|w| w.tabs.iter())
            .flat_map(|t| t.panes.values())
            .filter_map(Surface::editor)
            .map(|e| &e.read(cx).path)
            .collect();
        let key = crate::tree_watch::paths_key(paths.into_iter());
        if key == self.editor_watch.key && self.editor_watch.retry_done() {
            return;
        }
        self.editor_watch.key = key;
        let wanted: HashSet<PathBuf> = self
            .workspaces
            .iter()
            .flat_map(|w| w.tabs.iter())
            .flat_map(|t| t.panes.values())
            .filter_map(Surface::editor)
            .filter_map(|e| e.read(cx).path.parent().map(|p| p.to_path_buf()))
            .filter(|p| !p.as_os_str().is_empty())
            .collect();
        let watch = &mut self.editor_watch;
        if wanted == watch.dirs {
            watch.failed = false;
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
        // Folders that couldn't be watched are tried again next time.
        watch.failed = watching.len() < wanted.len();
        watch.dirs = watching;
    }
}
