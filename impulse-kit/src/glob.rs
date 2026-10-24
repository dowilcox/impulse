use std::path::Path;

/// Whether `path` matches a settings file-pattern override.
///
/// Deliberately NOT a full glob implementation: it supports exactly what the
/// settings file-pattern overrides support: `"*"` (match all), `"*.ext"`
/// (case-insensitive extension match), and exact filename matching.
pub fn matches_file_pattern(path: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(ext_pattern) = pattern.strip_prefix("*.") {
        if let Some(ext) = Path::new(path).extension() {
            return ext.to_string_lossy().eq_ignore_ascii_case(ext_pattern);
        }
        return false;
    }
    // Exact filename match (e.g. "Makefile")
    let filename = Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    filename == pattern
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard() {
        assert!(matches_file_pattern("/any/path.rs", "*"));
        assert!(matches_file_pattern("", "*"));
    }

    #[test]
    fn extension() {
        assert!(matches_file_pattern("/src/main.rs", "*.rs"));
        assert!(matches_file_pattern("/src/main.RS", "*.rs")); // case-insensitive
        assert!(!matches_file_pattern("/src/main.py", "*.rs"));
    }

    #[test]
    fn exact_name() {
        assert!(matches_file_pattern("/src/Makefile", "Makefile"));
        assert!(!matches_file_pattern("/src/makefile", "Makefile"));
    }

    #[test]
    fn no_extension_no_match() {
        assert!(!matches_file_pattern("/src/Makefile", "*.rs"));
    }
}
