// Safety snapshots: before any destructive action (discard, reset, branch
// switch over local changes, stash drop, ...) Impulse records the working
// tree — including untracked, non-ignored files — and the index as a commit
// under a private ref, so the action can be undone. The same mechanism
// records agent-turn checkpoints.
//
// Refs live outside refs/heads and refs/tags, so a normal `git push` never
// sends them (`git push --mirror` would). They are pruned by age and count.

import Foundation

public struct SafetySnapshot: Equatable, Sendable {
  /// Full ref name, e.g. refs/impulse/oplog/1767225600123-discard.
  public let ref: String
  /// Snapshot commit id (its tree is the working tree at the time).
  public let commit: String
  /// Tree of the index at the time (nil when the index had conflicts).
  public let indexTree: String?
  public let date: Date
  /// What was about to happen ("Discard changes to a.txt").
  public let reason: String
}

public enum SafetySnapshots {
  public static let oplogPrefix = "refs/impulse/oplog/"
  public static let checkpointPrefix = "refs/impulse/checkpoints/"

  /// Record the working tree + index under `prefix` (oplog by default).
  public static func create(
    reason: String, root: String, prefix: String = oplogPrefix
  ) -> Result<SafetySnapshot, GitOperationError> {
    func git(_ args: [String], _ timeout: TimeInterval = 60) -> Result<GitCLIResult, GitOperationError> {
      GitOperations.git(args, in: root, timeout: timeout)
    }
    guard
      case .success(let dirResult) = git(["rev-parse", "--absolute-git-dir"])
    else { return .failure(.invalid("Not a git repository")) }
    let gitDir = dirResult.stdout.trimmingCharacters(in: .whitespacesAndNewlines)

    // Build the working-tree tree in a temporary index so the real index is
    // untouched.
    let tempIndex = (gitDir as NSString).appendingPathComponent(
      "impulse-snapshot-\(UUID().uuidString).index")
    defer { try? FileManager.default.removeItem(atPath: tempIndex) }
    let realIndex = (gitDir as NSString).appendingPathComponent("index")
    if FileManager.default.fileExists(atPath: realIndex) {
      try? FileManager.default.copyItem(atPath: realIndex, toPath: tempIndex)
    }
    var env = GitOperations.environment
    env["GIT_INDEX_FILE"] = tempIndex
    func gitTemp(_ args: [String]) -> Result<GitCLIResult, GitOperationError> {
      GitCLI.run(args, in: root, environment: env, timeout: 120).mapError { .cli($0) }
    }
    if case .failure(let error) = gitTemp(["add", "--all", "--", "."]) { return .failure(error) }
    guard case .success(let treeResult) = gitTemp(["write-tree"]) else {
      return .failure(.invalid("Couldn't record the working tree."))
    }
    let worktreeTree = treeResult.stdout.trimmingCharacters(in: .whitespacesAndNewlines)

    // The real index's tree (fails with unresolved conflicts; then skip it).
    let indexTree: String? = {
      guard case .success(let result) = git(["write-tree"]) else { return nil }
      return result.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
    }()

    let head: String? = {
      guard case .success(let result) = git(["rev-parse", "--verify", "-q", "HEAD"]) else {
        return nil
      }
      let value = result.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
      return value.isEmpty ? nil : value
    }()

    var message = "impulse snapshot: \(reason)\n"
    if let indexTree { message += "\nindex-tree \(indexTree)\n" }
    var args = ["commit-tree", worktreeTree, "--no-gpg-sign", "-F", "-"]
    if let head { args += ["-p", head] }
    // A fixed identity: snapshots are internal and must work even when the
    // user hasn't configured git's user.name / user.email.
    var commitEnv = GitOperations.environment
    commitEnv["GIT_AUTHOR_NAME"] = "Impulse"
    commitEnv["GIT_AUTHOR_EMAIL"] = "snapshots@impulse.invalid"
    commitEnv["GIT_COMMITTER_NAME"] = "Impulse"
    commitEnv["GIT_COMMITTER_EMAIL"] = "snapshots@impulse.invalid"
    let commitResult = GitCLI.run(
      args, in: root, stdin: Data(message.utf8), environment: commitEnv, timeout: 60
    ).mapError { GitOperationError.cli($0) }
    guard case .success(let created) = commitResult else {
      if case .failure(let error) = commitResult { return .failure(error) }
      return .failure(.invalid("Couldn't record a snapshot."))
    }
    let commit = created.stdout.trimmingCharacters(in: .whitespacesAndNewlines)

    let now = Date()
    let millis = Int(now.timeIntervalSince1970 * 1000)
    let ref = prefix + "\(millis)-\(slug(reason))"
    if case .failure(let error) = git(["update-ref", ref, commit]) {
      return .failure(error)
    }
    return .success(
      SafetySnapshot(ref: ref, commit: commit, indexTree: indexTree, date: now, reason: reason))
  }

  /// Snapshots under `prefix` (including nested folders, e.g. one per
  /// terminal under the checkpoint prefix), newest first.
  public static func list(root: String, prefix: String = oplogPrefix) -> [SafetySnapshot] {
    guard
      case .success(let result) = GitOperations.git(
        [
          "for-each-ref", "--sort=-refname",
          "--format=%(refname)%00%(objectname)%00%(contents:subject)%00%(contents:body)%1e",
          prefix,
        ], in: root, timeout: 30)
    else { return [] }
    return result.stdout.components(separatedBy: "\u{1e}").compactMap { chunk in
      let fields = chunk.trimmingCharacters(in: .newlines).components(separatedBy: "\0")
      guard fields.count >= 4 else { return nil }
      let ref = fields[0]
      // <millis>-<reason> is the last path component.
      let leaf = ref.split(separator: "/").last ?? ""
      let millis = leaf.split(separator: "-").first.flatMap { Double($0) } ?? 0
      let subject = fields[2]
      let reason =
        subject.hasPrefix("impulse snapshot: ")
        ? String(subject.dropFirst("impulse snapshot: ".count)) : subject
      let indexTree = fields[3].split(separator: "\n")
        .first { $0.hasPrefix("index-tree ") }
        .map { String($0.dropFirst("index-tree ".count)) }
      return SafetySnapshot(
        ref: ref, commit: fields[1], indexTree: indexTree,
        date: Date(timeIntervalSince1970: millis / 1000), reason: reason)
    }.sorted { $0.date > $1.date }
  }

  /// Put `paths` (default: everything) back to how they were in `snapshot`,
  /// in the working tree and, when recorded, the index. Files created after
  /// the snapshot are left alone.
  public static func restore(_ snapshot: SafetySnapshot, paths: [String] = [], root: String)
    -> GitResult
  {
    let pathspec = paths.isEmpty ? ["."] : paths
    let worktree = GitOperations.git(
      ["restore", "--source=\(snapshot.commit)", "--worktree", "--"] + pathspec, in: root)
    if case .failure(let error) = worktree { return .failure(error) }
    // The empty tree means the index was empty: nothing to put back (and
    // git rejects a pathspec that matches nothing).
    let emptyTree = "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
    if let indexTree = snapshot.indexTree, indexTree != emptyTree {
      let index = GitOperations.git(
        ["restore", "--source=\(indexTree)", "--staged", "--"] + pathspec, in: root)
      if case .failure(let error) = index { return .failure(error) }
    }
    return .success(())
  }

  /// Delete snapshots beyond `keep` or older than `maxAge`.
  public static func prune(
    root: String, prefix: String = oplogPrefix, keep: Int = 200,
    maxAge: TimeInterval = 14 * 24 * 3600
  ) {
    let cutoff = Date().addingTimeInterval(-maxAge)
    for (index, snapshot) in list(root: root, prefix: prefix).enumerated()
    where index >= keep || snapshot.date < cutoff {
      _ = GitOperations.git(["update-ref", "-d", snapshot.ref], in: root, timeout: 30)
    }
  }

  static func slug(_ reason: String) -> String {
    let lowered = reason.lowercased()
    var out = ""
    for scalar in lowered.unicodeScalars {
      if CharacterSet.alphanumerics.contains(scalar) && scalar.isASCII {
        out.unicodeScalars.append(scalar)
      } else if !out.hasSuffix("-") {
        out.append("-")
      }
      if out.count >= 32 { break }
    }
    let trimmed = out.trimmingCharacters(in: CharacterSet(charactersIn: "-"))
    return trimmed.isEmpty ? "snapshot" : trimmed
  }
}
