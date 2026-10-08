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
  /// "Reviewed up to here" marks (see the review's "Since my last review").
  public static let reviewPrefix = "refs/impulse/reviews/"

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

    // The real index's tree, from a copy: write-tree on the index itself
    // takes index.lock, failing the user's or an agent's git add/commit
    // running at the same moment. (It fails with unresolved conflicts; then
    // there's no index tree.)
    let indexCopy = (gitDir as NSString).appendingPathComponent(
      "impulse-snapshot-\(UUID().uuidString).index-tree")
    defer { try? FileManager.default.removeItem(atPath: indexCopy) }
    if FileManager.default.fileExists(atPath: realIndex) {
      try? FileManager.default.copyItem(atPath: realIndex, toPath: indexCopy)
    }
    var indexEnv = GitOperations.environment
    indexEnv["GIT_INDEX_FILE"] = indexCopy
    let indexTree: String? = {
      guard case .success(let result) = GitCLI.run(["write-tree"], in: root, environment: indexEnv, timeout: 60)
      else { return nil }
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

  /// A commit of the working tree as it is (uncommitted and untracked,
  /// non-ignored files included) on top of HEAD, with no ref pointing at it,
  /// for comparing work that isn't committed (git's gc removes it in time).
  /// HEAD itself when nothing is uncommitted; nil when it can't be made.
  public static func workingTreeCommit(root: String) -> String? {
    guard case .success(let dirResult) = GitOperations.git(["rev-parse", "--absolute-git-dir"], in: root),
      case .success(let headResult) = GitOperations.git(["rev-parse", "--verify", "-q", "HEAD"], in: root)
    else { return nil }
    let head = headResult.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
    let gitDir = dirResult.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
    let tempIndex = (gitDir as NSString).appendingPathComponent("impulse-compare-\(UUID().uuidString).index")
    defer { try? FileManager.default.removeItem(atPath: tempIndex) }
    try? FileManager.default.copyItem(atPath: (gitDir as NSString).appendingPathComponent("index"), toPath: tempIndex)
    var env = GitOperations.environment
    env["GIT_INDEX_FILE"] = tempIndex
    guard case .success = GitCLI.run(["add", "--all", "--", "."], in: root, environment: env, timeout: 120),
      case .success(let tree) = GitCLI.run(["write-tree"], in: root, environment: env, timeout: 60)
    else { return nil }
    let treeID = tree.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
    if case .success(let headTree) = GitOperations.git(["rev-parse", "HEAD^{tree}"], in: root),
      headTree.stdout.trimmingCharacters(in: .whitespacesAndNewlines) == treeID
    {
      return head
    }
    var commitEnv = GitOperations.environment
    commitEnv["GIT_AUTHOR_NAME"] = "Impulse"
    commitEnv["GIT_AUTHOR_EMAIL"] = "snapshots@impulse.invalid"
    commitEnv["GIT_COMMITTER_NAME"] = "Impulse"
    commitEnv["GIT_COMMITTER_EMAIL"] = "snapshots@impulse.invalid"
    guard
      case .success(let commit) = GitCLI.run(
        ["commit-tree", treeID, "--no-gpg-sign", "-p", head, "-m", "impulse: working tree"], in: root,
        environment: commitEnv, timeout: 60)
    else { return nil }
    return commit.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
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
      let subject = fields[2]
      let reason =
        subject.hasPrefix("impulse snapshot: ")
        ? String(subject.dropFirst("impulse snapshot: ".count)) : subject
      let indexTree = fields[3].split(separator: "\n")
        .first { $0.hasPrefix("index-tree ") }
        .map { String($0.dropFirst("index-tree ".count)) }
      return SafetySnapshot(
        ref: ref, commit: fields[1], indexTree: indexTree, date: date(ofRef: ref), reason: reason)
    }.sorted { $0.date > $1.date }
  }

  /// Put `paths` (default: everything) back to how they were in `snapshot`,
  /// in the working tree and, when recorded, the index. Files that aren't in
  /// the snapshot (created after it) are left alone.
  public static func restore(_ snapshot: SafetySnapshot, paths: [String] = [], root: String)
    -> GitResult
  {
    let pathspec = paths.isEmpty ? ["."] : paths
    // Overlay: files that aren't in the snapshot stay as they are.
    let worktree = GitOperations.git(
      GitOperations.literal(["restore", "--overlay", "--source=\(snapshot.commit)", "--worktree", "--"] + pathspec),
      in: root)
    if case .failure(let error) = worktree { return .failure(error) }
    // The empty tree means the index was empty: nothing to put back (and
    // git rejects a pathspec that matches nothing).
    let emptyTree = "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
    if let indexTree = snapshot.indexTree, indexTree != emptyTree {
      // Only paths either index knows: an untracked file brought back above
      // matches neither, and git would reject the whole command.
      let staged = paths.isEmpty ? pathspec : paths.filter { path in
        indexHas(path, tree: indexTree, root: root)
      }
      guard !staged.isEmpty else { return .success(()) }
      let index = GitOperations.git(
        GitOperations.literal(["restore", "--overlay", "--source=\(indexTree)", "--staged", "--"] + staged),
        in: root)
      if case .failure(let error) = index { return .failure(error) }
    }
    return .success(())
  }

  /// `path` is in the saved index tree or the current index.
  private static func indexHas(_ path: String, tree: String, root: String) -> Bool {
    let inTree = GitOperations.git(
      GitOperations.literal(["ls-tree", "-r", "--name-only", tree, "--", path]), in: root)
    if case .success(let result) = inTree, !result.stdout.isEmpty { return true }
    let inIndex = GitOperations.git(GitOperations.literal(["ls-files", "--", path]), in: root)
    if case .success(let result) = inIndex, !result.stdout.isEmpty { return true }
    return false
  }

  /// How long `prune` keeps snapshots by default.
  public static let defaultMaxAge: TimeInterval = 14 * 24 * 3600

  /// Delete snapshots beyond the newest `keep` or older than `maxAge`, in
  /// one ref transaction (thousands of old refs take one git process, not
  /// one each). Returns the refs deleted.
  @discardableResult
  public static func prune(
    root: String, prefix: String = oplogPrefix, keep: Int = 200,
    maxAge: TimeInterval = defaultMaxAge
  ) -> [String] {
    guard
      case .success(let result) = GitOperations.git(
        ["for-each-ref", "--format=%(refname)", prefix], in: root, timeout: 30)
    else { return [] }
    let cutoff = Date().addingTimeInterval(-maxAge)
    let refs = result.stdout.split(separator: "\n").map { String($0) }
      .map { (ref: $0, date: date(ofRef: $0)) }
      .sorted { $0.date > $1.date }
    let doomed = refs.enumerated()
      .filter { $0.offset >= keep || $0.element.date < cutoff }
      .map(\.element.ref)
    guard !doomed.isEmpty else { return [] }
    let commands = doomed.map { "delete \($0)\n" }.joined()
    guard
      case .success = GitOperations.git(
        ["update-ref", "--stdin"], in: root, stdin: Data(commands.utf8), timeout: 60)
    else { return [] }
    return doomed
  }

  /// A snapshot's time, from the `<millis>-<reason>` last path component.
  static func date(ofRef ref: String) -> Date {
    let leaf = ref.split(separator: "/").last ?? ""
    let millis = leaf.split(separator: "-").first.flatMap { Double($0) } ?? 0
    return Date(timeIntervalSince1970: millis / 1000)
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
