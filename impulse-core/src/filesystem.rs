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
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let name = entry.file_name().to_string_lossy().to_string();

        if !show_hidden && name.starts_with('.') {
            continue;
        }

        let metadata = entry
            .metadata()
            .map_err(|e| format!("Failed to read metadata: {}", e))?;
        let file_type = entry
            .file_type()
            .map_err(|e| format!("Failed to get file type: {}", e))?;

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
            status_map.entry(dir_name).or_insert_with(|| "M".to_string());
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
        Ok(head
            .target()
            .map(|oid| { let s = oid.to_string(); s[..7.min(s.len())].to_string() }))
    }
}
