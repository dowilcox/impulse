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
            is_dir: file_type.is_dir(),
            is_symlink: file_type.is_symlink(),
            size: metadata.len(),
            modified,
            git_status: None,
        });
    }

    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    Ok(entries)
}

/// Get git status for files in a directory using libgit2.
pub fn get_git_status_for_directory(path: &str) -> Result<HashMap<String, String>, String> {
    let dir_path = PathBuf::from(path);
    let repo = match crate::git::open_repo(&dir_path) {
        Ok(r) => r,
        Err(_) => return Ok(HashMap::new()),
    };

    let repo_root = repo.workdir().ok_or("Bare repository")?.to_path_buf();

    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_unmodified(false);

    // Restrict to the requested directory relative to the repo root.
    if let Ok(rel) = dir_path.strip_prefix(&repo_root) {
        let mut spec = rel.to_string_lossy().to_string();
        if !spec.is_empty() && !spec.ends_with('/') {
            spec.push('/');
        }
        opts.pathspec(&spec);
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

        let s = entry.status();
        let code = if s.intersects(git2::Status::CONFLICTED) {
            "C"
        } else if s.intersects(git2::Status::WT_NEW | git2::Status::INDEX_NEW) {
            if s.contains(git2::Status::INDEX_NEW) {
                "A"
            } else {
                "?"
            }
        } else if s.intersects(git2::Status::WT_DELETED | git2::Status::INDEX_DELETED) {
            "D"
        } else if s.intersects(git2::Status::INDEX_RENAMED | git2::Status::WT_RENAMED) {
            "R"
        } else if s.intersects(
            git2::Status::WT_MODIFIED
                | git2::Status::INDEX_MODIFIED
                | git2::Status::WT_TYPECHANGE
                | git2::Status::INDEX_TYPECHANGE,
        ) {
            "M"
        } else {
            continue;
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
            // directory as modified so it shows as changed in the tree.
            let dir_name = components[0].as_os_str().to_string_lossy().to_string();
            status_map
                .entry(dir_name)
                .or_insert_with(|| "M".to_string());
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
}

/// Get current git branch name for a path using libgit2.
pub fn get_git_branch(path: &str) -> Result<Option<String>, String> {
    let repo = match crate::git::open_repo(std::path::Path::new(path)) {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };

    let head = match repo.head() {
        Ok(h) => h,
        Err(_) => return Ok(None),
    };

    if head.is_branch() {
        Ok(head.shorthand().map(String::from))
    } else {
        // Detached HEAD — return abbreviated commit hash
        Ok(head.target().map(|oid| {
            let s = oid.to_string();
            s[..7.min(s.len())].to_string()
        }))
    }
}
