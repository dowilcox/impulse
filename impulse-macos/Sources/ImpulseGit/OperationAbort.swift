// Aborting a merge, cherry-pick or revert so it always works and can be
// undone. `git merge --abort` (and the others) refuse once a conflicted file
// has been edited ("Entry … not uptodate. Cannot merge."), which leaves
// finishing the operation or `git reset --hard` as the only ways out. Here a
// snapshot is taken first; when git refuses, Impulse resets to HEAD itself
// and puts back the uncommitted work from before the operation.
//
// Telling that work apart from edits made while resolving needs a record
// taken when the operation started (`recordStart`): the files that were
// dirty then and that the incoming side doesn't change. Git won't start an
// operation whose changes would overwrite uncommitted work, so those files
// are exactly the work from before, untouched by the operation.

import Foundation
import ImpulseKit

public enum OperationAbort {
  /// The work from before an operation, recorded when it started.
  public struct StartRecord: Codable, Equatable, Sendable {
    /// HEAD and the incoming commit (MERGE_HEAD, CHERRY_PICK_HEAD or
    /// REVERT_HEAD) when it started; the record only applies to that
    /// operation.
    public var head: String
    public var incoming: String
    /// The files with uncommitted work from before.
    public var paths: [String]
    /// A snapshot holding them; nil when there were none.
    public var snapshot: String?
  }

  public enum Outcome: Equatable, Sendable {
    /// git's own `--abort` worked.
    case aborted
    /// git refused; Impulse reset to HEAD and put back `kept`. `exact` when
    /// a start record told the work from before apart from later edits.
    case reset(kept: [String], exact: Bool)
  }

  public struct Aborted: Equatable, Sendable {
    public let outcome: Outcome
    /// Everything as it was before aborting, for Undo.
    public let undo: SafetySnapshot?
    /// The incoming commit, for reopening a merge on Undo.
    public let incoming: String?
  }

  /// The operations this applies to (rebase aborts reliably already).
  public static func applies(to operation: RepoOperation) -> Bool {
    switch operation {
    case .merge, .cherryPick, .revert: return true
    default: return false
    }
  }

  /// Record the work from before the operation now in progress at `root`,
  /// unless it's recorded already. Only while the operation is new (its
  /// head file is under `maxAge` old): later, edits made while resolving
  /// could pass for earlier work.
  public static func recordStartIfNeeded(root: String, maxAge: TimeInterval = 60) {
    guard let gitDir = gitDirectory(root: root), let found = incomingHead(root: root),
      let head = revParse("HEAD", root: root)
    else { return }
    let (name, incoming) = (found.name, found.commit)
    if let record = loadRecord(gitDir: gitDir), record.head == head, record.incoming == incoming { return }
    let headFile = (gitDir as NSString).appendingPathComponent(name)
    guard let modified = (try? FileManager.default.attributesOfItem(atPath: headFile))?[.modificationDate] as? Date,
      Date().timeIntervalSince(modified) <= maxAge
    else { return }
    let paths = workFromBefore(root: root, name: name, incoming: incoming)
    var snapshot: String?
    if !paths.isEmpty {
      guard case .success(let created) = SafetySnapshots.create(reason: "before \(name)", root: root) else { return }
      snapshot = created.commit
    }
    saveRecord(StartRecord(head: head, incoming: incoming, paths: paths, snapshot: snapshot), gitDir: gitDir)
  }

  /// Abort `operation` at `root`: a snapshot of everything first (for
  /// Undo), then git's `--abort`, then, if git refuses, a hard reset to HEAD
  /// with the work from before the operation put back.
  public static func abort(_ operation: RepoOperation, root: String) -> Result<Aborted, GitOperationError> {
    let gitDir = gitDirectory(root: root)
    let incoming = incomingHead(root: root)
    let undo = try? SafetySnapshots.create(reason: "abort \(operation.title.lowercased())", root: root).get()
    let plain = GitOperations.perform(.abort, on: operation, root: root)
    if case .success = plain {
      if let gitDir { removeRecord(gitDir: gitDir) }
      return .success(Aborted(outcome: .aborted, undo: undo, incoming: incoming?.commit))
    }
    // Without a snapshot to undo with, don't go further than git would.
    guard let undo, let incoming, let head = revParse("HEAD", root: root) else {
      if case .failure(let error) = plain { return .failure(error) }
      return .failure(.invalid("Couldn't abort."))
    }

    var kept: [String]
    var source: String
    var exact = false
    if let gitDir, let record = loadRecord(gitDir: gitDir), record.head == head, record.incoming == incoming.commit {
      kept = record.paths
      source = record.snapshot ?? undo.commit
      exact = true
    } else {
      kept = workFromBefore(root: root, name: incoming.name, incoming: incoming.commit)
      source = undo.commit
    }

    if case .failure(let error) = GitOperations.git(["reset", "--hard", "-q", "HEAD"], in: root) {
      return .failure(error)
    }
    // A cherry-pick or revert of several commits keeps a sequencer.
    switch operation {
    case .cherryPick: _ = GitOperations.git(["cherry-pick", "--quit"], in: root)
    case .revert: _ = GitOperations.git(["revert", "--quit"], in: root)
    default: break
    }
    if !kept.isEmpty {
      let restored = GitOperations.git(
        GitOperations.literal(["restore", "--overlay", "--source=\(source)", "--worktree", "--"] + kept), in: root)
      if case .failure(let error) = restored { return .failure(error) }
    }
    if let gitDir { removeRecord(gitDir: gitDir) }
    return .success(Aborted(outcome: .reset(kept: kept, exact: exact), undo: undo, incoming: incoming.commit))
  }

  /// Undo an abort: reopen a merge (its conflicts come back) and put every
  /// file back as it was when Abort was chosen. A cherry-pick or revert
  /// isn't reopened; its files are.
  public static func undo(_ result: Aborted, operation: RepoOperation, root: String) -> GitResult {
    guard let undo = result.undo else { return .failure(.invalid("There's nothing to undo with.")) }
    if case .merge = operation, let incoming = result.incoming {
      // Conflicts are expected: they're what was being resolved.
      _ = GitOperations.git(["merge", "--no-ff", "--no-commit", incoming], in: root)
    }
    return SafetySnapshots.restore(undo, root: root)
  }

  // MARK: - Helpers

  /// The files with uncommitted work that the incoming side doesn't change.
  static func workFromBefore(root: String, name: String, incoming: String) -> [String] {
    guard case .success(let dirty) = GitOperations.git(["diff", "--name-only", "--no-renames", "HEAD"], in: root)
    else { return [] }
    let changed: Set<String>
    if name == "MERGE_HEAD" {
      guard let base = mergeBase("HEAD", incoming, root: root),
        case .success(let incomingChanges) = GitOperations.git(
          ["diff", "--name-only", "--no-renames", base, incoming], in: root)
      else { return [] }
      changed = Set(lines(incomingChanges.stdout))
    } else {
      guard
        case .success(let commitChanges) = GitOperations.git(
          ["diff-tree", "-r", "--root", "--no-commit-id", "--name-only", "--no-renames", incoming], in: root)
      else { return [] }
      changed = Set(lines(commitChanges.stdout))
    }
    return lines(dirty.stdout).filter { !changed.contains($0) }
  }

  /// The operation's head file and the commit in it.
  static func incomingHead(root: String) -> (name: String, commit: String)? {
    for name in ["MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD"] {
      if let commit = revParse(name, root: root) { return (name, commit) }
    }
    return nil
  }

  static func recordPath(gitDir: String) -> String {
    (gitDir as NSString).appendingPathComponent("impulse-operation.json")
  }

  static func loadRecord(gitDir: String) -> StartRecord? {
    FileManager.default.contents(atPath: recordPath(gitDir: gitDir))
      .flatMap { try? JSONDecoder().decode(StartRecord.self, from: $0) }
  }

  private static func saveRecord(_ record: StartRecord, gitDir: String) {
    guard let data = try? JSONEncoder().encode(record) else { return }
    try? data.write(to: URL(fileURLWithPath: recordPath(gitDir: gitDir)), options: .atomic)
  }

  private static func removeRecord(gitDir: String) {
    try? FileManager.default.removeItem(atPath: recordPath(gitDir: gitDir))
  }

  /// The worktree's own git folder (where MERGE_HEAD is), absolute.
  private static func gitDirectory(root: String) -> String? {
    guard case .success(let result) = GitOperations.git(["rev-parse", "--absolute-git-dir"], in: root) else {
      return nil
    }
    let path = result.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
    return path.isEmpty ? nil : path
  }

  private static func revParse(_ name: String, root: String) -> String? {
    guard case .success(let result) = GitOperations.git(["rev-parse", "-q", "--verify", name], in: root) else {
      return nil
    }
    let value = result.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
    return value.isEmpty ? nil : value
  }

  private static func mergeBase(_ a: String, _ b: String, root: String) -> String? {
    guard case .success(let result) = GitOperations.git(["merge-base", a, b], in: root) else { return nil }
    return lines(result.stdout).first
  }

  private static func lines(_ text: String) -> [String] {
    text.split(separator: "\n").map(String.init).filter { !$0.isEmpty }
  }
}
