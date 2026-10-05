import AppKit
import ImpulseGit
import ImpulseKit

/// What git UI needs from its window: open things, confirm, report.
protocol GitPanelHost: AnyObject {
  var toasts: ToastCenter { get }
  func gitOpenFile(_ absolutePath: String)
  func gitOpenReview(scope: DiffScope, focusPath: String?)
  func gitPresentError(_ error: GitOperationError, title: String)
  /// Ask before something destructive; `completion(true)` to proceed.
  func gitConfirm(
    title: String, message: String, confirmTitle: String, destructive: Bool,
    completion: @escaping (Bool) -> Void)
  /// Coding agents that could take text (for "Send to agent").
  var agentTargets: [AgentSummary] { get }
  /// Type text into an agent's prompt (queued while it's mid-turn).
  func sendToAgent(_ text: String, terminalID: UUID)
}

/// User-level git actions with the UX around them: safety snapshots + Undo
/// toasts for anything destructive, confirmation for file-level discards,
/// plain-English errors.
struct GitActions {
  let repository: GitRepositoryState
  weak var host: GitPanelHost?

  private var root: String { repository.root }

  // MARK: Staging

  func stage(_ changes: [FileChange]) {
    let paths = changes.flatMap { [$0.path] + ($0.oldPath.map { [$0] } ?? []) }
    repository.run { GitOperations.stage(paths: paths, root: $0) } completion: { result, _ in
      report(result, failure: "Couldn't stage")
    }
  }

  func unstage(_ changes: [FileChange]) {
    let paths = changes.flatMap { [$0.path] + ($0.oldPath.map { [$0] } ?? []) }
    let unborn = repository.snapshot?.isUnborn ?? false
    repository.run { GitOperations.unstage(paths: paths, root: $0, isUnborn: unborn) } completion: {
      result, _ in report(result, failure: "Couldn't unstage")
    }
  }

  func stageAll() {
    repository.run { GitOperations.stageAll(root: $0) } completion: { result, _ in
      report(result, failure: "Couldn't stage all changes")
    }
  }

  func unstageAll() {
    let unborn = repository.snapshot?.isUnborn ?? false
    repository.run { GitOperations.unstageAll(root: $0, isUnborn: unborn) } completion: { result, _ in
      report(result, failure: "Couldn't unstage all changes")
    }
  }

  /// Stage/unstage/revert part of a file (review surface).
  func apply(
    _ target: PatchTarget, selection: PatchSelection, change: FileChange,
    expectedHunkIds: [Int: String], options: DiffOptions,
    completion: ((Bool) -> Void)? = nil
  ) {
    let reason = target == .discard ? "Revert changes in \(change.path)" : nil
    repository.run(snapshotReason: reason) { root in
      GitOperations.apply(
        target, selection: selection, path: change.path, oldPath: change.oldPath,
        expectedHunkIds: expectedHunkIds, options: options, root: root)
    } completion: { result, snapshot in
      switch result {
      case .success:
        if target == .discard, let snapshot {
          offerUndo("Reverted changes in \((change.path as NSString).lastPathComponent)",
            snapshot: snapshot, paths: [change.path])
        }
        completion?(true)
      case .failure(let error):
        host?.gitPresentError(error, title: failureTitle(target))
        completion?(false)
      }
    }
  }

  private func failureTitle(_ target: PatchTarget) -> String {
    switch target {
    case .stage: return "Couldn't stage the selection"
    case .unstage: return "Couldn't unstage the selection"
    case .discard: return "Couldn't revert the selection"
    }
  }

  // MARK: Discarding files

  /// Discard working-tree changes (tracked) or move untracked files to the
  /// Trash, after confirming. Undoable.
  func discard(_ changes: [FileChange], includeStaged: Bool = false) {
    guard !changes.isEmpty else { return }
    let names = changes.count == 1 ? "“\((changes[0].path as NSString).lastPathComponent)”" : "\(changes.count) files"
    host?.gitConfirm(
      title: "Discard changes to \(names)?",
      message: includeStaged
        ? "Staged and unstaged changes will be reverted to the last commit. You can undo this right after."
        : "Your changes will be reverted. Untracked files move to the Trash. You can undo this right after.",
      confirmTitle: "Discard", destructive: true
    ) { proceed in
      guard proceed else { return }
      performDiscard(changes, includeStaged: includeStaged)
    }
  }

  private func performDiscard(_ changes: [FileChange], includeStaged: Bool) {
    let untracked = changes.filter { $0.status == .untracked }.map(\.path)
    let tracked = changes.filter { $0.status != .untracked }
    let trackedPaths = tracked.flatMap { [$0.path] + ($0.oldPath.map { [$0] } ?? []) }
    let label = changes.count == 1 ? (changes[0].path as NSString).lastPathComponent : "\(changes.count) files"
    repository.run(snapshotReason: "Discard \(label)") { root in
      if !trackedPaths.isEmpty {
        let result =
          includeStaged
          ? GitOperations.discardAll(paths: trackedPaths, root: root)
          : GitOperations.discardWorkingTree(paths: trackedPaths, root: root)
        if case .failure = result { return result }
      }
      for path in untracked {
        let url = URL(fileURLWithPath: root).appendingPathComponent(path)
        do {
          try FileManager.default.trashItem(at: url, resultingItemURL: nil)
        } catch {
          return .failure(.invalid("Couldn't move \(path) to the Trash: \(error.localizedDescription)"))
        }
      }
      return .success(())
    } completion: { result, snapshot in
      switch result {
      case .success:
        reloadEditors(paths: changes.map(\.path))
        if let snapshot {
          offerUndo("Discarded \(label)", snapshot: snapshot, paths: changes.map(\.path))
        }
      case .failure(let error):
        host?.gitPresentError(error, title: "Couldn't discard \(label)")
      }
    }
  }

  private func offerUndo(_ message: String, snapshot: SafetySnapshot, paths: [String]) {
    let repository = self.repository
    let host = self.host
    host?.toasts.show(
      Toast(
        kind: .success, message: message, actionTitle: "Undo",
        action: {
          repository.run { SafetySnapshots.restore(snapshot, paths: paths, root: $0) } completion: {
            result, _ in
            if case .failure(let error) = result {
              host?.gitPresentError(error, title: "Couldn't undo")
            } else {
              GitActions.notifyEditors(root: repository.root, paths: paths)
            }
          }
        }, lifetime: 10))
  }

  private func reloadEditors(paths: [String]) {
    Self.notifyEditors(root: root, paths: paths)
  }

  static func notifyEditors(root: String, paths: [String]) {
    for path in paths {
      NotificationCenter.default.post(
        name: .impulseReloadEditorFile, object: nil,
        userInfo: ["path": (root as NSString).appendingPathComponent(path)])
    }
  }

  // MARK: Conflicts

  func markResolved(_ changes: [FileChange]) {
    repository.run { GitOperations.markResolved(paths: changes.map(\.path), root: $0) } completion: {
      result, _ in report(result, failure: "Couldn't mark as resolved")
    }
  }

  func perform(_ action: GitOperations.OperationAction, on operation: RepoOperation) {
    let run = {
      repository.run("\(operation.title)…") { GitOperations.perform(action, on: operation, root: $0) }
      completion: { result, _ in
        report(result, failure: "Couldn't \(action.rawValue) — \(operation.title.lowercased())")
      }
    }
    if action == .abort {
      host?.gitConfirm(
        title: "Abort \(operation.title.lowercased())?",
        message: "Changes made during the operation will be lost.", confirmTitle: "Abort",
        destructive: true
      ) { if $0 { run() } }
    } else {
      run()
    }
  }

  // MARK: Commit

  /// Commit what's staged. If nothing is staged but tracked files changed,
  /// offer to commit those (never adds untracked files silently).
  func commit(
    message: String, amend: Bool, signOff: Bool, noVerify: Bool, thenPush: Bool,
    completion: @escaping (Bool) -> Void
  ) {
    guard let snapshot = repository.snapshot else { return }
    let hasStaged = !snapshot.staged.isEmpty
    let hasTracked = !snapshot.unstaged.isEmpty
    if !hasStaged && !amend {
      if hasTracked {
        host?.gitConfirm(
          title: "Nothing is staged",
          message: "Commit all \(snapshot.unstaged.count) changed tracked files? Untracked files aren't included.",
          confirmTitle: "Commit All", destructive: false
        ) { proceed in
          guard proceed else { return completion(false) }
          performCommit(
            message: message, options: .init(
              amend: amend, signOff: signOff, noVerify: noVerify, includeUnstagedTracked: true),
            thenPush: thenPush, completion: completion)
        }
      } else {
        host?.toasts.show(
          Toast(kind: .warning, message: "Stage the files you want to commit first."))
        completion(false)
      }
      return
    }
    performCommit(
      message: message,
      options: .init(amend: amend, signOff: signOff, noVerify: noVerify), thenPush: thenPush,
      completion: completion)
  }

  private func performCommit(
    message: String, options: GitOperations.CommitOptions, thenPush: Bool,
    completion: @escaping (Bool) -> Void
  ) {
    var sha: String?
    repository.run("Committing…") { root in
      switch GitOperations.commit(message: message, options: options, root: root) {
      case .success(let created):
        sha = created
        return .success(())
      case .failure(let error):
        return .failure(error)
      }
    } completion: { result, _ in
      switch result {
      case .success:
        CommitMessageHistory.record(message, root: root)
        let short = sha.map { String($0.prefix(7)) } ?? ""
        let repository = self.repository
        let host = self.host
        host?.toasts.show(
          Toast(
            kind: .success, message: options.amend ? "Amended \(short)" : "Committed \(short)",
            actionTitle: options.amend ? nil : "Uncommit",
            action: options.amend
              ? nil
              : {
                repository.run { GitOperations.uncommit(root: $0) } completion: { result, _ in
                  if case .failure(let error) = result {
                    host?.gitPresentError(error, title: "Couldn't uncommit")
                  }
                }
              }))
        completion(true)
        if thenPush { push() }
      case .failure(let error):
        host?.gitPresentError(error, title: "Couldn't commit")
        completion(false)
      }
    }
  }

  // MARK: Remote

  func fetch() {
    let repository = self.repository
    repository.run("Fetching…") { root in
      GitOperations.fetch(root: root) { repository.reportProgress($0) }
    } completion: { result, _ in
      report(result, failure: "Couldn't fetch")
    }
  }

  func pull(rebase: Bool = false) {
    let repository = self.repository
    repository.run("Pulling…") { root in
      GitOperations.pull(rebase: rebase, root: root) { repository.reportProgress($0) }
    } completion: { result, _ in
      report(result, failure: "Couldn't pull")
    }
  }

  /// Push; publishes the branch to the first remote when it has no upstream.
  func push(forceWithLease: Bool = false) {
    let repository = self.repository
    let snapshot = repository.snapshot
    let needsUpstream = snapshot?.upstream == nil
    let branch = snapshot?.branch
    repository.run(needsUpstream ? "Publishing…" : "Pushing…") { root in
      let remote = GitOperations.remotes(root: root).first ?? "origin"
      return GitOperations.push(
        setUpstream: needsUpstream, remote: remote, branch: branch,
        forceWithLease: forceWithLease, root: root
      ) { repository.reportProgress($0) }
    } completion: { result, _ in
      switch result {
      case .success:
        host?.toasts.show(
          Toast(kind: .success, message: needsUpstream ? "Published \(branch ?? "branch")" : "Pushed"))
      case .failure(let error):
        host?.gitPresentError(error, title: needsUpstream ? "Couldn't publish" : "Couldn't push")
      }
    }
  }

  // MARK: Branches & stash

  /// Switch branches. If local changes would be overwritten, offer to stash
  /// them first (and to pop them afterwards).
  func switchBranch(_ name: String) {
    repository.run("Switching to \(name)…") { GitOperations.switchBranch(name, root: $0) }
    completion: { result, _ in
      guard case .failure(let error) = result else { return }
      if case .cli(let cli) = error, cli.kind == .localChangesWouldBeOverwritten {
        host?.gitConfirm(
          title: "Your changes would be overwritten",
          message: "Stash your uncommitted changes, then switch to \(name)? You can pop the stash afterwards.",
          confirmTitle: "Stash & Switch", destructive: false
        ) { proceed in
          guard proceed else { return }
          stashAndSwitch(name)
        }
      } else {
        host?.gitPresentError(error, title: "Couldn't switch to \(name)")
      }
    }
  }

  private func stashAndSwitch(_ name: String) {
    repository.run("Switching to \(name)…") { root in
      let stashed = GitOperations.stash(
        message: "Impulse: before switching to \(name)", includeUntracked: true, root: root)
      if case .failure = stashed { return stashed }
      return GitOperations.switchBranch(name, root: root)
    } completion: { result, _ in
      switch result {
      case .success:
        let repository = self.repository
        let host = self.host
        host?.toasts.show(
          Toast(
            kind: .success, message: "Stashed your changes and switched to \(name)",
            actionTitle: "Pop Stash",
            action: {
              repository.run { GitOperations.stashApply(0, pop: true, root: $0) } completion: {
                result, _ in
                if case .failure(let error) = result {
                  host?.gitPresentError(error, title: "Couldn't pop the stash")
                }
              }
            }, lifetime: 12))
      case .failure(let error):
        host?.gitPresentError(error, title: "Couldn't switch to \(name)")
      }
    }
  }

  func createBranch(_ name: String) {
    repository.run { GitOperations.createBranch(name, root: $0) } completion: { result, _ in
      if case .success = result {
        host?.toasts.show(Toast(kind: .success, message: "Created and switched to \(name)"))
      }
      report(result, failure: "Couldn't create \(name)")
    }
  }

  func stashAll(message: String? = nil) {
    repository.run("Stashing…") {
      GitOperations.stash(message: message, includeUntracked: true, root: $0)
    } completion: { result, _ in
      if case .success = result {
        host?.toasts.show(Toast(kind: .success, message: "Stashed your changes"))
      }
      report(result, failure: "Couldn't stash")
    }
  }

  func applyStash(_ index: Int, pop: Bool) {
    repository.run { GitOperations.stashApply(index, pop: pop, root: $0) } completion: { result, _ in
      report(result, failure: pop ? "Couldn't pop the stash" : "Couldn't apply the stash")
    }
  }

  func dropStash(_ index: Int) {
    host?.gitConfirm(
      title: "Drop stash@{\(index)}?", message: "The stashed changes will be deleted.",
      confirmTitle: "Drop", destructive: true
    ) { proceed in
      guard proceed else { return }
      repository.run { GitOperations.stashDrop(index, root: $0) } completion: { result, _ in
        report(result, failure: "Couldn't drop the stash")
      }
    }
  }

  // MARK: History

  /// Look at an old commit (detached HEAD).
  func checkout(commit sha: String) {
    repository.run("Checking out \(sha.prefix(7))…") { GitOperations.checkoutDetached(sha, root: $0) }
      completion: { result, _ in
        report(result, failure: "Couldn't check out \(sha.prefix(7))")
      }
  }

  func createBranch(_ name: String, at sha: String) {
    repository.run("Creating \(name)…") {
      GitOperations.createBranch(name, startPoint: sha, checkout: true, root: $0)
    } completion: { result, _ in
      report(result, failure: "Couldn't create \(name)")
    }
  }

  /// Apply a commit on top of HEAD. Conflicts leave the operation open (the
  /// Changes panel offers Continue / Abort).
  func cherryPick(_ sha: String) {
    repository.run("Cherry-picking \(sha.prefix(7))…", snapshotReason: "cherry-pick \(sha.prefix(7))") {
      GitOperations.cherryPick(sha, root: $0)
    } completion: { result, _ in
      report(result, failure: "Cherry-pick stopped")
    }
  }

  func revert(_ sha: String) {
    repository.run("Reverting \(sha.prefix(7))…", snapshotReason: "revert \(sha.prefix(7))") {
      GitOperations.revert(sha, root: $0)
    } completion: { result, _ in
      report(result, failure: "Revert stopped")
    }
  }

  /// Move the current branch to a commit. Hard resets ask first; every reset
  /// can be undone (HEAD and the working tree come back).
  func reset(_ mode: GitOperations.ResetMode, to sha: String) {
    let previousHead = repository.snapshot?.headOid
    let perform = {
      repository.run("Resetting…", snapshotReason: "reset --\(mode.rawValue) \(sha.prefix(7))") {
        GitOperations.reset(mode, to: sha, root: $0)
      } completion: { [repository, host] result, snapshot in
        if case .failure(let error) = result {
          host?.gitPresentError(error, title: "Couldn't reset")
          return
        }
        guard let previousHead else { return }
        host?.toasts.show(
          Toast(
            kind: .success, message: "Reset to \(sha.prefix(7)) (\(mode.rawValue))", actionTitle: "Undo",
            action: {
              repository.run {
                let back = GitOperations.reset(.hard, to: previousHead, root: $0)
                guard case .success = back, let snapshot else { return back }
                return SafetySnapshots.restore(snapshot, root: $0)
              } completion: { result, _ in
                if case .failure(let error) = result { host?.gitPresentError(error, title: "Couldn't undo the reset") }
              }
            }, lifetime: 15))
      }
    }
    guard mode == .hard else { return perform() }
    host?.gitConfirm(
      title: "Reset hard to \(sha.prefix(7))?",
      message: "The branch moves to this commit and uncommitted changes are thrown away. A snapshot is kept so Undo can bring everything back.",
      confirmTitle: "Reset", destructive: true
    ) { confirmed in
      if confirmed { perform() }
    }
  }

  // MARK: Helpers

  private func report(_ result: GitResult, failure title: String) {
    if case .failure(let error) = result {
      host?.gitPresentError(error, title: title)
    }
  }
}

/// Recently used commit messages per repository (↑ in an empty composer).
enum CommitMessageHistory {
  private static func key(_ root: String) -> String { "commitMessages:\(root)" }

  static func messages(root: String) -> [String] {
    UserDefaults.standard.stringArray(forKey: key(root)) ?? []
  }

  static func record(_ message: String, root: String) {
    let trimmed = message.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else { return }
    var list = messages(root: root).filter { $0 != trimmed }
    list.insert(trimmed, at: 0)
    UserDefaults.standard.set(Array(list.prefix(25)), forKey: key(root))
  }
}
