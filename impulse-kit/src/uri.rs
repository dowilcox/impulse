//! `file://` URIs and the editor/LSP language id of a file, revived from
//! `impulse-core/src/util.rs` at `v0.29.0` (the Swift ports are
//! `ImpulseGit/Language.swift` and the URI helpers the fixtures cover).

use std::path::Path;

use url::Url;

/// Convert a local path to a `file://` URI.
///
/// Uses path syntax (trailing separator) rather than filesystem I/O to
/// distinguish directories from files, so it works for non-existent paths.
pub fn file_path_to_uri(path: &Path) -> Option<String> {
    let path_str = path.as_os_str().to_string_lossy();
    let is_dir = path_str.ends_with('/') || path_str.ends_with(std::path::MAIN_SEPARATOR);
    if is_dir {
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

/// Decode percent-encoded URL strings, preserving invalid escape sequences.
fn url_decode(input: &str) -> String {
    let mut bytes = Vec::with_capacity(input.len());
    let mut chars = input.as_bytes().iter();

    while let Some(&b) = chars.next() {
        if b == b'%' {
            let hex: Vec<u8> = chars.by_ref().take(2).copied().collect();
            if hex.len() == 2 {
                if let Ok(decoded) = u8::from_str_radix(&String::from_utf8_lossy(&hex), 16) {
                    bytes.push(decoded);
                } else {
                    bytes.push(b'%');
                    bytes.extend_from_slice(&hex);
                }
            } else {
                bytes.push(b'%');
                bytes.extend_from_slice(&hex);
            }
        } else {
            bytes.push(b);
        }
    }

    bytes.retain(|&b| b != 0);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Determine LSP language ID from a file URI based on extension.
pub fn language_from_uri(uri: &str) -> String {
    let path = uri_to_file_path(uri);
    let path_obj = Path::new(&path);
    if let Some(name) = path_obj.file_name().and_then(|n| n.to_str()) {
        if name.eq_ignore_ascii_case("dockerfile") {
            return "dockerfile".to_string();
        }
        // Common extensionless files
        match name {
            "Makefile" | "makefile" | "GNUmakefile" => return "makefile".to_string(),
            "CMakeLists.txt" => return "cmake".to_string(),
            "Gemfile" | "Rakefile" => return "ruby".to_string(),
            "Vagrantfile" => return "ruby".to_string(),
            "Jenkinsfile" => return "groovy".to_string(),
            _ => {}
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

/// The language id for a path (`language_from_uri` without the URI).
pub fn language_for_path(path: &Path) -> String {
    file_path_to_uri(path)
        .map(|uri| language_from_uri(&uri))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_to_file_path_non_file_uri_returns_as_is() {
        assert_eq!(
            uri_to_file_path("https://example.com"),
            "https://example.com"
        );
        assert_eq!(uri_to_file_path("/tmp/foo.txt"), "/tmp/foo.txt");
    }

    #[test]
    fn well_known_names() {
        assert_eq!(language_from_uri("file:///foo/Dockerfile"), "dockerfile");
        assert_eq!(language_from_uri("file:///foo/Jenkinsfile"), "groovy");
        assert_eq!(language_from_uri("file:///foo/SomeRandomFile"), "");
        assert_eq!(language_for_path(Path::new("/a/Makefile")), "makefile");
    }
}
