import AppKit
import SwiftUI
import ImpulseGit
import ImpulseKit

// Removing tasks' folders and bringing them back: the mechanics Archive Task…
// and Archive Merged Tasks… share. Uncommitted work goes into a safety
// snapshot first, the archive script runs, Impulse's own files for the task
// go too, and everything needed to undo it is kept.

/// A task whose folder was removed, with what Undo needs.
struct ArchivedTask {
  let root: String
  let branch: String
  let mainRoot: String
  let snapshot: SafetySnapshot?
  let record: TaskRecord?
  /// Set when its branch was deleted too: the commit to recreate it at.
  var deletedBranchCommit: String?
}

extension MainWindowController {
  /// Remove a task's folder (off the main thread). `archiveScript` runs
  /// first; a failure there doesn't stop it (`scriptFailed`). Uncommitted
  /// work is snapshotted first, and a task whose snapshot can't be made
  /// isn't removed.
  static func removeTask(
    root: String, branch: String, dirty: Bool, archiveScript: String?
  ) -> (result: Result<ArchivedTask, GitOperationError>, scriptFailed: Bool) {
    var scriptFailed = false
    if let archiveScript {
      scriptFailed = !runScript(archiveScript, in: root, extra: TaskRegistryStore.identity(forDirectory: root))
    }
    // Run git from the main checkout: the worktree is about to vanish.
    let mainRoot =
      GitClient.commonGitDirectory(forPath: root).map { ($0 as NSString).deletingLastPathComponent } ?? root
    // Look again: closing the workspace may have just saved files ("Save &
    // Close"), which then belong in the snapshot too.
    let dirty = dirty || (GitClient.snapshot(forPath: root)?.changedFileCount ?? 0) > 0
    var saved: SafetySnapshot?
    if dirty {
      switch SafetySnapshots.create(reason: "archive \(branch)", root: root) {
      case .success(let snapshot):
        saved = snapshot
      case .failure(let error):
        return (
          .failure(.invalid("The task wasn't archived: its uncommitted files couldn't be saved first. \(error.message)")),
          scriptFailed
        )
      }
    }
    if case .failure(let error) = GitOperations.removeWorktree(path: root, force: dirty, root: mainRoot) {
      return (.failure(error), scriptFailed)
    }
    let record = TaskRegistryStore.remove(path: root, root: mainRoot)
    // Impulse's own files for it (a Compose override) go with the folder.
    if let common = GitClient.commonGitDirectory(forPath: mainRoot) {
      try? FileManager.default.removeItem(atPath: taskFolder((root as NSString).lastPathComponent, common: common))
    }
    return (.success(ArchivedTask(root: root, branch: branch, mainRoot: mainRoot, snapshot: saved, record: record)), scriptFailed)
  }

  /// Bring an archived task back (off the main thread): its branch if it
  /// was deleted, its folder, its uncommitted files and its record.
  static func restoreArchived(_ task: ArchivedTask) -> GitResult {
    if let commit = task.deletedBranchCommit,
      case .failure(let error) = GitOperations.createBranch(task.branch, startPoint: commit, checkout: false, root: task.mainRoot)
    {
      return .failure(error)
    }
    let added = GitOperations.addWorktree(path: task.root, branch: task.branch, newBranch: false, root: task.mainRoot)
    if case .failure = added { return added }
    if let snapshot = task.snapshot { _ = SafetySnapshots.restore(snapshot, root: task.root) }
    if let record = task.record { TaskRegistryStore.restore(record, root: task.mainRoot) }
    return .success(())
  }
}

// MARK: - Archive Merged Tasks…

@Observable
final class MergedTasksModel {
  struct Row: Identifiable {
    var id: String { path }
    let path: String
    let branch: String
    let uncommitted: Int
    /// The remote its branch is published to, if any.
    let remote: String?
    var isOn = true
  }

  var rows: [Row] = []
  var isLoading = true
  var deleteBranches = false
  var deleteRemoteBranches = false
  var remoteNames: [String] { Array(Set(rows.filter(\.isOn).compactMap(\.remote))).sorted() }
}

struct MergedTasksSheet: View {
  @Environment(\.chrome) private var chrome
  @Bindable var model: MergedTasksModel
  let onCancel: () -> Void
  let onArchive: () -> Void

  var body: some View {
    VStack(alignment: .leading, spacing: 12) {
      Text("Archive Merged Tasks").font(ChromeFont.ui(15, weight: .semibold)).foregroundStyle(chrome.text)
      Text("These tasks' branches are merged into their base. Archiving removes their folders; uncommitted files are saved, and Undo brings it all back.")
        .font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textSecondary).fixedSize(horizontal: false, vertical: true)
      if model.isLoading {
        ProgressView().controlSize(.small)
      } else if model.rows.isEmpty {
        Text("No task's branch is merged.").font(ChromeFont.ui(12)).foregroundStyle(chrome.textTertiary)
      }
      ForEach($model.rows) { $row in
        Toggle(isOn: $row.isOn) {
          HStack(spacing: 6) {
            Text((row.path as NSString).lastPathComponent).font(ChromeFont.ui(12, weight: .medium)).foregroundStyle(chrome.text)
            if row.branch != (row.path as NSString).lastPathComponent {
              Text(row.branch).font(ChromeFont.mono(11)).foregroundStyle(chrome.textTertiary)
            }
            if row.uncommitted > 0 {
              Text("\(row.uncommitted) uncommitted").font(ChromeFont.ui(11)).foregroundStyle(chrome.warning)
            }
          }
        }
        .toggleStyle(.checkbox)
      }
      if !model.rows.isEmpty {
        Divider()
        Toggle(isOn: $model.deleteBranches) {
          Text("Delete their branches too").font(ChromeFont.ui(12)).foregroundStyle(chrome.text)
        }
        .toggleStyle(.checkbox)
        if !model.remoteNames.isEmpty {
          Toggle(isOn: $model.deleteRemoteBranches) {
            Text("…and on \(model.remoteNames.joined(separator: ", ")) (Undo can't bring those back)")
              .font(ChromeFont.ui(12)).foregroundStyle(chrome.text)
          }
          .toggleStyle(.checkbox)
          .disabled(!model.deleteBranches)
          .padding(.leading, 18)
        }
      }
      HStack {
        Spacer()
        ChromeButton(title: "Cancel", kind: .secondary) { onCancel() }.keyboardShortcut(.cancelAction)
        let count = model.rows.filter(\.isOn).count
        ChromeButton(title: count == 1 ? "Archive 1 Task" : "Archive \(count) Tasks", icon: .archive, kind: .primary) {
          onArchive()
        }
        .disabled(count == 0)
        .keyboardShortcut(.defaultAction)
      }
    }
    .padding(20)
    .frame(width: 460)
    .background(chrome.overlay)
  }
}

extension MainWindowController {
  /// "Archive Merged Tasks…": the repository's tasks whose branch is merged
  /// into their base, archived together with one Undo.
  func presentArchiveMergedTasks(from workspaceID: UUID) {
    guard let window, let root = tabManager.workspace(workspaceID)?.root else { return }
    let main = Self.mainCheckoutRoot(of: root)
    let model = MergedTasksModel()
    DispatchQueue.global(qos: .userInitiated).async {
      let merged = OverlapMonitor.mergedTasks(root: main)
      let registry = TaskRegistryStore.registry(root: main)
      let rows = (registry?.tasks ?? []).filter { merged.contains(TaskRegistry.canonical($0.path)) }.map { task in
        MergedTasksModel.Row(
          path: task.path, branch: task.branch, uncommitted: GitClient.snapshot(forPath: task.path)?.changedFileCount ?? 0,
          remote: GitClient.snapshot(forPath: task.path)?.upstream.flatMap { upstream in
            GitOperations.remotes(root: main).first { upstream.hasPrefix("\($0)/") }
          })
      }
      DispatchQueue.main.async {
        model.rows = rows
        model.isLoading = false
      }
    }
    let palette = windowModel.palette
    let sheet = NSWindow.themedSheet(palette: palette)
    window.beginThemedSheet(
      sheet, palette: palette,
      content: MergedTasksSheet(
        model: model,
        onCancel: { [weak window, weak sheet] in if let sheet { window?.endSheet(sheet) } },
        onArchive: { [weak self, weak window, weak sheet] in
          if let sheet { window?.endSheet(sheet) }
          self?.archiveTasks(
            model.rows.filter(\.isOn), deleteBranches: model.deleteBranches,
            deleteRemote: model.deleteBranches && model.deleteRemoteBranches, mainRoot: main)
        }))
  }

  /// Close the tasks' workspaces (each asks about unsaved files and running
  /// processes; one that's cancelled is skipped), then archive them.
  private func archiveTasks(
    _ rows: [MergedTasksModel.Row], deleteBranches: Bool, deleteRemote: Bool, mainRoot: String
  ) {
    var remaining = rows
    var ready: [MergedTasksModel.Row] = []
    func next() {
      guard !remaining.isEmpty else { return removeAll(ready) }
      let row = remaining.removeFirst()
      let path = TaskRegistry.canonical(row.path)
      guard let workspace = tabManager.workspaces.first(where: { TaskRegistry.canonical($0.root) == path }) else {
        ready.append(row)
        return next()
      }
      requestCloseWorkspace(
        workspace.id, recordForUndo: false, then: { ready.append(row); next() }, cancelled: { next() })
    }
    func removeAll(_ rows: [MergedTasksModel.Row]) {
      let scripts = rows.map { alreadyTrustedProjectConfig(root: $0.path)?.archiveScript }
      DispatchQueue.global(qos: .userInitiated).async { [weak self] in
        var archived: [ArchivedTask] = []
        var failures: [String] = []
        for (row, script) in zip(rows, scripts) {
          let removed = Self.removeTask(root: row.path, branch: row.branch, dirty: row.uncommitted > 0, archiveScript: script)
          if removed.scriptFailed { failures.append("\(row.branch): the archive script failed; archived anyway.") }
          switch removed.result {
          case .failure(let error):
            failures.append("\(row.branch): \(error.message)")
          case .success(var task):
            if deleteBranches, let commit = GitClient.resolveCommit(repoPath: mainRoot, revision: "refs/heads/\(row.branch)"),
              case .success = GitOperations.deleteBranch(row.branch, force: true, root: mainRoot)
            {
              task.deletedBranchCommit = commit
              if deleteRemote, let remote = row.remote {
                _ = GitOperations.deleteRemoteBranch(row.branch, remote: remote, root: mainRoot)
              }
            }
            archived.append(task)
          }
        }
        DispatchQueue.main.async {
          guard let self else { return }
          for failure in failures { self.toasts.show(Toast(kind: .warning, message: failure, lifetime: 12)) }
          guard !archived.isEmpty else { return }
          let count = archived.count
          self.toasts.show(
            Toast(
              kind: .success,
              message: "Archived \(count == 1 ? archived[0].branch : "\(count) tasks")"
                + (deleteBranches ? " and deleted \(count == 1 ? "its branch" : "their branches")" : ""),
              actionTitle: "Undo", action: { [weak self] in self?.restoreTasks(archived) }, lifetime: 15))
        }
      }
    }
    next()
  }
}
