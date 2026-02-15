use ignore::WalkBuilder;
use serde::Serialize;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
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

        if is_likely_binary(path) {
            continue;
        }

        let file = match File::open(path) {
            Ok(f) => f,
            Err(_) => continue,
        };

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

            if let Some(col) = haystack.find(&query_match) {
                results.push(SearchResult {
                    path: file_path.clone(),
                    name: file_name.clone(),
                    line_number: Some((line_idx + 1) as u32),
                    line_content: Some(line.chars().take(500).collect()),
                    column_start: Some(col as u32),
                    column_end: Some((col + query_match.len()) as u32),
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
/// Returns the number of replacements made.
pub fn replace_in_file(
    path: &str,
    search: &str,
    replacement: &str,
    case_sensitive: bool,
) -> Result<usize, String> {
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

    let content =
        std::fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {}", path, e))?;

    let (new_content, count) = if case_sensitive {
        let count = content.matches(search).count();
        (content.replace(search, replacement), count)
    } else {
        let mut result = String::with_capacity(content.len());
        let mut count = 0usize;
        let search_lower = search.to_lowercase();
        let search_lower_len = search_lower.len();
        let mut byte_offset = 0usize;
        while byte_offset < content.len() {
            let remaining = &content[byte_offset..];
            let remaining_lower = remaining.to_lowercase();
            if let Some(lower_pos) = remaining_lower.find(&search_lower) {
                // Map position in lowercased string back to original by
                // walking characters in lockstep.
                let mut orig_pos = 0usize;
                let mut lower_walked = 0usize;
                let mut orig_chars = remaining.chars();
                let mut lower_chars = remaining_lower.chars();
                while lower_walked < lower_pos {
                    if let (Some(oc), Some(lc)) = (orig_chars.next(), lower_chars.next()) {
                        orig_pos += oc.len_utf8();
                        lower_walked += lc.len_utf8();
                    } else {
                        break;
                    }
                }
                // Find the end of the match in the original string
                let mut match_end = orig_pos;
                let mut match_lower_len = 0usize;
                let mut match_chars = remaining[orig_pos..].chars();
                let mut match_lower_chars = remaining_lower[lower_pos..].chars();
                while match_lower_len < search_lower_len {
                    if let (Some(oc), Some(lc)) = (match_chars.next(), match_lower_chars.next()) {
                        match_end += oc.len_utf8();
                        match_lower_len += lc.len_utf8();
                    } else {
                        break;
                    }
                }
                result.push_str(&remaining[..orig_pos]);
                result.push_str(replacement);
                byte_offset += match_end;
                count += 1;
            } else {
                result.push_str(remaining);
                break;
            }
        }
        (result, count)
    };

    if count > 0 {
        std::fs::write(path, new_content)
            .map_err(|e| format!("Failed to write {}: {}", path, e))?;
    }

    Ok(count)
}

/// Replace all occurrences of `search` with `replacement` across multiple files.
/// Returns the total number of replacements made.
pub fn replace_in_files(
    paths: &[String],
    search: &str,
    replacement: &str,
    case_sensitive: bool,
) -> Result<usize, String> {
    let mut total = 0;
    for path in paths {
        total += replace_in_file(path, search, replacement, case_sensitive)?;
    }
    Ok(total)
}

fn is_likely_binary(path: &Path) -> bool {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return true,
    };

    let mut buffer = [0u8; 8192];
    let bytes_read = match file.read(&mut buffer) {
        Ok(n) => n,
        Err(_) => return true,
    };

    buffer[..bytes_read].contains(&0)
}
