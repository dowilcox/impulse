// Ports of `get_git_status_for_directory` and `get_all_git_statuses` from
// impulse-core/src/filesystem.rs (the git-status half of that module).

import Clibgit2
import Foundation

/// Priority ranking for git status codes. Higher value = higher priority.
/// Used when propagating status from files to parent directories.
func gitStatusPriority(_ code: String) -> UInt8 {
  switch code {
  case "C": return 6  // conflict
  case "D": return 5  // deleted
  case "A": return 4  // added (staged)
  case "?": return 3  // untracked
  case "R": return 2  // renamed
  case "M": return 1  // modified
  case "I": return 0  // ignored
  default: return 0
  }
}

/// Convert a `git_status_t` bitflags value to a single-character status code.
func statusToCode(_ status: UInt32) -> String? {
  func any(_ mask: UInt32) -> Bool { status & mask != 0 }

  if any(GIT_STATUS_IGNORED.rawValue) {
    return "I"
  } else if any(GIT_STATUS_CONFLICTED.rawValue) {
    return "C"
  } else if any(GIT_STATUS_WT_NEW.rawValue | GIT_STATUS_INDEX_NEW.rawValue) {
    return any(GIT_STATUS_INDEX_NEW.rawValue) ? "A" : "?"
  } else if any(GIT_STATUS_WT_DELETED.rawValue | GIT_STATUS_INDEX_DELETED.rawValue) {
    return "D"
  } else if any(GIT_STATUS_INDEX_RENAMED.rawValue | GIT_STATUS_WT_RENAMED.rawValue) {
    return "R"
  } else if any(
    GIT_STATUS_WT_MODIFIED.rawValue | GIT_STATUS_INDEX_MODIFIED.rawValue
      | GIT_STATUS_WT_TYPECHANGE.rawValue | GIT_STATUS_INDEX_TYPECHANGE.rawValue)
  {
    return "M"
  }
  return nil
}

/// Repo-relative path of a status entry, mirroring git2-rs `StatusEntry::path`
/// (head-to-index delta preferred, then index-to-workdir).
func statusEntryPath(_ entry: UnsafePointer<git_status_entry>) -> String? {
  if let headToIndex = entry.pointee.head_to_index {
    guard let path = headToIndex.pointee.old_file.path else { return nil }
    return String(cString: path)
  }
  if let indexToWorkdir = entry.pointee.index_to_workdir {
    guard let path = indexToWorkdir.pointee.old_file.path else { return nil }
    return String(cString: path)
  }
  return nil
}

extension GitClient {
  /// Git status for files directly in a directory: {filename: statusCode}.
  /// Subdirectories are marked with the highest-priority status among their
  /// descendants. Empty map when the path is not in a git repository.
  public static func statusForDirectory(_ path: String) -> [String: String]? {
    // Canonicalize to resolve symlinks (e.g. /var -> /private/var on macOS)
    // so paths match the repo root reported by libgit2.
    let dirPath = canonicalPath(path) ?? lexicallyNormalized(path)
    guard let repo = try? openRepo(at: dirPath) else { return [:] }
    guard let repoRoot = try? repo.workdir() else { return nil }

    var options = git_status_options()
    git_status_options_init(&options, UInt32(GIT_STATUS_OPTIONS_VERSION))
    options.show = GIT_STATUS_SHOW_INDEX_AND_WORKDIR
    options.flags =
      GIT_STATUS_OPT_INCLUDE_UNTRACKED.rawValue | GIT_STATUS_OPT_UPDATE_INDEX.rawValue

    // Restrict to the requested directory relative to the repo root. At the
    // repo root, skip the pathspec entirely to list all statuses.
    let relDir = relativePathComponents(of: dirPath, under: repoRoot)
    let spec: String? = {
      guard let rel = relDir, !rel.isEmpty else { return nil }
      return rel.joined(separator: "/") + "/"
    }()

    func collect(_ options: inout git_status_options) -> [String: String]? {
      var listPointer: OpaquePointer?
      guard git_status_list_new(&listPointer, repo.raw, &options) == 0, let list = listPointer
      else { return nil }
      defer { git_status_list_free(list) }

      let dirComponents = pathComponents(dirPath)
      let rootComponents = pathComponents(repoRoot)
      var statusMap: [String: String] = [:]

      for index in 0..<git_status_list_entrycount(list) {
        guard let entry = git_status_byindex(list, index),
          let relPath = statusEntryPath(entry),
          let code = statusToCode(entry.pointee.status.rawValue)
        else { continue }

        let absComponents = rootComponents + pathComponents(trimTrailingSlash(relPath))

        // Only include files within the requested directory.
        guard absComponents.count > dirComponents.count,
          Array(absComponents.prefix(dirComponents.count)) == dirComponents
        else { continue }
        let relToDir = Array(absComponents.dropFirst(dirComponents.count))

        if relToDir.count == 1 {
          // File is directly in this directory — use its exact status.
          statusMap[relToDir[0]] = code
        } else {
          // File is in a subdirectory — mark the immediate child directory
          // with the highest-priority status among its descendants. Ignored
          // status is never propagated to parent directories.
          if code == "I" { continue }
          let dirName = relToDir[0]
          if let existing = statusMap[dirName] {
            if gitStatusPriority(code) > gitStatusPriority(existing) {
              statusMap[dirName] = code
            }
          } else {
            statusMap[dirName] = code
          }
        }
      }
      return statusMap
    }

    if let spec {
      return withGitStrarray([spec]) { array in
        options.pathspec = array
        return collect(&options)
      }
    }
    return collect(&options)
  }

  /// Batch-fetch git status for the entire repository: outer key = directory
  /// absolute path, inner key = filename, value = status code. Parent
  /// directories receive the highest-priority status among their descendants.
  public static func allStatuses(root path: String) -> [String: [String: String]]? {
    let dirPath = canonicalPath(path) ?? lexicallyNormalized(path)
    guard let repo = try? openRepo(at: dirPath) else { return [:] }
    guard let repoRoot = try? repo.workdir() else { return nil }

    var options = git_status_options()
    git_status_options_init(&options, UInt32(GIT_STATUS_OPTIONS_VERSION))
    options.show = GIT_STATUS_SHOW_INDEX_AND_WORKDIR
    options.flags =
      GIT_STATUS_OPT_INCLUDE_UNTRACKED.rawValue | GIT_STATUS_OPT_INCLUDE_IGNORED.rawValue
      | GIT_STATUS_OPT_UPDATE_INDEX.rawValue

    var listPointer: OpaquePointer?
    guard git_status_list_new(&listPointer, repo.raw, &options) == 0, let list = listPointer
    else { return nil }
    defer { git_status_list_free(list) }

    var result: [String: [String: String]] = [:]
    let rootComponents = pathComponents(repoRoot)

    func merge(into dir: String, name: String, code: String) {
      var dirMap = result[dir] ?? [:]
      if let existing = dirMap[name] {
        if gitStatusPriority(code) > gitStatusPriority(existing) {
          dirMap[name] = code
        }
      } else {
        dirMap[name] = code
      }
      result[dir] = dirMap
    }

    for index in 0..<git_status_list_entrycount(list) {
      guard let entry = git_status_byindex(list, index),
        let relPath = statusEntryPath(entry),
        let code = statusToCode(entry.pointee.status.rawValue)
      else { continue }

      let relComponents = pathComponents(trimTrailingSlash(relPath))
      guard let fileName = relComponents.last else { continue }
      let absComponents = rootComponents + relComponents
      let parentComponents = Array(absComponents.dropLast())
      let parentPath = "/" + parentComponents.joined(separator: "/")

      // Add the file to its direct parent directory's map.
      merge(into: parentPath, name: fileName, code: code)

      // Don't propagate ignored status to ancestor directories.
      if code == "I" { continue }

      // Propagate status to ancestor directories up to the repo root.
      var child = parentComponents
      while !child.isEmpty {
        guard child.count > rootComponents.count,
          Array(child.prefix(rootComponents.count)) == rootComponents
        else { break }
        let ancestor = Array(child.dropLast())
        let dirName = child[child.count - 1]
        let ancestorPath = "/" + ancestor.joined(separator: "/")
        merge(into: ancestorPath, name: dirName, code: code)
        child = ancestor
      }
    }

    // Remap keys to use the caller's path prefix instead of repo_root (the
    // workdir may be canonicalized with different casing on macOS).
    let repoRootPrefix = trimTrailingSlash(repoRoot)
    let callerPrefix = trimTrailingSlash(path)
    if repoRootPrefix != callerPrefix {
      var remapped: [String: [String: String]] = [:]
      for (key, value) in result {
        if key.hasPrefix(repoRootPrefix) {
          remapped[callerPrefix + String(key.dropFirst(repoRootPrefix.count))] = value
        } else {
          remapped[key] = value
        }
      }
      return remapped
    }
    return result
  }
}
