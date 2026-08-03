// Ports of `commit_all`, `discard_file_changes`, `discard_path`,
// `staged_rename_original`, and `restore_rename` from impulse-core/src/git.rs.

import Clibgit2
import Foundation

extension GitClient {
  /// Stage all changes (additions, modifications, deletions) and create a
  /// commit on HEAD. Returns the new commit's OID as a hex string, or the raw
  /// git error text on failure.
  public static func commitAll(repoPath: String, message: String) -> Result<String, GitError> {
    if message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
      return .failure(GitError("Commit message is empty"))
    }

    let repo: GitRepo
    do {
      repo = try openRepo(at: repoPath)
    } catch let error as GitError {
      return .failure(error)
    } catch {
      return .failure(GitError(String(describing: error)))
    }

    // Refuse to commit while a merge/rebase/cherry-pick/etc. is in progress —
    // committing would bake conflict markers into the tree and drop the
    // in-progress operation's extra parent, corrupting history.
    if git_repository_state(repo.raw) != GIT_REPOSITORY_STATE_NONE.rawValue {
      return .failure(
        GitError(
          "Cannot commit: a merge, rebase, or other operation is in progress. Resolve it first."
        ))
    }

    var indexPointer: OpaquePointer?
    guard git_repository_index(&indexPointer, repo.raw) == 0, let index = indexPointer else {
      return .failure(GitError("Index error: \(gitLastError())"))
    }
    defer { git_index_free(index) }

    if git_index_has_conflicts(index) == 1 {
      return .failure(GitError("Cannot commit: there are unresolved merge conflicts."))
    }

    // Stage new + modified files.
    let addResult = withGitStrarray(["*"]) { array in
      git_index_add_all(index, &array, 0, nil, nil)
    }
    guard addResult == 0 else {
      return .failure(GitError("Failed to stage files: \(gitLastError())"))
    }
    // Stage deletions of tracked files (add_all does not remove them).
    let updateResult = withGitStrarray(["*"]) { array in
      git_index_update_all(index, &array, nil, nil)
    }
    guard updateResult == 0 else {
      return .failure(GitError("Failed to stage deletions: \(gitLastError())"))
    }
    guard git_index_write(index) == 0 else {
      return .failure(GitError("Failed to write index: \(gitLastError())"))
    }

    var treeId = git_oid()
    guard git_index_write_tree(&treeId, index) == 0 else {
      return .failure(GitError("Failed to write tree: \(gitLastError())"))
    }
    var treePointer: OpaquePointer?
    guard git_tree_lookup(&treePointer, repo.raw, &treeId) == 0, let tree = treePointer else {
      return .failure(GitError("Failed to find tree: \(gitLastError())"))
    }
    defer { git_tree_free(tree) }

    let parentCommit = repo.headCommit()
    defer {
      if let parent = parentCommit { git_commit_free(parent) }
    }

    // If nothing changed relative to the parent, refuse.
    if let parent = parentCommit, let parentTreeId = git_commit_tree_id(parent) {
      var newTreeId = treeId
      if git_oid_equal(parentTreeId, &newTreeId) == 1 {
        return .failure(GitError("nothing to commit"))
      }
    }

    var signaturePointer: UnsafeMutablePointer<git_signature>?
    guard git_signature_default(&signaturePointer, repo.raw) == 0, let sig = signaturePointer
    else {
      return .failure(
        GitError("No git signature (configure user.name/user.email): \(gitLastError())"))
    }
    defer { git_signature_free(sig) }

    var commitOid = git_oid()
    var parents: [OpaquePointer?] = parentCommit.map { [$0] } ?? []
    let parentCount = parents.count
    let rc = parents.withUnsafeMutableBufferPointer { buffer in
      git_commit_create(
        &commitOid, repo.raw, "HEAD", sig, sig, nil, message, tree, parentCount,
        buffer.baseAddress)
    }
    guard rc == 0 else {
      return .failure(GitError("Commit failed: \(gitLastError())"))
    }
    return .success(oidHex(commitOid))
  }

  /// Discard working-tree changes for a single file, restoring it to the HEAD
  /// version. For untracked files this is a no-op. `workspaceRoot` is used to
  /// validate that the file is within the workspace.
  public static func discardFileChanges(filePath: String, workspaceRoot: String) throws {
    do {
      try validatePathWithinRoot(filePath, root: workspaceRoot)
    } catch let error as GitError {
      throw GitError("Cannot discard changes: \(error.message)")
    }

    let repo = try openRepo(at: filePath)
    let workdir = try repo.workdir()
    guard let relComponents = relativePathComponents(of: filePath, under: workdir),
      !relComponents.isEmpty
    else {
      throw GitError("File not in repo")
    }
    try checkoutHead(repo: repo, path: relComponents.joined(separator: "/"))
  }

  /// Discard a single repo-relative path back to a clean state:
  /// - tracked modified/deleted: checkout from HEAD
  /// - untracked/new: delete the file (and unstage if staged)
  /// - new side of a staged rename: undo the rename, restoring the old path
  public static func discardPath(repoPath: String, filePath: String) throws {
    let repo = try openRepo(at: repoPath)
    let workdir = try repo.workdir()

    // Determine status FIRST so we can pick the right validation strategy.
    var statusFlags: UInt32 = 0
    guard git_status_file(&statusFlags, repo.raw, filePath) == 0 else {
      throw GitError("Failed to get status: \(gitLastError())")
    }

    if statusFlags & (GIT_STATUS_WT_NEW.rawValue | GIT_STATUS_INDEX_NEW.rawValue) != 0 {
      // Per-file status does no rename detection, so the NEW side of a staged
      // rename reports as INDEX_NEW here. Detect that case via repo-wide
      // status with rename detection and restore the original path instead.
      if let oldPath = try stagedRenameOriginal(repo: repo, rel: filePath) {
        try restoreRename(repo: repo, workdir: workdir, newPath: filePath, oldPath: oldPath)
        return
      }

      // Genuinely untracked / brand-new staged file. Validate via disk (we
      // are about to touch the filesystem).
      let absolute = joinPath(workdir, filePath)
      do {
        try validatePathWithinRoot(absolute, root: workdir)
      } catch let error as GitError {
        throw GitError("Cannot discard: \(error.message)")
      }

      var isDirectory: ObjCBool = false
      if FileManager.default.fileExists(atPath: absolute, isDirectory: &isDirectory),
        !isDirectory.boolValue
      {
        do {
          try FileManager.default.removeItem(atPath: absolute)
        } catch {
          throw GitError("Failed to remove \(absolute): \(error.localizedDescription)")
        }
      }
      // If it was staged, unstage it from the index.
      if statusFlags & GIT_STATUS_INDEX_NEW.rawValue != 0 {
        var indexPointer: OpaquePointer?
        guard git_repository_index(&indexPointer, repo.raw) == 0, let index = indexPointer
        else {
          throw GitError("Index error: \(gitLastError())")
        }
        defer { git_index_free(index) }
        guard git_index_remove_bypath(index, filePath) == 0 else {
          throw GitError("Failed to unstage \(filePath): \(gitLastError())")
        }
        guard git_index_write(index) == 0 else {
          throw GitError("Failed to write index: \(gitLastError())")
        }
      }
      return
    }

    // Tracked modified/deleted: restore from HEAD. Lexical containment
    // validation only — the target (and even its parent directory) may be
    // deleted, and checkout_head recreates missing directories.
    do {
      try validateRelPathLexically(root: workdir, rel: filePath)
    } catch let error as GitError {
      throw GitError("Cannot discard: \(error.message)")
    }
    try checkoutHead(repo: repo, path: filePath)
  }

  /// Force-checkout a single path from HEAD.
  static func checkoutHead(repo: GitRepo, path: String) throws {
    try withGitStrarray([path]) { array in
      var options = git_checkout_options()
      git_checkout_options_init(&options, UInt32(GIT_CHECKOUT_OPTIONS_VERSION))
      options.checkout_strategy = GIT_CHECKOUT_FORCE.rawValue
      options.paths = array
      guard git_checkout_head(repo.raw, &options) == 0 else {
        throw GitError("Checkout failed: \(gitLastError())")
      }
    }
  }

  /// If `rel` is the NEW side of a staged rename, return the original (old)
  /// path. Uses repo-wide statuses with rename detection enabled.
  static func stagedRenameOriginal(repo: GitRepo, rel: String) throws -> String? {
    var options = git_status_options()
    git_status_options_init(&options, UInt32(GIT_STATUS_OPTIONS_VERSION))
    options.show = GIT_STATUS_SHOW_INDEX_AND_WORKDIR
    options.flags =
      GIT_STATUS_OPT_INCLUDE_UNTRACKED.rawValue
      | GIT_STATUS_OPT_RENAMES_HEAD_TO_INDEX.rawValue
      | GIT_STATUS_OPT_RENAMES_INDEX_TO_WORKDIR.rawValue

    var listPointer: OpaquePointer?
    guard git_status_list_new(&listPointer, repo.raw, &options) == 0, let list = listPointer
    else {
      throw GitError("Failed to compute statuses: \(gitLastError())")
    }
    defer { git_status_list_free(list) }

    for index in 0..<git_status_list_entrycount(list) {
      guard let entry = git_status_byindex(list, index) else { continue }
      for deltaPointer in [entry.pointee.head_to_index, entry.pointee.index_to_workdir] {
        guard let delta = deltaPointer else { continue }
        if delta.pointee.status == GIT_DELTA_RENAMED,
          let newPath = delta.pointee.new_file.path, String(cString: newPath) == rel,
          let oldPath = delta.pointee.old_file.path
        {
          return String(cString: oldPath)
        }
      }
    }
    return nil
  }

  /// Undo a staged rename `oldPath` -> `newPath`: remove the new file from
  /// disk, unstage it, and restore the original path from HEAD.
  static func restoreRename(repo: GitRepo, workdir: String, newPath: String, oldPath: String)
    throws
  {
    // Validate both paths lexically (no disk access — oldPath may not exist).
    do {
      try validateRelPathLexically(root: workdir, rel: newPath)
      try validateRelPathLexically(root: workdir, rel: oldPath)
    } catch let error as GitError {
      throw GitError("Cannot discard: \(error.message)")
    }

    // Remove the renamed-to file from disk.
    let newAbsolute = joinPath(workdir, newPath)
    var isDirectory: ObjCBool = false
    if FileManager.default.fileExists(atPath: newAbsolute, isDirectory: &isDirectory),
      !isDirectory.boolValue
    {
      do {
        try FileManager.default.removeItem(atPath: newAbsolute)
      } catch {
        throw GitError("Failed to remove \(newAbsolute): \(error.localizedDescription)")
      }
    }

    // Reset the index so the new path is fully gone and the old path matches
    // HEAD again, then check out the old path so its content reappears.
    var indexPointer: OpaquePointer?
    guard git_repository_index(&indexPointer, repo.raw) == 0, let index = indexPointer else {
      throw GitError("Index error: \(gitLastError())")
    }
    defer { git_index_free(index) }
    guard git_index_remove_bypath(index, newPath) == 0 else {
      throw GitError("Failed to unstage \(newPath): \(gitLastError())")
    }

    // Restore the old path's index entry from HEAD (best-effort chain,
    // mirroring the Rust nested-if structure).
    if let treeObject = repo.headTree() {
      defer { git_object_free(treeObject) }
      var entryPointer: OpaquePointer?
      if git_tree_entry_bypath(&entryPointer, treeObject, oldPath) == 0,
        let treeEntry = entryPointer
      {
        defer { git_tree_entry_free(treeEntry) }
        var objectPointer: OpaquePointer?
        if git_tree_entry_to_object(&objectPointer, repo.raw, treeEntry) == 0,
          let object = objectPointer
        {
          defer { git_object_free(object) }
          var blobPointer: OpaquePointer?
          if git_object_peel(&blobPointer, object, GIT_OBJECT_BLOB) == 0,
            let blob = blobPointer
          {
            defer { git_object_free(blob) }
            var indexEntry = git_index_entry()
            indexEntry.mode = numericCast(git_tree_entry_filemode(treeEntry).rawValue)
            indexEntry.file_size = UInt32(
              truncatingIfNeeded: git_blob_rawsize(blob))
            indexEntry.id = git_blob_id(blob).pointee
            let rc = oldPath.withCString { cPath -> Int32 in
              indexEntry.path = cPath
              return git_index_add(index, &indexEntry)
            }
            guard rc == 0 else {
              throw GitError("Failed to restore index entry: \(gitLastError())")
            }
          }
        }
      }
    }
    guard git_index_write(index) == 0 else {
      throw GitError("Failed to write index: \(gitLastError())")
    }

    // Check out the old path from HEAD so its content reappears on disk.
    try checkoutHead(repo: repo, path: oldPath)
  }
}
