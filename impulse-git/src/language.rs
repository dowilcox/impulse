//! The Monaco/LSP language id of a file, by well-known names and the
//! lowercased extension (the original Rust's `util::language_from_uri`,
//! applied to a path). Unknown extensions fall through as themselves.

use std::path::Path;

pub fn language_for_path(path: &str) -> String {
    let name = Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.to_lowercase() == "dockerfile" {
        return "dockerfile".into();
    }
    match name.as_str() {
        "Makefile" | "makefile" | "GNUmakefile" => return "makefile".into(),
        "CMakeLists.txt" => return "cmake".into(),
        "Gemfile" | "Rakefile" | "Vagrantfile" => return "ruby".into(),
        "Jenkinsfile" => return "groovy".into(),
        _ => {}
    }
    let ext = extension(&name).to_lowercase();
    let language = match ext.as_str() {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "ts" => "typescript",
        "tsx" => "typescriptreact",
        "c" | "h" => "c",
        "cpp" | "cxx" | "cc" | "hpp" | "hxx" => "cpp",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" => "scss",
        "less" => "less",
        "json" => "json",
        "jsonc" => "jsonc",
        "yaml" | "yml" => "yaml",
        "vue" => "vue",
        "svelte" => "svelte",
        "graphql" | "gql" => "graphql",
        "sh" | "bash" | "zsh" | "fish" => "shellscript",
        "dockerfile" => "dockerfile",
        "go" => "go",
        "java" => "java",
        "rb" => "ruby",
        "lua" => "lua",
        "zig" => "zig",
        "php" => "php",
        _ => return ext,
    };
    language.into()
}

/// The extension as `NSString.pathExtension` gives it: after the last dot,
/// unless the dot starts the name (".gitignore" has none).
fn extension(name: &str) -> &str {
    match name.rfind('.') {
        Some(dot) if dot > 0 => &name[dot + 1..],
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages() {
        assert_eq!(language_for_path("/r/src/main.RS"), "rust");
        assert_eq!(language_for_path("/r/Dockerfile"), "dockerfile");
        assert_eq!(language_for_path("/r/Makefile"), "makefile");
        assert_eq!(language_for_path("/r/notes.txt"), "txt");
        assert_eq!(language_for_path("/r/.gitignore"), "");
        assert_eq!(language_for_path("/r/x.zsh"), "shellscript");
    }
}
