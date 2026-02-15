use std::path::Path;

use url::Url;

use crate::pty::url_decode;

/// Convert a local path to a `file://` URI.
pub fn file_path_to_uri(path: &Path) -> Option<String> {
    if path.is_dir() {
        Url::from_directory_path(path).ok().map(|u| u.to_string())
    } else {
        Url::from_file_path(path).ok().map(|u| u.to_string())
    }
}

/// Convert a `file://` URI to a local file path string.
pub fn uri_to_file_path(uri: &str) -> String {
    if let Ok(parsed) = Url::parse(uri) {
        if parsed.scheme() == "file" {
            if let Ok(path) = parsed.to_file_path() {
                return path.to_string_lossy().to_string();
            }

            // Host-form file URIs (e.g. file://hostname/path) may fail
            // to_file_path() on some platforms; fall back to URI path.
            let decoded = url_decode(parsed.path());
            if !decoded.is_empty() {
                return decoded;
            }
        }
    }

    // Fallback for non-standard file URI strings.
    if let Some(rest) = uri.strip_prefix("file://") {
        if let Some(slash_idx) = rest.find('/') {
            return url_decode(&rest[slash_idx..]);
        }
        return url_decode(rest);
    }

    uri.to_string()
}

/// Determine LSP language ID from a file URI based on extension.
pub fn language_from_uri(uri: &str) -> String {
    let path = uri_to_file_path(uri);
    let path_obj = Path::new(&path);
    if let Some(name) = path_obj.file_name().and_then(|n| n.to_str()) {
        if name.eq_ignore_ascii_case("dockerfile") {
            return "dockerfile".to_string();
        }
    }
    let ext = path_obj
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "rs" => "rust".to_string(),
        "py" | "pyi" => "python".to_string(),
        "js" | "mjs" | "cjs" => "javascript".to_string(),
        "jsx" => "javascriptreact".to_string(),
        "ts" => "typescript".to_string(),
        "tsx" => "typescriptreact".to_string(),
        "c" | "h" => "c".to_string(),
        "cpp" | "cxx" | "cc" | "hpp" | "hxx" => "cpp".to_string(),
        "html" | "htm" => "html".to_string(),
        "css" => "css".to_string(),
        "scss" => "scss".to_string(),
        "less" => "less".to_string(),
        "json" => "json".to_string(),
        "jsonc" => "jsonc".to_string(),
        "yaml" | "yml" => "yaml".to_string(),
        "vue" => "vue".to_string(),
        "svelte" => "svelte".to_string(),
        "graphql" | "gql" => "graphql".to_string(),
        "sh" | "bash" | "zsh" | "fish" => "shellscript".to_string(),
        "dockerfile" => "dockerfile".to_string(),
        "go" => "go".to_string(),
        "java" => "java".to_string(),
        "rb" => "ruby".to_string(),
        "lua" => "lua".to_string(),
        "zig" => "zig".to_string(),
        "php" => "php".to_string(),
        _ => ext,
    }
}

/// Check whether a file path matches a glob-like pattern.
///
/// Supports `"*"` (match all), `"*.ext"` (extension match), and exact
/// filename suffix matching.
#[must_use]
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
    filename == pattern || path.ends_with(pattern)
}

/// Validates that `path` is within `root` after canonicalization.
/// Returns the canonicalized path on success, or an error if path escapes root.
pub fn validate_path_within_root(path: &str, root: &str) -> Result<std::path::PathBuf, String> {
    let canonical_root = std::fs::canonicalize(root)
        .map_err(|e| format!("Failed to canonicalize root '{}': {}", root, e))?;
    let canonical_path = std::fs::canonicalize(path)
        .map_err(|e| format!("Failed to canonicalize path '{}': {}", path, e))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(format!(
            "Path '{}' is outside the workspace root '{}'",
            path, root
        ));
    }
    Ok(canonical_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_to_file_path_basic() {
        assert_eq!(
            uri_to_file_path("file:///home/user/project/main.rs"),
            "/home/user/project/main.rs"
        );
    }

    #[test]
    fn uri_to_file_path_percent_encoded() {
        assert_eq!(
            uri_to_file_path("file:///home/user/my%20project/main.rs"),
            "/home/user/my project/main.rs"
        );
    }

    #[test]
    fn uri_to_file_path_non_file_uri_returns_as_is() {
        assert_eq!(
            uri_to_file_path("https://example.com"),
            "https://example.com"
        );
    }

    #[test]
    fn uri_to_file_path_bare_string_returned_as_is() {
        assert_eq!(uri_to_file_path("/tmp/foo.txt"), "/tmp/foo.txt");
    }

    #[test]
    fn file_path_to_uri_regular_file() {
        // Use a path that we know exists to avoid is_dir() returning false
        // for a non-existent path. /tmp always exists.
        let uri = file_path_to_uri(Path::new("/tmp"));
        assert!(uri.is_some());
        assert!(uri.unwrap().starts_with("file:///tmp"));
    }

    #[test]
    fn language_from_uri_rust() {
        assert_eq!(language_from_uri("file:///foo/bar.rs"), "rust");
    }

    #[test]
    fn language_from_uri_typescript() {
        assert_eq!(language_from_uri("file:///foo/bar.ts"), "typescript");
        assert_eq!(language_from_uri("file:///foo/bar.tsx"), "typescriptreact");
    }

    #[test]
    fn language_from_uri_javascript() {
        assert_eq!(language_from_uri("file:///foo/bar.js"), "javascript");
        assert_eq!(language_from_uri("file:///foo/bar.mjs"), "javascript");
        assert_eq!(language_from_uri("file:///foo/bar.jsx"), "javascriptreact");
    }

    #[test]
    fn language_from_uri_python() {
        assert_eq!(language_from_uri("file:///foo/bar.py"), "python");
        assert_eq!(language_from_uri("file:///foo/bar.pyi"), "python");
    }

    #[test]
    fn language_from_uri_c_cpp() {
        assert_eq!(language_from_uri("file:///foo/bar.c"), "c");
        assert_eq!(language_from_uri("file:///foo/bar.h"), "c");
        assert_eq!(language_from_uri("file:///foo/bar.cpp"), "cpp");
        assert_eq!(language_from_uri("file:///foo/bar.hpp"), "cpp");
    }

    #[test]
    fn language_from_uri_dockerfile() {
        assert_eq!(language_from_uri("file:///foo/Dockerfile"), "dockerfile");
        assert_eq!(language_from_uri("file:///foo/dockerfile"), "dockerfile");
    }

    #[test]
    fn language_from_uri_unknown_returns_extension() {
        assert_eq!(language_from_uri("file:///foo/bar.xyz"), "xyz");
    }

    #[test]
    fn language_from_uri_no_extension_returns_empty() {
        assert_eq!(language_from_uri("file:///foo/Makefile"), "");
    }

    #[test]
    fn matches_file_pattern_wildcard() {
        assert!(matches_file_pattern("/any/path.rs", "*"));
        assert!(matches_file_pattern("", "*"));
    }

    #[test]
    fn matches_file_pattern_extension() {
        assert!(matches_file_pattern("/src/main.rs", "*.rs"));
        assert!(matches_file_pattern("/src/main.RS", "*.rs")); // case-insensitive
        assert!(!matches_file_pattern("/src/main.py", "*.rs"));
    }

    #[test]
    fn matches_file_pattern_exact_name() {
        assert!(matches_file_pattern("/src/Makefile", "Makefile"));
        assert!(!matches_file_pattern("/src/makefile", "Makefile"));
    }

    #[test]
    fn matches_file_pattern_no_extension_no_match() {
        assert!(!matches_file_pattern("/src/Makefile", "*.rs"));
    }
}
