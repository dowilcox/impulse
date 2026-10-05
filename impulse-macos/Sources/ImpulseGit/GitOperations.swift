// Mutating git operations. All of them run the user's `git` (GitCLI) so they
// behave exactly as in a terminal: hooks, signing, filters, credentials.

import Foundation
import ImpulseKit

/// Failure of a git operation: a classified CLI failure, or a precondition
/// Impulse checked itself (e.g. the hunk changed since it was shown).
public enum GitOperationError: Error, Equatable, CustomStringConvertible {
  case cli(GitCLIError)
  case stale(String)
  case invalid(String)

  public var message: String {
    switch self {
    case .cli(let error): return error.message
    case .stale(let detail): return detail
    case .invalid(let detail): return detail
    }
  }

  /// Raw git output, when there is any.
  public var output: String? {
    if case .cli(let error) = self { return error.output }
    return nil
  }

  public var description: String { message }
}

public typealias GitResult = Result<Void, GitOperationError>

public enum PatchTarget: Sendable {
  /// Stage selected changes from the unstaged diff into the index.
  case stage
  /// Remove selected changes from the index (staged diff).
  case unstage
  /// Revert selected changes in the working tree (unstaged diff).
  case discard

  var sourceScope: DiffScope { self == .unstage ? .staged : .unstaged }
  var reverse: Bool { self != .stage }
  var applyArguments: [String] {
    switch self {
    case .stage: return ["apply", "--cached", "--recount", "--whitespace=nowarn", "-"]
    case .unstage:
      return ["apply", "--cached", "--reverse", "--recount", "--whitespace=nowarn", "-"]
    case .discard: return ["apply", "--reverse", "--recount", "--whitespace=nowarn", "-"]
    }
  }
}

public enum GitOperations {
  /// Extra environment for every git invocation (tests pin identity/config).
  nonisolated(unsafe) public static var environment: [String: String] = [:]

  @discardableResult
  static func git(
    _ arguments: [String], in root: String, stdin: Data? = nil, timeout: TimeInterval = 120,
    onOutputLine: ((String) -> Void)? = nil
  ) -> Result<GitCLIResult, GitOperationError> {
    GitCLI.run(
      arguments, in: root, stdin: stdin, environment: environment, timeout: timeout,
      onOutputLine: onOutputLine
    ).mapError { .cli($0) }
  }

  static func void(_ result: Result<GitCLIResult, GitOperationError>) -> GitResult {
    result.map { _ in () }
  }

  // MARK: - Staging whole files

  /// `git add` the paths (new, modified, or deleted).
  public static func stage(paths: [String], root: String) -> GitResult {
    guard !paths.isEmpty else { return .success(()) }
    return void(git(["add", "--all", "--"] + paths, in: root))
  }

  /// Stage every change, including untracked files.
  public static func stageAll(root: String) -> GitResult {
    void(git(["add", "--all"], in: root))
  }

  /// Unstage the paths (keep working-tree changes). Works before the first
  /// commit too.
  public static func unstage(paths: [String], root: String, isUnborn: Bool) -> GitResult {
    guard !paths.isEmpty else { return .success(()) }
    if isUnborn {
      return void(git(["rm", "--cached", "-r", "--quiet", "--"] + paths, in: root))
    }
    return void(git(["restore", "--staged", "--"] + paths, in: root))
  }

  public static func unstageAll(root: String, isUnborn: Bool) -> GitResult {
    if isUnborn { return void(git(["rm", "--cached", "-r", "--quiet", "."], in: root)) }
    return void(git(["reset", "--quiet"], in: root))
  }

  /// Revert tracked files' working-tree changes to the index version.
  /// Untracked files are not touched (the app moves those to the Trash).
  public static func discardWorkingTree(paths: [String], root: String) -> GitResult {
    guard !paths.isEmpty else { return .success(()) }
    return void(git(["restore", "--worktree", "--"] + paths, in: root))
  }

  /// Revert files to HEAD in both the index and the working tree.
  public static func discardAll(paths: [String], root: String) -> GitResult {
    guard !paths.isEmpty else { return .success(()) }
    return void(git(["restore", "--staged", "--worktree", "--source=HEAD", "--"] + paths, in: root))
  }

  // MARK: - Hunk / line selections

  /// Stage, unstage or discard a selection of hunks/lines of one file.
  ///
  /// `expectedHunkIds` (hunk index → id from the `FileDiff` the user saw)
  /// guards against acting on a hunk that changed in the meantime.
  public static func apply(
    _ target: PatchTarget, selection: PatchSelection, path: String, oldPath: String? = nil,
    expectedHunkIds: [Int: String] = [:], options: DiffOptions = DiffOptions(), root: String
  ) -> GitResult {
    let scope = target.sourceScope
    if !expectedHunkIds.isEmpty {
      guard
        let current = try? GitClient.fileDiff(
          repoPath: root, path: path, oldPath: oldPath, scope: scope, options: options)
      else { return .failure(.stale("The file's diff couldn't be read. Refresh and try again.")) }
      for (index, id) in expectedHunkIds {
        guard current.hunkIds.indices.contains(index), current.hunkIds[index] == id else {
          return .failure(.stale("The file changed since it was shown. Refresh and try again."))
        }
      }
    }
    let patch: String
    do {
      guard
        let text = try GitClient.patchText(
          repoPath: root, path: path, oldPath: oldPath, scope: scope, options: options)
      else { return .failure(.stale("There are no changes left in \(path).")) }
      patch = try PatchBuilder.build(patch: text, selection: selection, reverse: target.reverse)
    } catch let error as PatchBuilder.BuildError {
      return .failure(.invalid(error.description))
    } catch {
      return .failure(.invalid("\(error)"))
    }
    return void(git(target.applyArguments, in: root, stdin: Data(patch.utf8)))
  }

  // MARK: - Commit

  public struct CommitOptions: Sendable {
    public var amend = false
    public var signOff = false
    public var noVerify = false
    public var allowEmpty = false
    /// Commit only what is staged (default); when true, `--all` stages
    /// tracked changes first.
    public var includeUnstagedTracked = false

    public init(
      amend: Bool = false, signOff: Bool = false, noVerify: Bool = false,
      allowEmpty: Bool = false, includeUnstagedTracked: Bool = false
    ) {
      self.amend = amend
      self.signOff = signOff
      self.noVerify = noVerify
      self.allowEmpty = allowEmpty
      self.includeUnstagedTracked = includeUnstagedTracked
    }
  }

  /// `git commit -F -` with the message on stdin. Returns the new HEAD id.
  public static func commit(message: String, options: CommitOptions = CommitOptions(), root: String)
    -> Result<String, GitOperationError>
  {
    let trimmed = message.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty || options.amend else {
      return .failure(.invalid("Write a commit message first."))
    }
    var args = ["commit", "--quiet", "--cleanup=strip"]
    if trimmed.isEmpty && options.amend {
      args.append("--no-edit")
    } else {
      args += ["-F", "-"]
    }
    if options.amend { args.append("--amend") }
    if options.signOff { args.append("--signoff") }
    if options.noVerify { args.append("--no-verify") }
    if options.allowEmpty { args.append("--allow-empty") }
    if options.includeUnstagedTracked { args.append("--all") }
    let stdin = args.contains("-F") ? Data((trimmed + "\n").utf8) : nil
    return git(args, in: root, stdin: stdin, timeout: 600).flatMap { _ in
      git(["rev-parse", "HEAD"], in: root).map {
        $0.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
      }
    }
  }

  /// Undo the last commit, keeping its changes staged.
  public static func uncommit(root: String) -> GitResult {
    void(git(["reset", "--soft", "HEAD~1"], in: root))
  }

  /// Full message of the HEAD commit (for amend).
  public static func headMessage(root: String) -> String? {
    try? git(["log", "-1", "--format=%B"], in: root).get().stdout
      .trimmingCharacters(in: .whitespacesAndNewlines)
  }

  // MARK: - Branches

  public static func switchBranch(_ name: String, root: String) -> GitResult {
    void(git(["switch", name], in: root))
  }

  /// Create a branch at `startPoint` (default HEAD), optionally switching to it.
  public static func createBranch(
    _ name: String, startPoint: String? = nil, checkout: Bool = true, root: String
  ) -> GitResult {
    var args = checkout ? ["switch", "-c", name] : ["branch", name]
    if let startPoint { args.append(startPoint) }
    return void(git(args, in: root))
  }

  public static func renameBranch(_ name: String, to newName: String, root: String) -> GitResult {
    void(git(["branch", "-m", name, newName], in: root))
  }

  public static func deleteBranch(_ name: String, force: Bool = false, root: String) -> GitResult {
    void(git(["branch", force ? "-D" : "-d", name], in: root))
  }

  /// Local and remote-tracking branch names (`refs/heads`, `refs/remotes`),
  /// most recently committed first.
  public static func branches(root: String) -> (local: [String], remote: [String]) {
    guard
      case .success(let result) = git(
        [
          "for-each-ref", "--sort=-committerdate", "--format=%(refname)",
          "refs/heads", "refs/remotes",
        ], in: root)
    else { return ([], []) }
    var local: [String] = []
    var remote: [String] = []
    for line in result.stdout.split(separator: "\n") {
      if line.hasPrefix("refs/heads/") {
        local.append(String(line.dropFirst(11)))
      } else if line.hasPrefix("refs/remotes/"), !line.hasSuffix("/HEAD") {
        remote.append(String(line.dropFirst(13)))
      }
    }
    return (local, remote)
  }

  // MARK: - Stash

  public struct StashEntry: Equatable, Sendable {
    public let index: Int
    public let message: String
    public let branch: String?
  }

  public static func stashList(root: String) -> [StashEntry] {
    guard case .success(let result) = git(["stash", "list", "--format=%gd%x00%gs"], in: root)
    else { return [] }
    return result.stdout.split(separator: "\n").compactMap { line in
      let parts = line.split(separator: "\0", maxSplits: 1).map(String.init)
      guard parts.count == 2,
        let open = parts[0].firstIndex(of: "{"), let close = parts[0].firstIndex(of: "}"),
        let index = Int(parts[0][parts[0].index(after: open)..<close])
      else { return nil }
      // "WIP on main: abc123 subject" / "On main: message"
      var branch: String?
      if let colon = parts[1].firstIndex(of: ":") {
        let prefix = parts[1][..<colon]
        branch = prefix.split(separator: " ").last.map(String.init)
      }
      return StashEntry(index: index, message: parts[1], branch: branch)
    }
  }

  public static func stash(
    message: String?, includeUntracked: Bool = true, keepIndex: Bool = false,
    paths: [String] = [], root: String
  ) -> GitResult {
    var args = ["stash", "push"]
    if let message, !message.isEmpty { args += ["-m", message] }
    if includeUntracked { args.append("--include-untracked") }
    if keepIndex { args.append("--keep-index") }
    if !paths.isEmpty { args += ["--"] + paths }
    return void(git(args, in: root))
  }

  public static func stashApply(_ index: Int, pop: Bool, root: String) -> GitResult {
    void(git(["stash", pop ? "pop" : "apply", "--index", "stash@{\(index)}"], in: root))
  }

  public static func stashDrop(_ index: Int, root: String) -> GitResult {
    void(git(["stash", "drop", "stash@{\(index)}"], in: root))
  }

  // MARK: - Remote

  public static func fetch(root: String, onProgress: ((String) -> Void)? = nil) -> GitResult {
    void(git(["fetch", "--prune", "--progress"], in: root, timeout: 600, onOutputLine: onProgress))
  }

  public static func pull(rebase: Bool, root: String, onProgress: ((String) -> Void)? = nil)
    -> GitResult
  {
    void(
      git(
        ["pull", rebase ? "--rebase" : "--ff-only", "--progress"], in: root, timeout: 600,
        onOutputLine: onProgress))
  }

  /// Push the current branch. `setUpstream` publishes it to `remote`.
  public static func push(
    setUpstream: Bool = false, remote: String = "origin", branch: String? = nil,
    forceWithLease: Bool = false, root: String, onProgress: ((String) -> Void)? = nil
  ) -> GitResult {
    var args = ["push", "--progress"]
    if forceWithLease { args.append("--force-with-lease") }
    if setUpstream {
      args += ["--set-upstream", remote]
      if let branch { args.append(branch) }
    }
    return void(git(args, in: root, timeout: 600, onOutputLine: onProgress))
  }

  public static func remotes(root: String) -> [String] {
    guard case .success(let result) = git(["remote"], in: root) else { return [] }
    return result.stdout.split(separator: "\n").map(String.init)
  }

  // MARK: - Operations in progress

  public enum OperationAction: String, Sendable { case `continue`, skip, abort }

  public static func perform(
    _ action: OperationAction, on operation: RepoOperation, root: String
  ) -> GitResult {
    let command: String
    switch operation {
    case .merge: command = "merge"
    case .rebase: command = "rebase"
    case .cherryPick: command = "cherry-pick"
    case .revert: command = "revert"
    case .applyMailbox: command = "am"
    case .bisect:
      return action == .abort
        ? void(git(["bisect", "reset"], in: root))
        : .failure(.invalid("Bisect is driven from the terminal."))
    }
    if action == .skip && !operation.canSkip {
      return .failure(.invalid("\(operation.title) can't skip a step."))
    }
    return void(git([command, "--\(action.rawValue)"], in: root, timeout: 600))
  }

  /// Mark conflicted files resolved (stage them).
  public static func markResolved(paths: [String], root: String) -> GitResult {
    stage(paths: paths, root: root)
  }

  // MARK: - History edits

  /// Resolve conflicted files by taking one side wholesale (`ours` is HEAD,
  /// `theirs` the incoming change) and marking them resolved.
  public static func resolveConflicts(_ paths: [String], takeOurs: Bool, root: String) -> GitResult {
    guard !paths.isEmpty else { return .success(()) }
    if case .failure(let error) = git(
      ["checkout", takeOurs ? "--ours" : "--theirs", "--"] + paths, in: root)
    {
      return .failure(error)
    }
    return void(git(["add", "--"] + paths, in: root))
  }

  public enum ResetMode: String, Sendable { case soft, mixed, hard }

  public static func reset(_ mode: ResetMode, to revision: String, root: String) -> GitResult {
    void(git(["reset", "--\(mode.rawValue)", revision], in: root))
  }

  public static func cherryPick(_ revision: String, root: String) -> GitResult {
    void(git(["cherry-pick", revision], in: root, timeout: 600))
  }

  public static func revert(_ revision: String, root: String) -> GitResult {
    void(git(["revert", "--no-edit", revision], in: root, timeout: 600))
  }

  public static func checkoutDetached(_ revision: String, root: String) -> GitResult {
    void(git(["switch", "--detach", revision], in: root))
  }

  // MARK: - Worktrees

  public struct Worktree: Equatable, Sendable {
    public let path: String
    public let head: String?
    public let branch: String?
    public let isBare: Bool
    public let isDetached: Bool
    public let isLocked: Bool
    public let isPrunable: Bool
  }

  public static func worktrees(root: String) -> [Worktree] {
    guard case .success(let result) = git(["worktree", "list", "--porcelain"], in: root) else {
      return []
    }
    var list: [Worktree] = []
    for block in result.stdout.components(separatedBy: "\n\n") where !block.isEmpty {
      var path: String?
      var head: String?
      var branch: String?
      var bare = false
      var detached = false
      var locked = false
      var prunable = false
      for line in block.split(separator: "\n") {
        if line.hasPrefix("worktree ") { path = String(line.dropFirst(9)) }
        else if line.hasPrefix("HEAD ") { head = String(line.dropFirst(5)) }
        else if line.hasPrefix("branch ") {
          let ref = String(line.dropFirst(7))
          branch = ref.hasPrefix("refs/heads/") ? String(ref.dropFirst(11)) : ref
        } else if line == "bare" { bare = true }
        else if line == "detached" { detached = true }
        else if line.hasPrefix("locked") { locked = true }
        else if line.hasPrefix("prunable") { prunable = true }
      }
      if let path {
        list.append(
          Worktree(
            path: path, head: head, branch: branch, isBare: bare, isDetached: detached,
            isLocked: locked, isPrunable: prunable))
      }
    }
    return list
  }

  /// Add a worktree at `path`. With `newBranch`, create it from `base`
  /// (default HEAD); otherwise check out the existing `branch`.
  public static func addWorktree(
    path: String, branch: String, newBranch: Bool, base: String? = nil, root: String
  ) -> GitResult {
    var args = ["worktree", "add"]
    if newBranch {
      args += ["-b", branch, path]
      if let base { args.append(base) }
    } else {
      args += [path, branch]
    }
    return void(git(args, in: root, timeout: 300))
  }

  public static func removeWorktree(path: String, force: Bool = false, root: String) -> GitResult {
    void(git(["worktree", "remove"] + (force ? ["--force"] : []) + [path], in: root))
  }

  public static func pruneWorktrees(root: String) -> GitResult {
    void(git(["worktree", "prune"], in: root))
  }
}
