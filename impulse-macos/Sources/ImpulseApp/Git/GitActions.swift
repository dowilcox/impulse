import AppKit
import ImpulseGit
import ImpulseKit

/// What git UI needs from its window: open things, confirm, report.
protocol GitPanelHost: AnyObject {
  var toasts: ToastCenter { get }
  func gitOpenFile(_ absolutePath: String)
  /// Open the file in the editor's diff view (index ↔ working copy).
  func gitOpenDiffEditor(_ absolutePath: String)
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
    repository.run(snapshotReason: reason, requireSnapshot: target == .discard) { root in
      GitOperations.apply(
        target, selection: selection, path: change.path, oldPath: change.oldPath,
        expectedHunkIds: expectedHunkIds, options: options, root: root)
    } completion: { result, snapshot in
      switch result {
      case .success:
        if target == .discard {
          // Open editors show the reverted file, not the stale one.
          reloadEditors(paths: [change.path])
          if let snapshot {
            offerUndo("Reverted changes in \((change.path as NSString).lastPathComponent)",
              snapshot: snapshot, paths: [change.path])
          }
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
    repository.run(snapshotReason: "Discard \(label)", requireSnapshot: true) { root in
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
    if action == .abort, OperationAbort.applies(to: operation) { return abort(operation) }
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

  /// Abort a merge, cherry-pick or revert so it always works: everything
  /// is snapshotted first, and when git refuses (files edited while
  /// resolving), Impulse resets to HEAD and puts back the work from before
  /// the operation. Undo reopens a merge with the files as they were.
  private func abort(_ operation: RepoOperation) {
    let name = operation.title.lowercased()
    host?.gitConfirm(
      title: "Abort \(name)?",
      message: "Your files go back to how they were before it started. Everything as it is now is saved in a snapshot that Undo restores.",
      confirmTitle: "Abort", destructive: true
    ) { confirmed in
      guard confirmed else { return }
      var aborted: OperationAbort.Aborted?
      repository.run("Aborting…") { root in
        switch OperationAbort.abort(operation, root: root) {
        case .success(let result):
          aborted = result
          return .success(())
        case .failure(let error):
          return .failure(error)
        }
      } completion: { result, _ in
        guard case .success = result, let aborted else {
          return report(result, failure: "Couldn't abort — \(name)")
        }
        var detail: String?
        if case .reset(let kept, let exact) = aborted.outcome {
          let work = kept.isEmpty ? "" : " and put back your \(kept.count == 1 ? "file" : "\(kept.count) files") from before it"
          detail = "git couldn't abort, so Impulse reset to HEAD\(work)."
          if !exact, !kept.isEmpty { detail! += " Edits made during it outside its files were kept too." }
        }
        host?.toasts.show(
          Toast(
            kind: .success, message: "Aborted \(name)", detail: detail, actionTitle: aborted.undo == nil ? nil : "Undo",
            action: aborted.undo == nil
              ? nil
              : {
                repository.run("Undoing…") { OperationAbort.undo(aborted, operation: operation, root: $0) } completion: {
                  result, _ in report(result, failure: "Couldn't undo the abort")
                }
              },
            lifetime: 15))
      }
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

  /// Undo the last commit, keeping its changes staged. Asks first when the
  /// commit is already on the upstream (pushing again would need a force).
  func undoLastCommit() {
    let repository = self.repository
    guard let snapshot = repository.snapshot, let head = snapshot.headOid, !snapshot.isUnborn else { return }
    let perform = {
      repository.run { GitOperations.uncommit(root: $0) } completion: { result, _ in
        guard case .success = result else { return report(result, failure: "Couldn't undo the commit") }
        host?.toasts.show(
          Toast(
            kind: .success, message: "Undid \(head.prefix(7)); its changes are staged", actionTitle: "Redo",
            action: {
              repository.run { GitOperations.reset(.soft, to: head, root: $0) } completion: { result, _ in
                if case .failure(let error) = result { host?.gitPresentError(error, title: "Couldn't redo the commit") }
              }
            }, lifetime: 15))
      }
    }
    guard let upstream = snapshot.upstream, snapshot.ahead == 0 else { return perform() }
    host?.gitConfirm(
      title: "Undo a pushed commit?",
      message: "\(head.prefix(7)) is already on \(upstream). Undoing it here means the next push has to force.",
      confirmTitle: "Undo Commit", destructive: true
    ) { confirmed in
      if confirmed { perform() }
    }
  }

  // MARK: Remote

  /// Fetch the branch's remote (or every remote) and say what came in.
  func fetch(allRemotes: Bool = false) {
    let repository = self.repository
    let before = repository.snapshot?.behind ?? 0
    repository.run(allRemotes ? "Fetching all remotes…" : "Fetching…") { root in
      GitOperations.fetch(allRemotes: allRemotes, root: root) { repository.reportProgress($0) }
    } completion: { result, _ in
      repository.lastFetch = Date()
      guard case .success = result else { return report(result, failure: "Couldn't fetch") }
      // The refreshed snapshot says how far behind the branch is now.
      repository.afterNextRefresh { snapshot in
        let behind = snapshot?.behind ?? 0
        let message =
          behind > before
          ? "Fetched: \(behind) commit\(behind == 1 ? "" : "s") to pull" : "Fetched: nothing new for this branch"
        host?.toasts.show(Toast(kind: .success, message: message))
      }
    }
  }

  /// Pull with the strategy from settings (or the one given).
  func pull(mode: GitOperations.PullMode? = nil) {
    let repository = self.repository
    let mode = mode ?? GitOperations.PullMode(rawValue: SettingsStore.shared.settings.gitPullMode) ?? .fastForwardOnly
    var before: String?
    repository.run("Pulling…", snapshotReason: mode == .fastForwardOnly ? nil : "pull --\(mode.rawValue)") { root in
      // Where HEAD is when the pull runs (not when it was asked for).
      before = GitClient.resolveCommit(repoPath: root, revision: "HEAD")
      return GitOperations.pull(mode: mode, root: root) { repository.reportProgress($0) }
    } completion: { result, snapshot in
      repository.lastFetch = Date()
      guard case .success = result else { return report(result, failure: "Couldn't pull") }
      let root = repository.root
      DispatchQueue.global(qos: .userInitiated).async {
        let after = GitClient.resolveCommit(repoPath: root, revision: "HEAD")
        let count = before.flatMap { before in after.flatMap { GitOperations.commitCount(from: before, to: $0, root: root) } }
        DispatchQueue.main.async {
          guard before != after else {
            host?.toasts.show(Toast(kind: .success, message: "Already up to date"))
            return
          }
          let message = count.map { "Pulled \($0) commit\($0 == 1 ? "" : "s")" } ?? "Pulled"
          guard let before, mode != .fastForwardOnly else {
            host?.toasts.show(Toast(kind: .success, message: message))
            return
          }
          // Rebasing or merging rewrote or added commits: offer Undo.
          offerHeadUndo(message, previousHead: before, snapshot: snapshot, failure: "Couldn't undo the pull")
        }
      }
    }
  }

  func pull(rebase: Bool) {
    pull(mode: rebase ? .rebase : .fastForwardOnly)
  }

  /// Push; publishes the branch when it has no upstream (to its remote, else
  /// origin, else the first: `GitOperations.defaultRemote`). `then` runs
  /// after a successful push.
  func push(forceWithLease: Bool = false, then: (() -> Void)? = nil) {
    let repository = self.repository
    let snapshot = repository.snapshot
    let needsUpstream = snapshot?.upstream == nil
    let branch = snapshot?.branch
    let followTags = SettingsStore.shared.settings.gitPushFollowTags
    var remote = "origin"
    let output = OutputLines()
    repository.run(needsUpstream ? "Publishing…" : forceWithLease ? "Force pushing…" : "Pushing…") { root in
      remote = GitOperations.defaultRemote(root: root, branch: branch) ?? "origin"
      return GitOperations.push(
        setUpstream: needsUpstream, remote: remote, branch: branch,
        forceWithLease: forceWithLease, followTags: followTags, root: root
      ) { line in
        output.append(line)
        repository.reportProgress(line)
      }
    } completion: { result, _ in
      switch result {
      case .success:
        let message =
          needsUpstream ? "Published \(branch ?? "branch") to \(remote)" : forceWithLease ? "Force pushed" : "Pushed"
        // What the server said back, such as a link for a merge request.
        let reply = GitServerMessage.parse(output.lines)
        host?.toasts.show(
          Toast(
            kind: .success, message: message,
            detail: reply.map { "\(remote): \($0.linkCaption ?? $0.lines.joined(separator: " "))" },
            actionTitle: reply?.link == nil ? nil : "Open Link",
            action: reply?.link.map { url in { NSWorkspace.shared.open(url) } },
            lifetime: reply == nil ? 6 : 20))
        then?()
      case .failure(let error):
        host?.gitPresentError(error, title: needsUpstream ? "Couldn't publish" : "Couldn't push")
      }
    }
  }

  /// Overwrite the upstream with this branch, after asking. With a lease:
  /// git refuses if the remote moved since the last fetch.
  func forcePush() {
    guard let snapshot = repository.snapshot, let branch = snapshot.branch else {
      host?.toasts.show(Toast(kind: .info, message: "Check out a branch to push."))
      return
    }
    let upstream = snapshot.upstream ?? "the remote"
    host?.gitConfirm(
      title: "Force push \(branch)?",
      message: "\(upstream) is replaced with your \(branch), dropping any commits only it has. It stops if someone else pushed since your last fetch (--force-with-lease).",
      confirmTitle: "Force Push", destructive: true
    ) { confirmed in
      if confirmed { push(forceWithLease: true) }
    }
  }

  // MARK: Tags

  /// Tag a commit; an empty message makes a lightweight tag. `push` sends
  /// it to the branch's remote straight away.
  func createTag(_ name: String, at sha: String = "HEAD", message: String?, push: Bool, force: Bool = false) {
    let repository = self.repository
    // A moved tag's old target, so Undo can put it back.
    var previous: String?
    repository.run("Tagging \(name)…") { root in
      if force { previous = GitOperations.resolveRef("refs/tags/\(name)", root: root) }
      return GitOperations.createTag(name, at: sha, message: message, force: force, root: root)
    } completion: { result, _ in
      switch result {
      case .success:
        if push {
          pushTag(name, created: true, force: force)
        } else {
          host?.toasts.show(
            Toast(
              kind: .success, message: force ? "Moved \(name)" : "Tagged \(name)", actionTitle: "Undo",
              action: {
                repository.run { root in
                  if let previous { return GitOperations.restoreRef("refs/tags/\(name)", to: previous, root: root) }
                  return GitOperations.deleteTag(name, root: root)
                }
              }, lifetime: 10))
        }
      case .failure(.cli(let error)) where error.kind == .tagAlreadyExists && !force:
        host?.gitConfirm(
          title: "\(name) already exists",
          message: "Move it to \(sha == "HEAD" ? "the current commit" : String(sha.prefix(7)))? Anyone who already fetched it keeps the old one.",
          confirmTitle: "Move Tag", destructive: true
        ) { confirmed in
          if confirmed { createTag(name, at: sha, message: message, push: push, force: true) }
        }
      case .failure(let error):
        host?.gitPresentError(error, title: "Couldn't create \(name)")
      }
    }
  }

  func pushTag(_ name: String, created: Bool = false, force: Bool = false) {
    let repository = self.repository
    var remote = "origin"
    repository.run("Pushing \(name)…") { root in
      remote = GitOperations.defaultRemote(root: root) ?? "origin"
      return GitOperations.pushTag(name, remote: remote, force: force, root: root) { repository.reportProgress($0) }
    } completion: { result, _ in
      switch result {
      case .success:
        host?.toasts.show(
          Toast(kind: .success, message: created ? "Tagged and pushed \(name) to \(remote)" : "Pushed \(name) to \(remote)"))
      case .failure(let error):
        host?.gitPresentError(error, title: created ? "Tagged \(name), but couldn't push it" : "Couldn't push \(name)")
      }
    }
  }

  func pushAllTags() {
    let repository = self.repository
    var remote = "origin"
    repository.run("Pushing tags…") { root in
      remote = GitOperations.defaultRemote(root: root) ?? "origin"
      return GitOperations.pushAllTags(remote: remote, root: root) { repository.reportProgress($0) }
    } completion: { result, _ in
      if case .success = result { host?.toasts.show(Toast(kind: .success, message: "Pushed tags to \(remote)")) }
      report(result, failure: "Couldn't push tags")
    }
  }

  /// Delete a local tag. Undo puts it back, annotation included.
  func deleteTag(_ name: String) {
    let repository = self.repository
    var object: String?
    repository.run { root in
      object = GitOperations.resolveRef("refs/tags/\(name)", root: root)
      return GitOperations.deleteTag(name, root: root)
    } completion: { result, _ in
      guard case .success = result else { return report(result, failure: "Couldn't delete \(name)") }
      host?.toasts.show(
        Toast(
          kind: .success, message: "Deleted tag \(name)", actionTitle: object == nil ? nil : "Undo",
          action: object.map { object in
            { repository.run { GitOperations.restoreRef("refs/tags/\(name)", to: object, root: $0) } }
          }, lifetime: 15))
    }
  }

  /// Delete a tag on the remote, after asking (it can't be undone there).
  func deleteRemoteTag(_ name: String) {
    let repository = self.repository
    let remote = GitOperations.defaultRemote(root: repository.root) ?? "origin"
    host?.gitConfirm(
      title: "Delete \(name) from \(remote)?",
      message: "The tag is removed from the remote; your local tag stays. Anyone who already fetched it keeps their copy.",
      confirmTitle: "Delete", destructive: true
    ) { confirmed in
      guard confirmed else { return }
      repository.run("Deleting \(name) from \(remote)…") {
        GitOperations.deleteRemoteTag(name, remote: remote, root: $0)
      } completion: { result, _ in
        if case .success = result { host?.toasts.show(Toast(kind: .success, message: "Deleted \(name) from \(remote)")) }
        report(result, failure: "Couldn't delete \(name) from \(remote)")
      }
    }
  }

  // MARK: Merge & rebase

  /// Merge a branch or commit into the current branch. Undo resets to
  /// where it was; conflicts leave the merge open in the Changes panel. A
  /// branch that a task is still working on asks first, and is merged at
  /// the commit the question named.
  func merge(_ revision: String, label: String? = nil) {
    let root = repository.root
    DispatchQueue.global(qos: .userInitiated).async {
      let task = TaskActivity.find(branch: revision, root: root)
      DispatchQueue.main.async {
        guard var task else { return mergeNow(revision, label: label) }
        guard let question = task.question() else { return mergeNow(revision, label: label) }
        host?.gitConfirm(
          title: question.title, message: question.message, confirmTitle: "Merge \(task.shortCommit)",
          destructive: false
        ) { confirmed in
          if confirmed { mergeNow(task.commit, label: "\(revision) at \(task.shortCommit)") }
        }
      }
    }
  }

  private func mergeNow(_ revision: String, label: String? = nil) {
    let label = label ?? String(revision.prefix(7))
    let into = repository.snapshot?.branch ?? "HEAD"
    var before: String?
    repository.run("Merging \(label)…", snapshotReason: "merge \(label)") { root in
      before = GitClient.resolveCommit(repoPath: root, revision: "HEAD")
      return GitOperations.merge(revision, root: root)
    } completion: { result, snapshot in
      guard case .success = result else { return report(result, failure: "Merge of \(label) stopped") }
      guard let before, headMoved(from: before) else {
        host?.toasts.show(Toast(kind: .info, message: "\(into) already has \(label)"))
        return
      }
      offerHeadUndo("Merged \(label) into \(into)", previousHead: before, snapshot: snapshot, failure: "Couldn't undo the merge")
    }
  }

  /// Rebase the current branch onto a branch or commit.
  func rebase(onto revision: String, label: String? = nil) {
    let label = label ?? String(revision.prefix(7))
    let branch = repository.snapshot?.branch ?? "HEAD"
    var before: String?
    repository.run("Rebasing onto \(label)…", snapshotReason: "rebase onto \(label)") { root in
      before = GitClient.resolveCommit(repoPath: root, revision: "HEAD")
      return GitOperations.rebase(onto: revision, root: root)
    } completion: { result, snapshot in
      guard case .success = result else { return report(result, failure: "Rebase onto \(label) stopped") }
      guard let before, headMoved(from: before) else {
        host?.toasts.show(Toast(kind: .info, message: "\(branch) is already on top of \(label)"))
        return
      }
      offerHeadUndo("Rebased \(branch) onto \(label)", previousHead: before, snapshot: snapshot, failure: "Couldn't undo the rebase")
    }
  }

  // MARK: Remote address

  /// The default remote's URL, as configured (off the main thread); a
  /// toast and nil when there's no remote.
  private func withRemoteURL(_ body: @escaping (String) -> Void) {
    let root = repository.root
    DispatchQueue.global(qos: .userInitiated).async {
      let url = GitOperations.defaultRemote(root: root).flatMap { GitOperations.remoteURL($0, root: root) }
      DispatchQueue.main.async {
        guard let url else {
          host?.toasts.show(Toast(kind: .info, message: "This repository has no remote."))
          return
        }
        body(url)
      }
    }
  }

  /// Open the default remote's address as a web page (`git@host:o/r.git` →
  /// `https://host/o/r`). Impulse knows nothing about the host itself.
  func openRepositoryInBrowser() {
    withRemoteURL { [host] remote in
      guard let url = RemoteWebURL(remote: remote)?.repository else {
        host?.toasts.show(Toast(kind: .info, message: "\(remote) isn't a web address."))
        return
      }
      NSWorkspace.shared.open(url)
    }
  }

  /// Copy the default remote's URL, as git has it.
  func copyRemoteURL() {
    withRemoteURL { [host] remote in
      NSPasteboard.general.clearContents()
      NSPasteboard.general.setString(remote, forType: .string)
      host?.toasts.show(Toast(kind: .success, message: "Copied \(remote)"))
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

  /// Delete a local branch; Undo recreates it. A branch that isn't merged
  /// yet is deleted only after asking (`base` names what it isn't in).
  func deleteBranch(_ name: String, base: String? = nil, force: Bool = false, completion: (() -> Void)? = nil) {
    let repository = self.repository
    let sha = GitClient.resolveCommit(repoPath: repository.root, revision: "refs/heads/\(name)")
    repository.run { GitOperations.deleteBranch(name, force: force, root: $0) } completion: { result, _ in
      completion?()
      switch result {
      case .success:
        host?.toasts.show(
          Toast(
            kind: .success, message: "Deleted \(name)", actionTitle: sha == nil ? nil : "Undo",
            action: sha.map { sha in
              {
                repository.run {
                  GitOperations.createBranch(name, startPoint: sha, checkout: false, root: $0)
                } completion: { _, _ in completion?() }
              }
            }, lifetime: 15))
      case .failure(.cli(let error)) where !force && error.output.contains("not fully merged"):
        // Not merged: say what would be lost, then force.
        host?.gitConfirm(
          title: "\(name) isn't merged",
          message: "Its commits aren't in \(base ?? "the current branch") yet. Delete it anyway? Undo stays available for a few seconds.",
          confirmTitle: "Delete", destructive: true
        ) { confirmed in
          if confirmed { deleteBranch(name, base: base, force: true, completion: completion) }
        }
      case .failure(let error):
        host?.gitPresentError(error, title: "Couldn't delete \(name)")
      }
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
    let root = repository.root
    guard pop, let commit = GitOperations.stashCommit(index, root: root) else {
      repository.run { GitOperations.stashApply(index, pop: pop, root: $0) } completion: { result, _ in
        report(result, failure: pop ? "Couldn't pop the stash" : "Couldn't apply the stash")
      }
      return
    }
    // Popping drops the entry: remember it, and snapshot the working tree,
    // so Undo can put both back.
    let message = GitOperations.stashList(root: root).first { $0.index == index }?.message ?? "Restored stash"
    let paths = GitOperations.stashPaths(commit, root: root)
    repository.run(snapshotReason: "pop stash") {
      GitOperations.stashApply(index, pop: true, root: $0)
    } completion: { [repository, host] result, snapshot in
      guard case .success = result else {
        report(result, failure: "Couldn't pop the stash")
        return
      }
      GitActions.notifyEditors(root: root, paths: paths)
      guard let snapshot else { return }
      host?.toasts.show(
        Toast(
          kind: .success, message: "Popped “\(message)”", actionTitle: "Undo",
          action: {
            repository.run {
              GitOperations.undoStashPop(commit, message: message, snapshot: snapshot, root: $0)
            } completion: { result, _ in
              if case .failure(let error) = result { host?.gitPresentError(error, title: "Couldn't undo the pop") }
              GitActions.notifyEditors(root: root, paths: paths)
            }
          }, lifetime: 10))
    }
  }

  /// Drop a stash. No confirmation: the toast's Undo stores it again.
  func dropStash(_ index: Int) {
    let root = repository.root
    guard let commit = GitOperations.stashCommit(index, root: root) else { return }
    let message = GitOperations.stashList(root: root).first { $0.index == index }?.message ?? "Restored stash"
    repository.run { GitOperations.stashDrop(index, root: $0) } completion: { [repository, host] result, _ in
      guard case .success = result else {
        report(result, failure: "Couldn't drop the stash")
        return
      }
      host?.toasts.show(
        Toast(
          kind: .success, message: "Dropped “\(message)”", actionTitle: "Undo",
          action: {
            repository.run { GitOperations.stashStore(commit, message: message, root: $0) } completion: {
              result, _ in
              if case .failure(let error) = result { host?.gitPresentError(error, title: "Couldn't restore the stash") }
            }
          }, lifetime: 15))
    }
  }

  // MARK: Conflicts

  /// Agents that could take a prompt.
  var agentTargets: [AgentSummary] { host?.agentTargets ?? [] }

  /// Hand the conflict blocks in these files (every conflicted file when
  /// empty) to an agent, or copy the prompt when `terminalID` is nil.
  func askAgentToResolve(_ changes: [FileChange], terminalID: UUID?) {
    let root = repository.root
    let targets = changes.isEmpty ? repository.snapshot?.conflicted ?? [] : changes
    let files = targets.compactMap { change -> (path: String, text: String)? in
      let path = (root as NSString).appendingPathComponent(change.path)
      return (try? String(contentsOfFile: path, encoding: .utf8)).map { (change.path, $0) }
    }
    guard files.contains(where: { !ConflictPrompt.blocks(in: $0.text).isEmpty }) else {
      host?.toasts.show(Toast(kind: .info, message: "No conflict markers left. Mark the files resolved."))
      return
    }
    let operation: String? = {
      switch repository.snapshot?.operation {
      case .merge: return "merge"
      case .rebase: return "rebase"
      case .cherryPick: return "cherry-pick"
      case .revert: return "revert"
      default: return nil
      }
    }()
    let prompt = ConflictPrompt.make(files: files, operation: operation)
    if let terminalID {
      host?.sendToAgent(prompt, terminalID: terminalID)
    } else {
      NSPasteboard.general.clearContents()
      NSPasteboard.general.setString(prompt, forType: .string)
      host?.toasts.show(Toast(kind: .success, message: "Copied a prompt to resolve the conflicts"))
    }
  }

  /// Take one side of every conflict in these files (snapshot first). Undo
  /// puts the files back in conflict, with any edits made to them before.
  func resolve(_ changes: [FileChange], takeOurs: Bool) {
    let paths = changes.map(\.path)
    var point: GitOperations.ConflictPoint?
    repository.run(
      takeOurs ? "Keeping current…" : "Taking incoming…",
      snapshotReason: takeOurs ? "keep current side" : "take incoming side", requireSnapshot: true
    ) { root in
      point = GitOperations.conflictPoint(root: root)
      return GitOperations.resolveConflicts(paths, takeOurs: takeOurs, root: root)
    } completion: { [repository, host] result, snapshot in
      if case .failure(let error) = result {
        host?.gitPresentError(error, title: "Couldn't resolve the conflict")
        return
      }
      GitActions.notifyEditors(root: repository.root, paths: paths)
      // Undo refuses once the merge/rebase has moved on from `point`.
      guard let snapshot, let point else { return }
      let names = paths.count == 1 ? (paths[0] as NSString).lastPathComponent : "\(paths.count) files"
      host?.toasts.show(
        Toast(
          kind: .success, message: takeOurs ? "Kept current in \(names)" : "Took incoming in \(names)",
          actionTitle: "Undo",
          action: {
            repository.run(snapshotReason: "before undo") { root in
              let reopened = GitOperations.reopenConflicts(paths, at: point, root: root)
              guard case .success = reopened else { return reopened }
              // The conflict as it was, edits included (the snapshot has no
              // index: it can't be recorded while files are in conflict).
              return SafetySnapshots.restore(snapshot, paths: paths, root: root)
            } completion: { result, _ in
              if case .failure(let error) = result { host?.gitPresentError(error, title: "Couldn't undo") }
              GitActions.notifyEditors(root: repository.root, paths: paths)
            }
          }, lifetime: 15))
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
  /// Changes panel offers Continue / Abort); otherwise Undo takes the new
  /// commit back off.
  func cherryPick(_ sha: String) {
    var before: String?
    repository.run("Cherry-picking \(sha.prefix(7))…", snapshotReason: "cherry-pick \(sha.prefix(7))") { root in
      before = GitClient.resolveCommit(repoPath: root, revision: "HEAD")
      return GitOperations.cherryPick(sha, root: root)
    } completion: { result, snapshot in
      guard case .success = result else { return report(result, failure: "Cherry-pick stopped") }
      guard let before, headMoved(from: before) else { return }
      offerHeadUndo(
        "Cherry-picked \(sha.prefix(7))", previousHead: before, snapshot: snapshot,
        failure: "Couldn't undo the cherry-pick")
    }
  }

  /// A new commit undoing `sha`; Undo takes it back off.
  func revert(_ sha: String) {
    var before: String?
    repository.run("Reverting \(sha.prefix(7))…", snapshotReason: "revert \(sha.prefix(7))") { root in
      before = GitClient.resolveCommit(repoPath: root, revision: "HEAD")
      return GitOperations.revert(sha, root: root)
    } completion: { result, snapshot in
      guard case .success = result else { return report(result, failure: "Revert stopped") }
      guard let before, headMoved(from: before) else { return }
      offerHeadUndo(
        "Reverted \(sha.prefix(7))", previousHead: before, snapshot: snapshot, failure: "Couldn't undo the revert")
    }
  }

  /// Move the current branch to a commit. Hard resets ask first; every reset
  /// can be undone (HEAD and the working tree come back).
  func reset(_ mode: GitOperations.ResetMode, to sha: String) {
    let perform = {
      var previousHead: String?
      repository.run(
        "Resetting…", snapshotReason: "reset --\(mode.rawValue) \(sha.prefix(7))", requireSnapshot: mode == .hard
      ) { root in
        previousHead = GitClient.resolveCommit(repoPath: root, revision: "HEAD")
        return GitOperations.reset(mode, to: sha, root: root)
      } completion: { result, snapshot in
        if case .failure(let error) = result {
          host?.gitPresentError(error, title: "Couldn't reset")
          return
        }
        guard let previousHead else { return }
        offerHeadUndo(
          "Reset to \(sha.prefix(7)) (\(mode.rawValue))", previousHead: previousHead, snapshot: snapshot,
          failure: "Couldn't undo the reset")
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

  private func headMoved(from previous: String) -> Bool {
    GitClient.resolveCommit(repoPath: repository.root, revision: "HEAD") != previous
  }

  /// A success toast whose Undo moves the branch back to `previousHead` and
  /// restores the working tree from the safety snapshot. Undo snapshots the
  /// current state first (edits made since aren't thrown away unrecorded);
  /// without the operation's snapshot it moves HEAD with `reset --keep`,
  /// which keeps uncommitted work, instead of `--hard`.
  private func offerHeadUndo(_ message: String, previousHead: String, snapshot: SafetySnapshot?, failure: String) {
    let repository = self.repository
    host?.toasts.show(
      Toast(
        kind: .success, message: message, actionTitle: "Undo",
        action: { [host] in
          let undo: (String) -> GitResult = { root in
            guard let snapshot else { return GitOperations.reset(.keep, to: previousHead, root: root) }
            let back = GitOperations.reset(.hard, to: previousHead, root: root)
            guard case .success = back else { return back }
            return SafetySnapshots.restore(snapshot, root: root)
          }
          repository.run(snapshotReason: "before undo", requireSnapshot: snapshot != nil, undo) { result, _ in
            if case .failure(let error) = result { host?.gitPresentError(error, title: failure) }
          }
        }, lifetime: 15))
  }

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

/// Lines a git command printed, collected from its progress callback.
private final class OutputLines: @unchecked Sendable {
  private let lock = NSLock()
  private var collected: [String] = []

  func append(_ line: String) {
    lock.lock()
    collected.append(line)
    lock.unlock()
  }

  var lines: [String] {
    lock.lock()
    defer { lock.unlock() }
    return collected
  }
}
