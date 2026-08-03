// Port of `util::language_from_uri` (impulse-core/src/util.rs) operating
// directly on a file path — the Rust code round-trips through a file:// URI,
// which is equivalent for absolute paths.

import Foundation

/// Monaco/LSP language id for a file path, based on well-known filenames and
/// the (lowercased) extension. Unknown extensions fall through as themselves.
func languageForPath(_ path: String) -> String {
  let name = (path as NSString).lastPathComponent
  if name.lowercased() == "dockerfile" {
    return "dockerfile"
  }
  // Common extensionless files
  switch name {
  case "Makefile", "makefile", "GNUmakefile": return "makefile"
  case "CMakeLists.txt": return "cmake"
  case "Gemfile", "Rakefile": return "ruby"
  case "Vagrantfile": return "ruby"
  case "Jenkinsfile": return "groovy"
  default: break
  }

  let ext = (name as NSString).pathExtension.lowercased()
  switch ext {
  case "rs": return "rust"
  case "py", "pyi": return "python"
  case "js", "mjs", "cjs": return "javascript"
  case "jsx": return "javascriptreact"
  case "ts": return "typescript"
  case "tsx": return "typescriptreact"
  case "c", "h": return "c"
  case "cpp", "cxx", "cc", "hpp", "hxx": return "cpp"
  case "html", "htm": return "html"
  case "css": return "css"
  case "scss": return "scss"
  case "less": return "less"
  case "json": return "json"
  case "jsonc": return "jsonc"
  case "yaml", "yml": return "yaml"
  case "vue": return "vue"
  case "svelte": return "svelte"
  case "graphql", "gql": return "graphql"
  case "sh", "bash", "zsh", "fish": return "shellscript"
  case "dockerfile": return "dockerfile"
  case "go": return "go"
  case "java": return "java"
  case "rb": return "ruby"
  case "lua": return "lua"
  case "zig": return "zig"
  case "php": return "php"
  default: return ext
  }
}
