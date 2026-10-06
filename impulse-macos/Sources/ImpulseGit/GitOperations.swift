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

  public struct BranchInfo: Equatable, Sendable {
    public let name: String
    public let upstream: String?
    public let ahead: Int
    public let behind: Int
    /// The upstream branch was deleted on the remote.
    public let upstreamGone: Bool
    public let lastCommit: Date
    public let subject: String
    public let isCurrent: Bool
    /// Fully merged into the base branch (safe to delete).
    public let isMerged: Bool
  }

  /// Local branches with their tracking state, most recently committed
  /// first. `base` decides "merged" (the default branch).
  public static func branchDetails(root: String, base: String?) -> [BranchInfo] {
    guard
      case .success(let result) = git(
        [
          "for-each-ref", "--sort=-committerdate",
          "--format=%(refname:short)%1f%(upstream:short)%1f%(upstream:track)%1f%(committerdate:unix)%1f%(subject)%1f%(HEAD)",
          "refs/heads",
        ], in: root)
    else { return [] }
    var merged = Set<String>()
    if let base,
      case .success(let list) = git(["branch", "--merged", base, "--format=%(refname:short)"], in: root)
    {
      merged = Set(list.stdout.split(separator: "\n").map(String.init))
    }
    return result.stdout.split(separator: "\n").compactMap { line in
      let fields = line.components(separatedBy: "\u{1f}")
      guard fields.count >= 6 else { return nil }
      let track = fields[2]
      func count(_ label: String) -> Int {
        guard let range = track.range(of: label + " ") else { return 0 }
        return Int(track[range.upperBound...].prefix { $0.isNumber }) ?? 0
      }
      return BranchInfo(
        name: fields[0], upstream: fields[1].isEmpty ? nil : fields[1],
        ahead: count("ahead"), behind: count("behind"), upstreamGone: track.contains("gone"),
        lastCommit: Date(timeIntervalSince1970: Double(fields[3]) ?? 0), subject: fields[4],
        isCurrent: fields[5] == "*", isMerged: merged.contains(fields[0]) && fields[0] != base)
    }
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

  /// The commit behind `stash@{index}` (kept so a drop or pop can be undone).
  public static func stashCommit(_ index: Int, root: String) -> String? {
    guard case .success(let result) = git(["rev-parse", "--verify", "-q", "stash@{\(index)}"], in: root)
    else { return nil }
    let sha = result.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
    return sha.isEmpty ? nil : sha
  }

  /// Put a dropped stash commit back on top of the stash list.
  public static func stashStore(_ commit: String, message: String, root: String) -> GitResult {
    void(git(["stash", "store", "-m", message, commit], in: root))
  }

  /// Undo `git stash pop`: put the stash's paths back the way `snapshot`
  /// (taken just before the pop) had them, remove what the pop created, and
  /// store the stash entry again. Other files are left alone.
  public static func undoStashPop(
    _ commit: String, message: String, snapshot: SafetySnapshot, root: String
  ) -> GitResult {
    let paths = stashPaths(commit, root: root)
    var existed = Set<String>()
    if !paths.isEmpty,
      case .success(let result) = git(
        ["ls-tree", "-r", "-z", "--name-only", snapshot.commit, "--"] + paths, in: root)
    {
      existed = Set(result.stdout.split(separator: "\0").map(String.init))
    }
    for path in paths where !existed.contains(path) {
      try? FileManager.default.removeItem(atPath: (root as NSString).appendingPathComponent(path))
      _ = git(["rm", "--cached", "-q", "--ignore-unmatch", "--", path], in: root)
    }
    if !existed.isEmpty {
      let restored = SafetySnapshots.restore(snapshot, paths: paths.filter(existed.contains), root: root)
      if case .failure(let error) = restored { return .failure(error) }
    }
    return stashStore(commit, message: message, root: root)
  }

  /// Every path a stash touches: tracked changes plus untracked files.
  public static func stashPaths(_ commit: String, root: String) -> [String] {
    var paths: [String] = []
    if case .success(let result) = git(
      ["diff-tree", "-r", "-z", "--name-only", "--no-commit-id", "\(commit)^1", commit], in: root)
    {
      paths = result.stdout.split(separator: "\0").map(String.init)
    }
    return paths + stashUntrackedPaths(commit, root: root)
  }

  /// Untracked files a stash carries (its third parent), relative to `root`.
  public static func stashUntrackedPaths(_ commit: String, root: String) -> [String] {
    guard case .success(let result) = git(["ls-tree", "-r", "-z", "--name-only", "\(commit)^3"], in: root)
    else { return [] }
    return result.stdout.split(separator: "\0").map(String.init)
  }

  // MARK: - Remote

  /// Fetch the default remote, or every remote. Deleted remote branches are
  /// pruned. `timeout` is shorter for quiet background fetches.
  public static func fetch(
    allRemotes: Bool = false, root: String, timeout: TimeInterval = 600, onProgress: ((String) -> Void)? = nil
  ) -> GitResult {
    var args = ["fetch", "--prune", "--progress"]
    if allRemotes { args.append("--all") }
    return void(git(args, in: root, timeout: timeout, onOutputLine: onProgress))
  }

  /// How `pull` brings in upstream commits.
  public enum PullMode: String, Sendable, CaseIterable {
    /// Only when the branch hasn't diverged (never creates a merge commit).
    case fastForwardOnly = "ff-only"
    /// Replay local commits on top of the upstream.
    case rebase
    /// Merge the upstream in (a merge commit when they diverged).
    case merge
  }

  public static func pull(mode: PullMode, root: String, onProgress: ((String) -> Void)? = nil) -> GitResult {
    let flag: String
    switch mode {
    case .fastForwardOnly: flag = "--ff-only"
    case .rebase: flag = "--rebase"
    case .merge: flag = "--no-rebase"
    }
    return void(git(["pull", flag, "--no-edit", "--progress"], in: root, timeout: 600, onOutputLine: onProgress))
  }

  public static func pull(rebase: Bool, root: String, onProgress: ((String) -> Void)? = nil)
    -> GitResult
  {
    pull(mode: rebase ? .rebase : .fastForwardOnly, root: root, onProgress: onProgress)
  }

  /// Push the current branch. `setUpstream` publishes it to `remote`;
  /// `followTags` also pushes annotated tags on the pushed commits.
  public static func push(
    setUpstream: Bool = false, remote: String = "origin", branch: String? = nil,
    forceWithLease: Bool = false, followTags: Bool = false, root: String,
    onProgress: ((String) -> Void)? = nil
  ) -> GitResult {
    var args = ["push", "--progress"]
    if forceWithLease { args.append("--force-with-lease") }
    if followTags { args.append("--follow-tags") }
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

  /// The checked-out branch's name (nil when detached or unborn-less).
  public static func currentBranch(root: String) -> String? {
    guard case .success(let result) = git(["symbolic-ref", "--quiet", "--short", "HEAD"], in: root) else { return nil }
    let name = result.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
    return name.isEmpty ? nil : name
  }

  /// The remote to push tags to and browse: the current branch's remote,
  /// else `origin`, else the first one.
  public static func defaultRemote(root: String) -> String? {
    let remotes = remotes(root: root)
    if case .success(let head) = git(["symbolic-ref", "--quiet", "--short", "HEAD"], in: root),
      case .success(let config) = git(
        ["config", "--get", "branch.\(head.stdout.trimmingCharacters(in: .whitespacesAndNewlines)).remote"],
        in: root),
      case let remote = config.stdout.trimmingCharacters(in: .whitespacesAndNewlines),
      remotes.contains(remote)
    {
      return remote
    }
    return remotes.contains("origin") ? "origin" : remotes.first
  }

  /// The URL `remote` fetches from.
  public static func remoteURL(_ remote: String, root: String) -> String? {
    guard case .success(let result) = git(["remote", "get-url", remote], in: root) else { return nil }
    let url = result.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
    return url.isEmpty ? nil : url
  }

  // MARK: - Tags

  public struct TagInfo: Equatable, Sendable {
    public let name: String
    /// The commit it points at.
    public let commit: String
    /// Annotated tags carry a message, tagger and date of their own.
    public let isAnnotated: Bool
    /// Tagger date (annotated) or the commit's date (lightweight).
    public let date: Date?
    /// The annotation's first line, or the commit's subject.
    public let subject: String
  }

  /// Tag names, newest first.
  public static func tags(root: String) -> [String] {
    guard case .success(let result) = git(["tag", "--sort=-creatordate"], in: root) else { return [] }
    return result.stdout.split(separator: "\n").map(String.init)
  }

  /// Every tag with what it points at, newest first.
  public static func tagDetails(root: String) -> [TagInfo] {
    let format = "%(refname:short)%1f%(objecttype)%1f%(objectname)%1f%(*objectname)%1f%(creatordate:unix)%1f%(contents:subject)%1e"
    guard
      case .success(let result) = git(
        ["for-each-ref", "--sort=-creatordate", "--format=\(format)", "refs/tags"], in: root)
    else { return [] }
    return result.stdout.split(separator: "\u{1e}").compactMap { record in
      let fields = record.trimmingCharacters(in: .newlines).split(separator: "\u{1f}", omittingEmptySubsequences: false)
        .map(String.init)
      guard fields.count >= 6, !fields[0].isEmpty else { return nil }
      let annotated = fields[1] == "tag"
      return TagInfo(
        name: fields[0], commit: annotated && !fields[3].isEmpty ? fields[3] : fields[2], isAnnotated: annotated,
        date: TimeInterval(fields[4]).map { Date(timeIntervalSince1970: $0) }, subject: fields[5])
    }
  }

  /// Tag `revision`. With a message the tag is annotated (it records who
  /// tagged it, when, and why); without one it is a lightweight name.
  public static func createTag(
    _ name: String, at revision: String = "HEAD", message: String? = nil, force: Bool = false, root: String
  ) -> GitResult {
    guard GitRefName.isValid(name) else { return .failure(.invalid("“\(name)” isn't a valid tag name.")) }
    var args = ["tag"]
    if force { args.append("--force") }
    let message = message?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    if !message.isEmpty {
      args += ["--annotate", "--file=-", name, revision]
      return void(git(args, in: root, stdin: Data((message + "\n").utf8)))
    }
    return void(git(args + [name, revision], in: root))
  }

  public static func deleteTag(_ name: String, root: String) -> GitResult {
    void(git(["tag", "--delete", name], in: root))
  }

  /// The object a ref points at, unpeeled (an annotated tag's tag object).
  public static func resolveRef(_ ref: String, root: String) -> String? {
    guard case .success(let result) = git(["rev-parse", "--verify", "--quiet", ref], in: root) else { return nil }
    let sha = result.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
    return sha.isEmpty ? nil : sha
  }

  /// Point a ref at an object again (undoing a delete while the object is
  /// still in the repository).
  public static func restoreRef(_ ref: String, to object: String, root: String) -> GitResult {
    void(git(["update-ref", ref, object], in: root))
  }

  /// Commits in `to` that aren't in `from` (`git rev-list --count from..to`).
  public static func commitCount(from: String, to: String, root: String) -> Int? {
    guard case .success(let result) = git(["rev-list", "--count", "\(from)..\(to)"], in: root) else { return nil }
    return Int(result.stdout.trimmingCharacters(in: .whitespacesAndNewlines))
  }

  /// Push one tag to `remote`.
  public static func pushTag(
    _ name: String, remote: String, root: String, onProgress: ((String) -> Void)? = nil
  ) -> GitResult {
    void(git(["push", "--progress", remote, "refs/tags/\(name)"], in: root, timeout: 600, onOutputLine: onProgress))
  }

  /// Push every local tag to `remote`.
  public static func pushAllTags(
    remote: String, root: String, onProgress: ((String) -> Void)? = nil
  ) -> GitResult {
    void(git(["push", "--progress", "--tags", remote], in: root, timeout: 600, onOutputLine: onProgress))
  }

  /// Delete a tag on `remote` (the local tag is left alone).
  public static func deleteRemoteTag(_ name: String, remote: String, root: String) -> GitResult {
    void(git(["push", remote, "--delete", "refs/tags/\(name)"], in: root, timeout: 600))
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

  /// Merge `revision` into the current branch (a fast-forward when
  /// possible). Conflicts leave the merge open for Continue / Abort.
  public static func merge(_ revision: String, noFastForward: Bool = false, root: String) -> GitResult {
    var args = ["merge", "--no-edit"]
    if noFastForward { args.append("--no-ff") }
    return void(git(args + [revision], in: root, timeout: 600))
  }

  /// Replay the current branch's own commits on top of `revision`.
  /// Conflicts leave the rebase open for Continue / Skip / Abort.
  public static func rebase(onto revision: String, root: String) -> GitResult {
    void(git(["rebase", revision], in: root, timeout: 600))
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
