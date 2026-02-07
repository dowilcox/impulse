use ignore::WalkBuilder;
use serde::Serialize;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

#[derive(Serialize, Clone, Debug)]
pub struct SearchResult {
    pub path: String,
    pub name: String,
    pub line_number: Option<u32>,
    pub line_content: Option<String>,
    pub match_type: String,
}

/// Search for files by name pattern (substring matching, case-insensitive).
pub fn search_filenames(
    root: &str,
    query: &str,
    limit: usize,
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
                match_type: "file".to_string(),
            });
        }
    }

    Ok(results)
}

/// Search file contents for a text pattern.
pub fn search_contents(
    root: &str,
    query: &str,
    limit: usize,
    case_sensitive: bool,
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

            let matches = if case_sensitive {
                line.contains(&query_match)
            } else {
                line.to_lowercase().contains(&query_match)
            };

            if matches {
                results.push(SearchResult {
                    path: file_path.clone(),
                    name: file_name.clone(),
                    line_number: Some((line_idx + 1) as u32),
                    line_content: Some(line.chars().take(500).collect()),
                    match_type: "content".to_string(),
                });
            }
        }
    }

    Ok(results)
}

/// Search files by name, content, or both.
pub fn search(
    root: &str,
    query: &str,
    search_type: &str,
    case_sensitive: bool,
    limit: usize,
) -> Result<Vec<SearchResult>, String> {
    if query.is_empty() {
        return Ok(Vec::new());
    }

    match search_type {
        "filename" => search_filenames(root, query, limit),
        "content" => search_contents(root, query, limit, case_sensitive),
        "both" => {
            let mut results = search_filenames(root, query, limit)?;
            let remaining = limit.saturating_sub(results.len());
            if remaining > 0 {
                let content_results = search_contents(root, query, remaining, case_sensitive)?;
                results.extend(content_results);
            }
            Ok(results)
        }
        _ => Err(format!("Unknown search type: {}", search_type)),
    }
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
