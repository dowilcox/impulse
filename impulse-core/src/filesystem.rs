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

/// Get git status for files in a directory.
pub fn get_git_status_for_directory(path: &str) -> Result<HashMap<String, String>, String> {
    use std::process::Command;

    let git_root = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(path)
        .output()
        .map_err(|e| format!("Failed to run git: {}", e))?;

    if !git_root.status.success() {
        return Ok(HashMap::new());
    }

    let output = Command::new("git")
        .args(["status", "--porcelain", "-uall"])
        .current_dir(path)
        .output()
        .map_err(|e| format!("Failed to run git status: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut status_map = HashMap::new();

    let git_root_path = PathBuf::from(String::from_utf8_lossy(&git_root.stdout).trim());
    let dir_path = PathBuf::from(path);

    for line in stdout.lines() {
        if line.len() < 4 {
            continue;
        }
        let status_chars = &line[..2];
        let file_path = line[3..].trim();

        let abs_path = git_root_path.join(file_path);

        if abs_path.starts_with(&dir_path)
            || abs_path.parent().map(|p| p == dir_path).unwrap_or(false)
        {
            let status = match status_chars.trim() {
                "M" | " M" | "MM" => "M",
                "A" | "AM" => "A",
                "D" | " D" => "D",
                "R" => "R",
                "??" => "?",
                "UU" | "AA" | "DD" => "C",
                _ => "M",
            };

            if let Some(name) = abs_path.file_name() {
                status_map.insert(name.to_string_lossy().to_string(), status.to_string());
            }
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

/// Get current git branch name for a path.
pub fn get_git_branch(path: &str) -> Result<Option<String>, String> {
    use std::process::Command;

    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(path)
        .output()
        .map_err(|e| format!("Failed to run git: {}", e))?;

    if output.status.success() {
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(Some(branch))
    } else {
        Ok(None)
    }
}
