use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Clone, Debug)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: u64,
    pub git_status: Option<String>,
}

/// Read directory contents, sorted: directories first, then files, alphabetical within each group.
pub fn read_directory_entries(path: &str, show_hidden: bool) -> Result<Vec<FileEntry>, String> {
    let dir_path = PathBuf::from(path);
    if !dir_path.is_dir() {
        return Err(format!("Not a directory: {}", path));
    }

    let mut entries = Vec::new();
    let read_dir =
        fs::read_dir(&dir_path).map_err(|e| format!("Failed to read directory: {}", e))?;

    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                log::warn!("Skipping unreadable entry in '{}': {}", path, e);
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().to_string();

        if !show_hidden && name.starts_with('.') {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                log::warn!("Skipping entry '{}': failed to read metadata: {}", name, e);
                continue;
            }
        };
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(e) => {
                log::warn!("Skipping entry '{}': failed to get file type: {}", name, e);
                continue;
            }
        };

        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        entries.push(FileEntry {
            name,
            path: entry.path().to_string_lossy().to_string(),
            is_dir: metadata.is_dir(),
            is_symlink: file_type.is_symlink(),
            size: metadata.len(),
            modified,
            git_status: None,
        });
    }

    entries.sort_by_cached_key(|e| (!e.is_dir, e.name.to_lowercase()));

    Ok(entries)
}

/// Get git status for files in a directory using libgit2.
pub fn get_git_status_for_directory(path: &str) -> Result<HashMap<String, String>, String> {
    // Canonicalize path to resolve symlinks (e.g. /var -> /private/var on macOS)
    // so it matches the repo root from libgit2.
    let dir_path = fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
    let repo = match crate::git::open_repo(&dir_path) {
        Ok(r) => r,
        Err(_) => return Ok(HashMap::new()),
    };

    let repo_root = repo.workdir().ok_or("Bare repository")?.to_path_buf();

    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(false)
        .include_unmodified(false);

    // Restrict to the requested directory relative to the repo root.
    // When at the repo root (empty relative path), skip pathspec entirely
    // to list all statuses — pathspec("") matches nothing with some libgit2
    // configurations.
    if let Ok(rel) = dir_path.strip_prefix(&repo_root) {
        let spec = rel.to_string_lossy().to_string();
        if !spec.is_empty() {
            let spec = if spec.ends_with('/') {
                spec
            } else {
                format!("{}/", spec)
            };
            opts.pathspec(&spec);
        }
    }

    let statuses = repo
        .statuses(Some(&mut opts))
        .map_err(|e| format!("Failed to get git status: {}", e))?;

    let mut status_map = HashMap::new();

    for entry in statuses.iter() {
        let Some(rel_path) = entry.path() else {
            continue;
        };
        let abs_path = repo_root.join(rel_path);

        // Only include files directly within the requested directory, or
        // directories that are children of it (for recursive status markers).
        if !abs_path.starts_with(&dir_path) {
            continue;
        }

        let code = match status_to_code(entry.status()) {
            Some(c) => c,
            None => continue,
        };

        // Compute path relative to the requested directory so we can
        // distinguish direct children from files in subdirectories.
        let rel = match abs_path.strip_prefix(&dir_path) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let components: Vec<_> = rel.components().collect();
        if components.len() == 1 {
            // File is directly in this directory — use its exact status.
            if let Some(name) = abs_path.file_name() {
                status_map.insert(name.to_string_lossy().to_string(), code.to_string());
            }
        } else if components.len() > 1 {
            // File is in a subdirectory — mark the immediate child
            // directory with the highest-priority status among its descendants.
            let dir_name = components[0].as_os_str().to_string_lossy().to_string();
            status_map
                .entry(dir_name)
                .and_modify(|existing| {
                    if git_status_priority(code) > git_status_priority(existing) {
                        *existing = code.to_string();
                    }
                })
                .or_insert_with(|| code.to_string());
        }
    }

    Ok(status_map)
}

/// Read directory contents with git status enrichment.
pub fn read_directory_with_git_status(
    path: &str,
    show_hidden: bool,
) -> Result<Vec<FileEntry>, String> {
    let mut entries = read_directory_entries(path, show_hidden)?;

    if let Ok(git_status) = get_git_status_for_directory(path) {
        for entry in &mut entries {
            if let Some(status) = git_status.get(&entry.name) {
                entry.git_status = Some(status.clone());
            }
        }
    }

    Ok(entries)
}

/// Priority ranking for git status codes. Higher value = higher priority.
/// Used when propagating status from files to parent directories.
fn git_status_priority(code: &str) -> u8 {
    match code {
        "C" => 6, // conflict
        "D" => 5, // deleted
        "A" => 4, // added (staged)
        "?" => 3, // untracked
        "R" => 2, // renamed
        "M" => 1, // modified
        _ => 0,
    }
}

/// Convert a `git2::Status` bitflags value to a single-character status code.
fn status_to_code(s: git2::Status) -> Option<&'static str> {
    if s.intersects(git2::Status::CONFLICTED) {
        Some("C")
    } else if s.intersects(git2::Status::WT_NEW | git2::Status::INDEX_NEW) {
        if s.contains(git2::Status::INDEX_NEW) {
            Some("A")
        } else {
            Some("?")
        }
    } else if s.intersects(git2::Status::WT_DELETED | git2::Status::INDEX_DELETED) {
        Some("D")
    } else if s.intersects(git2::Status::INDEX_RENAMED | git2::Status::WT_RENAMED) {
        Some("R")
    } else if s.intersects(
        git2::Status::WT_MODIFIED
            | git2::Status::INDEX_MODIFIED
            | git2::Status::WT_TYPECHANGE
            | git2::Status::INDEX_TYPECHANGE,
    ) {
        Some("M")
    } else {
        None
    }
}

/// Batch-fetch git status for the entire repository at once.
///
/// Opens the repo once, calls `repo.statuses()` once with no pathspec filter,
/// and buckets results by parent directory. Returns a nested map:
/// outer key = directory absolute path, inner key = filename, value = status code.
///
/// Parent directories receive the highest-priority status among their descendants
/// (conflict > deleted > added > untracked > renamed > modified).
pub fn get_all_git_statuses(
    path: &str,
) -> Result<HashMap<String, HashMap<String, String>>, String> {
    let dir_path = fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
    let repo = match crate::git::open_repo(&dir_path) {
        Ok(r) => r,
        Err(_) => return Ok(HashMap::new()),
    };

    let repo_root = repo.workdir().ok_or("Bare repository")?.to_path_buf();

    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(false)
        .include_unmodified(false);

    let statuses = repo
        .statuses(Some(&mut opts))
        .map_err(|e| format!("Failed to get git status: {}", e))?;

    let mut result: HashMap<String, HashMap<String, String>> = HashMap::new();

    for entry in statuses.iter() {
        let Some(rel_path) = entry.path() else {
            continue;
        };
        let abs_path = repo_root.join(rel_path);

        let code = match status_to_code(entry.status()) {
            Some(c) => c,
            None => continue,
        };

        // Add file to its direct parent directory's map
        let Some(parent) = abs_path.parent() else {
            continue;
        };
        if let Some(file_name) = abs_path.file_name() {
            let file_name = file_name.to_string_lossy().to_string();
            let parent_str = parent.to_string_lossy().to_string();
            let dir_map = result.entry(parent_str).or_default();
            dir_map
                .entry(file_name)
                .and_modify(|existing| {
                    if git_status_priority(code) > git_status_priority(existing) {
                        *existing = code.to_string();
                    }
                })
                .or_insert_with(|| code.to_string());
        }

        // Propagate status to ancestor directories up to repo root
        let mut child = parent.to_path_buf();
        while let Some(ancestor) = child.parent() {
            if !child.starts_with(&repo_root) || child == repo_root {
                break;
            }
            let dir_name = child
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let ancestor_str = ancestor.to_string_lossy().to_string();

            let ancestor_map = result.entry(ancestor_str).or_default();
            ancestor_map
                .entry(dir_name)
                .and_modify(|existing| {
                    if git_status_priority(code) > git_status_priority(existing) {
                        *existing = code.to_string();
                    }
                })
                .or_insert_with(|| code.to_string());

            child = ancestor.to_path_buf();
        }
    }

    // Remap keys to use the caller's path prefix instead of repo_root.
    // This is necessary because repo.workdir() may return a canonicalized path
    // with different casing than what the caller used (macOS case-insensitive FS).
    let repo_root_str = repo_root.to_string_lossy();
    let repo_root_prefix = repo_root_str.trim_end_matches('/');
    let caller_prefix = path.trim_end_matches('/');

    if repo_root_prefix != caller_prefix {
        let remapped = result
            .into_iter()
            .map(|(key, val)| {
                if let Some(suffix) = key.strip_prefix(repo_root_prefix) {
                    (format!("{}{}", caller_prefix, suffix), val)
                } else {
                    (key, val)
                }
            })
            .collect();
        Ok(remapped)
    } else {
        Ok(result)
    }
}

/// Read directory contents with git status from a pre-computed batch status map.
///
/// This avoids redundant git work when refreshing multiple directories —
/// call `get_all_git_statuses()` once and pass the result here for each directory.
pub fn read_directory_with_git_status_batch(
    path: &str,
    show_hidden: bool,
    batch_statuses: &HashMap<String, HashMap<String, String>>,
) -> Result<Vec<FileEntry>, String> {
    let mut entries = read_directory_entries(path, show_hidden)?;

    if let Some(dir_statuses) = batch_statuses.get(path) {
        for entry in &mut entries {
            if let Some(status) = dir_statuses.get(&entry.name) {
                entry.git_status = Some(status.clone());
            }
        }
    }

    Ok(entries)
}

/// Get current git branch name for a path using libgit2.
///
/// Re-exported from `crate::git::get_git_branch` for backward compatibility.
pub fn get_git_branch(path: &str) -> Result<Option<String>, String> {
    crate::git::get_git_branch(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn non_git_directory_returns_empty_map() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("file.txt"), "hello").unwrap();
        let result = get_git_status_for_directory(dir.path().to_str().unwrap()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn clean_repo_returns_empty_map() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();

        // Create a file and commit it so the repo is clean
        let file_path = dir.path().join("tracked.txt");
        fs::write(&file_path, "content").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("tracked.txt")).unwrap();
        index.write().unwrap();
        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let sig = git2::Signature::now("test", "test@test.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();

        let result = get_git_status_for_directory(dir.path().to_str().unwrap()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn untracked_file_shows_question_mark() {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        fs::write(dir.path().join("new_file.txt"), "hello").unwrap();

        let result = get_git_status_for_directory(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.get("new_file.txt").map(String::as_str), Some("?"));
    }

    #[test]
    fn modified_tracked_file_shows_m() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();

        // Commit a file, then modify it
        let file_path = dir.path().join("tracked.txt");
        fs::write(&file_path, "original").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("tracked.txt")).unwrap();
        index.write().unwrap();
        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let sig = git2::Signature::now("test", "test@test.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();

        fs::write(&file_path, "modified").unwrap();

        let result = get_git_status_for_directory(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.get("tracked.txt").map(String::as_str), Some("M"));
    }

    #[test]
    fn staged_new_file_shows_a() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();

        // Create an initial commit so HEAD exists
        let init_path = dir.path().join(".gitkeep");
        fs::write(&init_path, "").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new(".gitkeep")).unwrap();
        index.write().unwrap();
        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let sig = git2::Signature::now("test", "test@test.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();

        // Stage a new file (INDEX_NEW)
        let file_path = dir.path().join("added.txt");
        fs::write(&file_path, "new content").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("added.txt")).unwrap();
        index.write().unwrap();

        let result = get_git_status_for_directory(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.get("added.txt").map(String::as_str), Some("A"));
    }

    #[test]
    fn deleted_file_shows_d() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();

        // Commit a file, then delete it
        let file_path = dir.path().join("doomed.txt");
        fs::write(&file_path, "content").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("doomed.txt")).unwrap();
        index.write().unwrap();
        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let sig = git2::Signature::now("test", "test@test.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();

        fs::remove_file(&file_path).unwrap();

        let result = get_git_status_for_directory(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.get("doomed.txt").map(String::as_str), Some("D"));
    }

    #[test]
    fn subdirectory_aggregation_marks_parent_as_modified() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();

        // Commit a file in a subdirectory, then modify it
        let sub_dir = dir.path().join("subdir");
        fs::create_dir(&sub_dir).unwrap();
        let file_path = sub_dir.join("nested.txt");
        fs::write(&file_path, "original").unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_path(std::path::Path::new("subdir/nested.txt"))
            .unwrap();
        index.write().unwrap();
        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let sig = git2::Signature::now("test", "test@test.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();

        fs::write(&file_path, "modified").unwrap();

        // Query the parent directory — the subdirectory should show "M"
        let result = get_git_status_for_directory(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.get("subdir").map(String::as_str), Some("M"));
        // The nested file itself should NOT appear (it's not a direct child)
        assert!(!result.contains_key("nested.txt"));
    }

    #[test]
    fn batch_statuses_buckets_by_directory() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();

        // Commit files in root and subdirectory, then modify them
        let sub_dir = dir.path().join("subdir");
        fs::create_dir(&sub_dir).unwrap();
        let root_file = dir.path().join("root.txt");
        let nested_file = sub_dir.join("nested.txt");
        fs::write(&root_file, "original").unwrap();
        fs::write(&nested_file, "original").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("root.txt")).unwrap();
        index
            .add_path(std::path::Path::new("subdir/nested.txt"))
            .unwrap();
        index.write().unwrap();
        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let sig = git2::Signature::now("test", "test@test.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();

        fs::write(&root_file, "modified").unwrap();
        fs::write(&nested_file, "modified").unwrap();

        let input_path = dir.path().to_str().unwrap();
        let result = get_all_git_statuses(input_path).unwrap();

        // Root directory should have root.txt=M and subdir=M (propagated).
        // Keys use the caller's path (not canonicalized) to match the caller's
        // path casing on case-insensitive filesystems.
        let root_key = input_path.trim_end_matches('/');
        let root_map = result
            .get(root_key)
            .expect("root directory should be in result");
        assert_eq!(root_map.get("root.txt").map(String::as_str), Some("M"));
        assert_eq!(root_map.get("subdir").map(String::as_str), Some("M"));

        // Subdirectory should have nested.txt=M
        let sub_key = format!("{}/subdir", root_key);
        let sub_map = result
            .get(&sub_key)
            .expect("subdirectory should be in result");
        assert_eq!(sub_map.get("nested.txt").map(String::as_str), Some("M"));
    }
}

