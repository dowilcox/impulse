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
  /// The folder was trusted in its own right (Undo trusts it again).
  var wasTrusted = false
  /// Set when its branch was deleted too: the commit to recreate it at.
  var deletedBranchCommit: String?
}

extension MainWindowController {
  /// Remove a task's folder (off the main thread). Uncommitted work is
  /// snapshotted first, and a task whose snapshot can't be made isn't
  /// removed; then `archiveScript` runs, and a failure there doesn't stop
  /// it (`scriptFailed`). A task holding a repository of its own isn't
  /// removed either. The folder's trust goes with it, so a task at the same
  /// path later (another branch's) starts out restricted.
  static func removeTask(
    root: String, branch: String, dirty: Bool, archiveScript: String?
  ) -> (result: Result<ArchivedTask, GitOperationError>, scriptFailed: Bool) {
    // Run git from the main checkout: the worktree is about to vanish.
    let mainRoot =
      GitClient.commonGitDirectory(forPath: root).map { ($0 as NSString).deletingLastPathComponent } ?? root
    let trusted = Trust.shared.isTrusted(root)
    // A snapshot records a repository inside the task (a clone, a
    // submodule) only by its commit id: removing the folder would lose its
    // files and history. Not knowing counts as having one.
    guard let nested = GitOperations.nestedRepositories(root: root, trusted: trusted) else {
      return (.failure(.invalid("The task wasn't archived. \(nestedRepositoryMessage(nil))")), false)
    }
    if !nested.isEmpty {
      return (.failure(.invalid("The task wasn't archived. \(nestedRepositoryMessage(nested))")), false)
    }
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
          false
        )
      }
    }
    // After the snapshot, so Undo brings back what the script changes too.
    var scriptFailed = false
    if let archiveScript {
      scriptFailed = !runScript(archiveScript, in: root, extra: TaskRegistryStore.identity(forDirectory: root))
    }
    // What the script left behind (a log, say) mustn't stop the removal.
    let force = dirty || (GitClient.snapshot(forPath: root)?.changedFileCount ?? 0) > 0
    // Worked out while the folder exists: the trusted list holds resolved
    // paths.
    let folder = WorkspaceTrust.normalize(root)
    let wasTrusted = Trust.shared.trustingFolder(folder) == folder
    if case .failure(let error) = GitOperations.removeWorktree(path: root, force: force, root: mainRoot) {
      return (.failure(error), scriptFailed)
    }
    let record = TaskRegistryStore.remove(path: root, root: mainRoot)
    // Impulse's own files for it (a Compose override) go with the folder.
    if let common = GitClient.commonGitDirectory(forPath: mainRoot) {
      try? FileManager.default.removeItem(atPath: taskFolder((root as NSString).lastPathComponent, common: common))
    }
    if wasTrusted {
      Trust.shared.revoke(folder)
      DispatchQueue.main.async { trustDidChange() }
    }
    let archived = ArchivedTask(
      root: root, branch: branch, mainRoot: mainRoot, snapshot: saved, record: record, wasTrusted: wasTrusted)
    return (.success(archived), scriptFailed)
  }

  /// Why a task holding repositories of its own (nil: one that couldn't be
  /// checked) isn't archived.
  static func nestedRepositoryMessage(_ nested: [String]?) -> String {
    guard let nested else {
      return "Impulse couldn't check whether its folder holds a git repository of its own (a clone or a submodule), which removing it would delete for good."
    }
    let shown = nested.prefix(3).joined(separator: ", ") + (nested.count > 3 ? ", …" : "")
    let one = nested.count == 1
    return "It holds \(one ? "a git repository" : "git repositories") of its own (\(shown)), which removing the folder would delete for good, with any commits not pushed. Move \(one ? "it" : "them") out of the task first (a submodule: push its commits, then `git submodule deinit` it)."
  }

  /// "Ignored files in it (.env, node_modules/, …) are deleted, and Undo
  /// can't bring them back." Nil when there are none; `ignored` nil when
  /// they couldn't be listed.
  static func ignoredFilesWarning(_ ignored: [String]?, undo: String = "Undo") -> String? {
    guard let ignored else {
      return "Impulse couldn't list the ignored files in it; any there are deleted, and \(undo) can't bring them back."
    }
    guard !ignored.isEmpty else { return nil }
    let shown = ignored.prefix(3).joined(separator: ", ") + (ignored.count > 3 ? ", …" : "")
    return "Ignored files in it (\(shown)) are deleted, and \(undo) can't bring them back."
  }

  /// The archive script to run in the task at `root`: as it is in a trusted
  /// folder; in one that isn't (a task made from someone else's branch),
  /// only once the user says so, every time, as with its setup script.
  func confirmArchiveScript(_ script: String?, root: String, branch: String, completion: @escaping (String?) -> Void) {
    guard let script, !Trust.shared.isTrusted(root) else { return completion(script) }
    let alert = NSAlert()
    alert.alertStyle = .warning
    alert.messageText = "Run the archive script for \(branch)?"
    alert.informativeText = """
      Archiving \(branch) runs this script first:

        \(script)

      It runs in the branch's checkout, which isn't a trusted folder, and the branch decides what these \
      commands do (a changed package.json script, for example). Run it only if you trust the branch's \
      changes. The task is archived either way.
      """
    alert.addButton(withTitle: "Run Archive Script")
    alert.addButton(withTitle: "Don't Run")
    presentWhenNoSheet { window in
      alert.beginSheetModal(for: window) { response in
        completion(response == .alertFirstButtonReturn ? script : nil)
      }
    }
  }

  /// Why Restore Task fell short. `folderRestored`: its folder is back (Impulse
  /// made it again), and only its uncommitted files aren't.
  struct RestoreFailure: Error {
    let error: GitOperationError
    let folderRestored: Bool
  }

  /// Bring an archived task back (off the main thread): its branch if it
  /// was deleted, its folder, its uncommitted files and its record.
  static func restoreArchived(_ task: ArchivedTask) -> Result<Void, RestoreFailure> {
    if let commit = task.deletedBranchCommit,
      case .failure(let error) = GitOperations.createBranch(task.branch, startPoint: commit, checkout: false, root: task.mainRoot)
    {
      return .failure(RestoreFailure(error: error, folderRestored: false))
    }
    let added = GitOperations.addWorktree(path: task.root, branch: task.branch, newBranch: false, root: task.mainRoot)
    if case .failure(let error) = added { return .failure(RestoreFailure(error: error, folderRestored: false)) }
    if let record = task.record { TaskRegistryStore.restore(record, root: task.mainRoot) }
    if let snapshot = task.snapshot, case .failure(let error) = SafetySnapshots.restore(snapshot, root: task.root) {
      let message =
        "\(task.branch) is back, but its uncommitted files couldn't be put back. They're kept in the snapshot \(snapshot.ref). \(error.message)"
      return .failure(RestoreFailure(error: .invalid(message), folderRestored: true))
    }
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
    /// What it's merged into (`origin/main`), checked again before its
    /// branch is deleted.
    var base: String?
    /// Ignored files and folders, which go with the folder for good (nil:
    /// they couldn't be listed).
    var ignored: [String]? = []
    /// Repositories of its own inside it (nil: that couldn't be checked):
    /// it can't be archived.
    var nested: [String]? = []
    var isOn = true

    var canArchive: Bool { nested?.isEmpty == true }
  }

  var rows: [Row] = []
  var isLoading = true
  var deleteBranches = false
  var deleteRemoteBranches = false
  var remoteNames: [String] { Array(Set(rows.filter(\.isOn).compactMap(\.remote))).sorted() }
  /// Ignored files in the chosen tasks; nil when one's couldn't be listed.
  var ignored: [String]? {
    let chosen = rows.filter(\.isOn)
    guard chosen.allSatisfy({ $0.ignored != nil }) else { return nil }
    return chosen.flatMap { $0.ignored ?? [] }
  }
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
            if let nested = row.nested, let first = nested.first {
              Text("holds a repository of its own (\(first)); move it out first")
                .font(ChromeFont.ui(11)).foregroundStyle(chrome.warning)
            } else if row.nested == nil {
              Text("couldn't check it for repositories of its own").font(ChromeFont.ui(11)).foregroundStyle(chrome.warning)
            } else {
              if row.uncommitted > 0 {
                Text("\(row.uncommitted) uncommitted").font(ChromeFont.ui(11)).foregroundStyle(chrome.warning)
              }
              if let ignored = row.ignored, !ignored.isEmpty {
                Text("\(ignored.count) ignored").font(ChromeFont.ui(11)).foregroundStyle(chrome.warning)
                  .help(ignored.joined(separator: ", "))
              } else if row.ignored == nil {
                Text("ignored files unknown").font(ChromeFont.ui(11)).foregroundStyle(chrome.warning)
              }
            }
          }
        }
        .toggleStyle(.checkbox)
        .disabled(!row.canArchive)
      }
      if let ignored = model.ignored, !ignored.isEmpty {
        Text(
          "Ignored files in them (\(ignored.prefix(3).joined(separator: ", "))\(ignored.count > 3 ? ", …" : "")) are deleted, and Undo can't bring them back."
        )
        .font(ChromeFont.ui(11.5)).foregroundStyle(chrome.warning).fixedSize(horizontal: false, vertical: true)
      } else if model.ignored == nil {
        Text("Ignored files in them are deleted, and Undo can't bring them back.")
          .font(ChromeFont.ui(11.5)).foregroundStyle(chrome.warning).fixedSize(horizontal: false, vertical: true)
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
      let remotes = GitOperations.remotes(root: main)
      let rows = OverlapMonitor.mergedRecords(root: main).map { task in
        let snapshot = GitClient.snapshot(forPath: task.path)
        let trusted = Trust.shared.isTrusted(task.path)
        let nested = GitOperations.nestedRepositories(root: task.path, trusted: trusted)
        return MergedTasksModel.Row(
          path: task.path, branch: task.branch, uncommitted: snapshot?.changedFileCount ?? 0,
          remote: snapshot?.upstream.flatMap { upstream in remotes.first { upstream.hasPrefix("\($0)/") } },
          base: task.baseRef, ignored: GitOperations.ignoredEntryList(root: task.path, trusted: trusted), nested: nested,
          isOn: nested?.isEmpty == true)
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
    var ready: [(row: MergedTasksModel.Row, script: String?)] = []
    // Each task's archive script, asked about where its folder isn't trusted.
    func prepare(_ row: MergedTasksModel.Row) {
      confirmArchiveScript(
        alreadyTrustedProjectConfig(root: row.path)?.archiveScript, root: row.path, branch: row.branch
      ) { script in
        ready.append((row, script))
        next()
      }
    }
    func next() {
      guard !remaining.isEmpty else { return removeAll(ready) }
      let row = remaining.removeFirst()
      let path = TaskRegistry.canonical(row.path)
      guard let workspace = tabManager.workspaces.first(where: { TaskRegistry.canonical($0.root) == path }) else {
        return prepare(row)
      }
      requestCloseWorkspace(workspace.id, recordForUndo: false, then: { prepare(row) }, cancelled: { next() })
    }
    func removeAll(_ ready: [(row: MergedTasksModel.Row, script: String?)]) {
      DispatchQueue.global(qos: .userInitiated).async { [weak self] in
        var archived: [ArchivedTask] = []
        var failures: [String] = []
        for (row, script) in ready {
          let removed = Self.removeTask(root: row.path, branch: row.branch, dirty: row.uncommitted > 0, archiveScript: script)
          if removed.scriptFailed { failures.append("\(row.branch): the archive script failed; archived anyway.") }
          switch removed.result {
          case .failure(let error):
            failures.append("\(row.branch): \(error.message)")
          case .success(var task):
            if deleteBranches {
              // Merged when the sheet looked isn't enough: checked again now.
              let cleanup = TaskLanding.deleteLandedBranch(
                row.branch, mergedInto: [row.base].compactMap { $0 }, remote: deleteRemote ? row.remote : nil,
                root: mainRoot, trusted: Trust.shared.isTrusted(mainRoot))
              task.deletedBranchCommit = cleanup.deletedCommit
              failures += cleanup.kept
            }
            archived.append(task)
          }
        }
        DispatchQueue.main.async {
          guard let self else { return }
          for failure in failures { self.toasts.show(Toast(kind: .warning, message: failure, lifetime: 12)) }
          guard !archived.isEmpty else { return }
          let count = archived.count
          let deleted = archived.filter { $0.deletedBranchCommit != nil }.count
          let branches: String
          switch deleted {
          case 0: branches = ""
          case count: branches = " and deleted \(count == 1 ? "its branch" : "their branches")"
          default: branches = " and deleted \(deleted) of their branches"
          }
          self.toasts.show(
            Toast(
              kind: .success, message: "Archived \(count == 1 ? archived[0].branch : "\(count) tasks")\(branches)",
              actionTitle: "Undo", action: { [weak self] in self?.restoreTasks(archived) }, lifetime: 15))
        }
      }
    }
    next()
  }
}
