import Foundation

// Ported from `matches_file_pattern` in impulse-core/src/util.rs.
//
// Deliberately NOT a full glob implementation — it supports exactly what the
// settings file-pattern overrides support: `"*"` (match all), `"*.ext"`
// (case-insensitive extension match), and exact filename matching.
public enum Glob {
  public static func matchesFilePattern(path: String, pattern: String) -> Bool {
    if pattern == "*" {
      return true
    }
    if pattern.hasPrefix("*.") {
      let extPattern = String(pattern.dropFirst(2))
      guard let ext = fileExtension(of: path) else { return false }
      return ext.lowercased() == extPattern.lowercased()
    }
    // Exact filename match (e.g. "Makefile")
    return fileName(of: path) == pattern
  }

  /// Last path component, mirroring Rust's `Path::file_name` (empty string
  /// for paths ending in `..` or root).
  private static func fileName(of path: String) -> String {
    let component = (path as NSString).lastPathComponent
    return component
  }

  /// Extension semantics matching Rust's `Path::extension`: the suffix after
  /// the last `.` of the file name, and only if the part before that `.` is
  /// non-empty (so ".hidden" has no extension).
  private static func fileExtension(of path: String) -> String? {
    let name = fileName(of: path)
    guard let dotIndex = name.lastIndex(of: "."), dotIndex != name.startIndex else {
      return nil
    }
    let ext = name[name.index(after: dotIndex)...]
    return ext.isEmpty ? nil : String(ext)
  }
}
