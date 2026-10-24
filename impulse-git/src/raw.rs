//! libgit2 plumbing shared by the read modules: opening the repository that
//! holds a path (with the root cached by directory), reusing handles that
//! short reads left idle, and path helpers with Rust `Path` semantics, as the
//! original Rust had them.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use git2::{ObjectType, Repository};

use crate::repo_cache::REPO_CACHE;
use crate::GitError;

/// Open the repository containing `path`, from the cached root when there is
/// one, else discovering it (and caching the root).
pub(crate) fn open_repo(path: &str) -> Result<Repository, GitError> {
    open_repo_from(path, &lookup_directory(path))
}

/// The directory a path's repository root is cached by: a file's parent
/// (most lookups are for files).
fn lookup_directory(path: &str) -> String {
    if Path::new(path).is_file() {
        if let Some(parent) = Path::new(path).parent() {
            return parent.to_string_lossy().into_owned();
        }
    }
    path.to_string()
}

fn open_repo_from(path: &str, lookup_dir: &str) -> Result<Repository, GitError> {
    if let Some(root) = REPO_CACHE.root(lookup_dir) {
        if let Ok(repo) = Repository::open(&root) {
            return Ok(repo);
        }
    }
    // Root no longer valid (or never cached): discover and cache.
    let repo = Repository::discover(path)
        .map_err(|e| GitError(format!("Not a git repo: {}", e.message())))?;
    let Some(root) = workdir_raw(&repo) else {
        return Err(GitError::new("Bare repository"));
    };
    REPO_CACHE.store(&root, lookup_dir);
    Ok(repo)
}

/// Run `body` on the repository containing `path`, reusing the handle an
/// earlier short read left idle. For quick reads of refs and commits only:
/// nothing libgit2 hands out from the repository may outlive `body`.
pub(crate) fn with_repo<R>(path: &str, body: impl FnOnce(&Repository) -> R) -> Result<R, GitError> {
    let lookup_dir = lookup_directory(path);
    let reused = REPO_CACHE
        .root(&lookup_dir)
        .and_then(|root| HANDLES.take(&root));
    let repo = match reused {
        Some(repo) => repo,
        None => open_repo_from(path, &lookup_dir)?,
    };
    let result = body(&repo);
    HANDLES.put(repo);
    Ok(result)
}

/// How many repositories short reads have left open; for tests.
#[doc(hidden)]
pub fn idle_handle_count() -> usize {
    HANDLES.idle.lock().unwrap().len()
}

/// Repositories opened for short reads, kept idle for a few seconds so a
/// burst of them (a revision per task, a lookup per branch) opens each
/// repository once instead of re-reading its config every time. A handle
/// serves one caller at a time; one whose folders are gone, or that has been
/// idle a while, is closed instead (an open repository holds on to pack
/// files and some config from when it opened).
struct RepoHandles {
    /// By working directory, as libgit2 reports it (the `RepoCache` roots).
    idle: Mutex<HashMap<String, (Repository, Instant)>>,
    close_scheduled: Mutex<bool>,
}

const HANDLE_CAPACITY: usize = 8;
const HANDLE_LIFETIME: Duration = Duration::from_secs(5);

static HANDLES: LazyLock<RepoHandles> = LazyLock::new(|| RepoHandles {
    idle: Mutex::new(HashMap::new()),
    close_scheduled: Mutex::new(false),
});

impl RepoHandles {
    fn take(&self, root: &str) -> Option<Repository> {
        let (repo, since) = self.idle.lock().unwrap().remove(root)?;
        (since.elapsed() < HANDLE_LIFETIME && Path::new(root).exists() && repo.path().exists())
            .then_some(repo)
    }

    fn put(&self, repo: Repository) {
        let Some(root) = workdir_raw(&repo) else {
            return;
        };
        {
            let mut idle = self.idle.lock().unwrap();
            idle.insert(root, (repo, Instant::now()));
            if idle.len() > HANDLE_CAPACITY {
                if let Some(oldest) = idle
                    .iter()
                    .min_by_key(|(_, (_, since))| *since)
                    .map(|(k, _)| k.clone())
                {
                    idle.remove(&oldest);
                }
            }
        }
        self.schedule_close();
    }

    /// Close what's been idle for the handle lifetime, until nothing is left.
    fn schedule_close(&self) {
        let mut scheduled = self.close_scheduled.lock().unwrap();
        if *scheduled {
            return;
        }
        *scheduled = true;
        std::thread::spawn(|| loop {
            std::thread::sleep(HANDLE_LIFETIME);
            let handles = &*HANDLES;
            let mut scheduled = handles.close_scheduled.lock().unwrap();
            let mut idle = handles.idle.lock().unwrap();
            idle.retain(|_, (_, since)| since.elapsed() < HANDLE_LIFETIME);
            if idle.is_empty() {
                *scheduled = false;
                return;
            }
        });
    }
}

/// The working directory exactly as libgit2 reports it (trailing slash), or
/// `None` for a bare repository.
pub(crate) fn workdir_raw(repo: &Repository) -> Option<String> {
    repo.workdir().map(|p| p.to_string_lossy().into_owned())
}

/// The working directory without a trailing slash; "Bare repository" for a
/// bare one, as the original Rust said.
pub(crate) fn workdir(repo: &Repository) -> Result<String, GitError> {
    workdir_raw(repo)
        .map(|raw| trim_trailing_slash(&raw))
        .ok_or_else(|| GitError::new("Bare repository"))
}

/// The git directory (no trailing slash).
pub(crate) fn git_dir(repo: &Repository) -> String {
    trim_trailing_slash(&repo.path().to_string_lossy())
}

/// HEAD peeled to a tree, or `None` (unborn branch, error).
pub(crate) fn head_tree(repo: &Repository) -> Option<git2::Tree<'_>> {
    repo.head()
        .ok()?
        .peel(ObjectType::Tree)
        .ok()?
        .into_tree()
        .ok()
}

/// A path as libgit2 gave it, decoded the way Swift's `String(cString:)`
/// does (invalid UTF-8 replaced).
pub(crate) fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A name for a temporary file or folder that no other process or call
/// picks.
pub(crate) fn unique_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{:x}-{:x}-{:x}",
        std::process::id(),
        nanos,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// A unix time, as git prints it, as a `SystemTime`.
pub(crate) fn unix_time(seconds: f64) -> SystemTime {
    if seconds.is_finite() && seconds > 0.0 {
        UNIX_EPOCH + Duration::from_secs_f64(seconds)
    } else {
        UNIX_EPOCH
    }
}

// MARK: Path helpers (component-wise, with Rust `Path` semantics)

/// Remove trailing slashes (but keep a lone "/").
pub(crate) fn trim_trailing_slash(path: &str) -> String {
    let mut out = path;
    while out.len() > 1 && out.ends_with('/') {
        out = &out[..out.len() - 1];
    }
    out.to_string()
}

/// Symlinks resolved and the path normalized (realpath); `None` when it
/// doesn't exist.
pub(crate) fn canonical_path(path: &str) -> Option<String> {
    std::fs::canonicalize(path)
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Lexical normalization (collapse `//` and `.`), as
/// `PathBuf::from(path).components().collect::<PathBuf>()` does.
pub(crate) fn lexically_normalized(path: &str) -> String {
    let parts: Vec<&str> = path
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    let joined = parts.join("/");
    if path.starts_with('/') {
        format!("/{joined}")
    } else if joined.is_empty() {
        path.to_string()
    } else {
        joined
    }
}

pub(crate) fn path_components(path: &str) -> Vec<&str> {
    path.split('/').filter(|p| !p.is_empty()).collect()
}

/// Component-wise prefix check, like `Path::starts_with`.
pub(crate) fn has_path_prefix(path: &str, prefix: &str) -> bool {
    let path = path_components(path);
    let prefix = path_components(prefix);
    path.len() >= prefix.len() && path[..prefix.len()] == prefix[..]
}

/// Component-wise `Path::strip_prefix`: the components of `path` under
/// `root`, or `None` when it isn't inside `root`.
pub(crate) fn relative_path_components<'a>(path: &'a str, root: &str) -> Option<Vec<&'a str>> {
    let path_parts = path_components(path);
    let root_parts = path_components(root);
    (path_parts.len() >= root_parts.len() && path_parts[..root_parts.len()] == root_parts[..])
        .then(|| path_parts[root_parts.len()..].to_vec())
}

pub(crate) fn join_path(root: &str, rel: &str) -> String {
    if rel.is_empty() {
        return root.to_string();
    }
    format!("{}/{rel}", trim_trailing_slash(root))
}

/// Size of the file at `path`, following symlinks as `fs::metadata` does.
pub(crate) fn file_size(path: &str) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.len())
}
