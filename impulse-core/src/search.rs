use ignore::WalkBuilder;
use regex::RegexBuilder;
use serde::Serialize;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Serialize, Clone, Debug)]
pub struct SearchResult {
    pub path: String,
    pub name: String,
    pub line_number: Option<u32>,
    pub line_content: Option<String>,
    pub column_start: Option<u32>,
    pub column_end: Option<u32>,
    pub match_type: String,
}

/// Search for files by name pattern (substring matching, case-insensitive).
/// If `cancel` is provided and set to `true`, the search stops early and returns partial results.
pub fn search_filenames(
    root: &str,
    query: &str,
    limit: usize,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<SearchResult>, String> {
    let query_lower = query.to_lowercase();
    let mut results = Vec::new();

    let walker = WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .max_depth(Some(15))
        .same_file_system(true)
        .build();

    for entry in walker {
        if results.len() >= limit {
            break;
        }

        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            break;
        }

        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(true) {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();

        if name.to_lowercase().contains(&query_lower) {
            results.push(SearchResult {
                path: entry.path().to_string_lossy().to_string(),
                name,
                line_number: None,
                line_content: None,
                column_start: None,
                column_end: None,
                match_type: "file".to_string(),
            });
        }
    }

    Ok(results)
}

/// Check the first 8KB of an already-opened file for null bytes (binary indicator).
/// Returns `Ok(true)` if the file appears to be binary, `Ok(false)` if it appears to be text.
/// Returns `Err` on permission or I/O errors so the caller can decide how to handle them.
/// On success, the file handle is seeked back to the beginning for subsequent reading.
fn check_binary_and_rewind(file: &mut File) -> Result<bool, std::io::Error> {
    let mut buffer = [0u8; 8192];
    let bytes_read = file.read(&mut buffer)?;
    let is_binary = buffer[..bytes_read].contains(&0);
    file.seek(SeekFrom::Start(0))?;
    Ok(is_binary)
}

/// Search file contents for a text pattern.
/// If `cancel` is provided and set to `true`, the search stops early and returns partial results.
pub fn search_contents(
    root: &str,
    query: &str,
    limit: usize,
    case_sensitive: bool,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<SearchResult>, String> {
    let query_match = if case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };

    let mut results = Vec::new();

    let walker = WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .max_depth(Some(15))
        .same_file_system(true)
        .build();

    for entry in walker {
        if results.len() >= limit {
            break;
        }

        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            break;
        }

        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        if entry.file_type().map(|ft| !ft.is_file()).unwrap_or(true) {
            continue;
        }

        let path = entry.path();

        if entry
            .metadata()
            .map(|m| m.len() > 1_048_576)
            .unwrap_or(false)
        {
            continue;
        }

        // Open the file once: check for binary content, then reuse the handle for reading lines.
        let mut file = match File::open(path) {
            Ok(f) => f,
            Err(e) => {
                log::warn!("Failed to open '{}': {}", path.display(), e);
                continue;
            }
        };

        match check_binary_and_rewind(&mut file) {
            Ok(true) => continue,  // binary file, skip
            Ok(false) => {}        // text file, proceed
            Err(e) => {
                log::warn!("Failed to read '{}': {}", path.display(), e);
                continue;
            }
        }

        let reader = BufReader::new(file);
        let file_name = entry.file_name().to_string_lossy().to_string();
        let file_path = path.to_string_lossy().to_string();

        for (line_idx, line) in reader.lines().enumerate() {
            if results.len() >= limit {
                break;
            }

            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };

            let haystack = if case_sensitive {
                line.clone()
            } else {
                line.to_lowercase()
            };

            // Find all matches on this line, not just the first.
            // Use character-based column positions so that non-ASCII text
            // (and case-insensitive lowercasing that changes byte lengths)
            // reports correct columns.
            let line_content: Option<String> = None;
            for (byte_pos, _) in haystack.match_indices(&query_match) {
                if results.len() >= limit {
                    break;
                }

                // Convert byte offset in the haystack to a character offset.
                let col_start_chars = haystack[..byte_pos].chars().count();

                // For column_end, we need the character length of the match
                // in the *original* line (not the lowercased haystack), because
                // lowercasing can change character count for certain Unicode.
                // The match length in characters from the original line is the
                // same span of characters starting at col_start_chars.
                let match_char_len = query.chars().count();
                let col_end_chars = col_start_chars + match_char_len;

                results.push(SearchResult {
                    path: file_path.clone(),
                    name: file_name.clone(),
                    line_number: Some((line_idx + 1) as u32),
                    line_content: line_content
                        .clone()
                        .or_else(|| Some(line.chars().take(500).collect())),
                    column_start: Some(col_start_chars as u32),
                    column_end: Some(col_end_chars as u32),
                    match_type: "content".to_string(),
                });
            }
        }
    }

    Ok(results)
}

/// Search files by name, content, or both.
/// If `cancel` is provided and set to `true`, the search stops early and returns partial results.
pub fn search(
    root: &str,
    query: &str,
    search_type: &str,
    case_sensitive: bool,
    limit: usize,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<SearchResult>, String> {
    if query.is_empty() {
        return Ok(Vec::new());
    }

    match search_type {
        "filename" => search_filenames(root, query, limit, cancel),
        "content" => search_contents(root, query, limit, case_sensitive, cancel),
        "both" => {
            let mut results = search_filenames(root, query, limit, cancel)?;
            let remaining = limit.saturating_sub(results.len());
            if remaining > 0 {
                let content_results =
                    search_contents(root, query, remaining, case_sensitive, cancel)?;
                results.extend(content_results);
            }
            Ok(results)
        }
        _ => Err(format!("Unknown search type: {}", search_type)),
    }
}

/// Replace all occurrences of `search` with `replacement` in a single file.
/// Uses atomic file replacement: writes to a temporary file then renames over
/// the original to prevent data loss on crash. Preserves file permissions.
/// Returns the number of replacements made.
pub fn replace_in_file(
    path: &str,
    search: &str,
    replacement: &str,
    case_sensitive: bool,
) -> Result<usize, String> {
    use std::os::unix::fs::PermissionsExt;

    // Check file size before reading
    let metadata = std::fs::metadata(path)
        .map_err(|e| format!("Failed to read metadata for '{}': {}", path, e))?;
    if metadata.len() > 1_048_576 {
        return Err(format!(
            "File '{}' is too large for replacement ({} bytes, max 1MB)",
            path,
            metadata.len()
        ));
    }

    let permissions = metadata.permissions();

    let content =
        std::fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {}", path, e))?;

    let (new_content, count) = if case_sensitive {
        let count = content.matches(search).count();
        (content.replace(search, replacement), count)
    } else {
        let re = RegexBuilder::new(&regex::escape(search))
            .case_insensitive(true)
            .build()
            .map_err(|e| format!("Invalid search pattern: {}", e))?;
        let count = re.find_iter(&content).count();
        (re.replace_all(&content, replacement).into_owned(), count)
    };

    if count > 0 {
        // Write to a temporary file in the same directory, then atomically rename.
        // This ensures the original file is not corrupted if we crash mid-write.
        let original_path = Path::new(path);
        let parent = original_path
            .parent()
            .ok_or_else(|| format!("Cannot determine parent directory of '{}'", path))?;
        let tmp_path = parent.join(format!(
            ".{}.impulse-tmp",
            original_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "file".to_string())
        ));

        {
            let mut tmp_file = File::create(&tmp_path)
                .map_err(|e| format!("Failed to create temp file '{}': {}", tmp_path.display(), e))?;
            tmp_file.write_all(new_content.as_bytes())
                .map_err(|e| {
                    // Clean up temp file on write failure
                    let _ = std::fs::remove_file(&tmp_path);
                    format!("Failed to write temp file '{}': {}", tmp_path.display(), e)
                })?;
            tmp_file.sync_all()
                .map_err(|e| {
                    let _ = std::fs::remove_file(&tmp_path);
                    format!("Failed to sync temp file '{}': {}", tmp_path.display(), e)
                })?;
        }

        // Preserve original file permissions
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(permissions.mode()))
            .map_err(|e| {
                let _ = std::fs::remove_file(&tmp_path);
                format!("Failed to set permissions on temp file: {}", e)
            })?;

        // Atomic rename
        std::fs::rename(&tmp_path, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp_path);
            format!("Failed to rename temp file to '{}': {}", path, e)
        })?;
    }

    Ok(count)
}

/// Replace all occurrences of `search` with `replacement` across multiple files.
/// Each file path is validated to be within `root` before modification.
/// Returns a list of (path, result) pairs so callers can see per-file outcomes.
pub fn replace_in_files(
    paths: &[String],
    search: &str,
    replacement: &str,
    case_sensitive: bool,
    root: &str,
) -> Vec<(String, Result<usize, String>)> {
    paths
        .iter()
        .map(|path| {
            let result = match crate::util::validate_path_within_root(path, root) {
                Ok(_) => replace_in_file(path, search, replacement, case_sensitive),
                Err(e) => Err(e),
            };
            (path.clone(), result)
        })
        .collect()
}

