import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI
import os.log

extension MainWindowController {

  // MARK: - Markdown Preview

  @objc func previewButtonClicked(_ sender: Any?) {
    togglePreview()
  }

  /// For ten seconds after a tab or pane closes, ⌘Z (or the toast) brings it
  /// back: its scrollback, folder and agent session, in a new shell.
  func offerUndoClose(title: String, isPane: Bool) {
    guard let undoManager = window?.undoManager else { return }
    let token = NSObject()
    var used = false
    let reopen: () -> Void = { [weak self, weak undoManager] in
      guard !used else { return }
      used = true
      undoManager?.removeAllActions(withTarget: token)
      self?.tabManager.reopenLastClosedTab()
    }
    undoManager.registerUndo(withTarget: token) { _ in reopen() }
    undoManager.setActionName(isPane ? "Close Pane" : "Close Tab")
    DispatchQueue.main.asyncAfter(deadline: .now() + 10) { [weak undoManager] in
      used = true
      undoManager?.removeAllActions(withTarget: token)
    }
    toasts.show(
      Toast(
        kind: .info, message: "Closed \(title)", actionTitle: "Undo ⌘Z", action: reopen, lifetime: 10))
  }

  /// The file against its staged version in Monaco's diff editor; the
  /// working copy side stays editable.
  func toggleDiffView() {
    guard let editor = tabManager.selectedEditor else {
      toasts.show(Toast(kind: .info, message: "Open a file to see its changes."))
      return
    }
    setDiffView(editor, enabled: !editor.isDiffView)
  }

  func setDiffView(_ editor: EditorTab, enabled: Bool) {
    if enabled {
      guard let path = editor.filePath, GitClient.repoRoot(forPath: path) != nil else {
        toasts.show(Toast(kind: .info, message: "This file isn't in a git repository."))
        return
      }
      if editor.isPreviewing, !editor.isPreviewBeside { togglePreview() }
    }
    editor.setDiffView(enabled)
  }

  /// Markdown or SVG preview beside the editor, following edits.
  func togglePreviewBeside() {
    guard let editor = tabManager.selectedEditor, let fp = editor.filePath, EditorTab.isPreviewableFile(fp) else {
      toasts.show(Toast(kind: .info, message: "Open a Markdown or SVG file to preview it."))
      return
    }
    let themeJSON = ThemeManager.markdownThemeJSON(forName: theme.id)
    if let isPreviewing = editor.togglePreviewBeside(themeJSON: themeJSON, bgColor: theme.bg) {
      windowModel.isPreviewing = isPreviewing
    }
  }

  /// A preview's Run button: run the command in a terminal of the same tab
  /// (the first one), or in a new one split below.
  func runFromPreview(_ command: String, directory: String, editor: EditorTab) {
    if let location = tabManager.location(of: editor),
      case .split(let split) = tabManager.tabs[location.tabIndex],
      let terminal = split.orderedPanes.lazy.compactMap({ pane -> TerminalTab? in
        if case .terminal(let container) = pane.entry { return container.activeTerminal } else { return nil }
      }).first
    {
      terminal.runCommand(command)
      terminal.focus()
      return
    }
    let container = tabManager.makeTerminalContainer(directory: directory, initialCommand: command)
    tabManager.splitSelectedTab(with: .terminal(container), axis: .vertical)
  }

  /// Toggle preview for the active editor tab (markdown or SVG).
  func togglePreview() {
    guard let editor = tabManager.selectedEditor,
      let fp = editor.filePath,
      EditorTab.isPreviewableFile(fp)
    else { return }

    let themeJSON = ThemeManager.markdownThemeJSON(forName: theme.id)
    if let isPreviewing = editor.togglePreview(themeJSON: themeJSON, bgColor: theme.bg) {
      windowModel.isPreviewing = isPreviewing
    }
  }

  /// ⌘G: the palette's line mode ("42" or "42:7").
  func showGoToLineDialog() {
    guard tabManager.selectedEditor != nil else { return }
    showPalette(prefix: ":")
  }

  // MARK: - Git Diff Decorations

  /// Applies git diff gutter decorations to an editor tab by querying
  /// the FFI bridge for diff markers.
  /// Send the editor its file's git base (index version) and blame; Monaco
  /// computes change marks against the live buffer from it.
  func applyGitDiffDecorations(editor: EditorTab) {
    guard let path = editor.filePath else { return }
    DispatchQueue.global(qos: .utility).async {
      let base = GitClient.baseContent(forFile: path)
      var blame: [EditorBlameLine] = []
      if base != nil, let root = GitClient.repoRoot(forPath: path) {
        let relative = path.hasPrefix(root + "/") ? String(path.dropFirst(root.count + 1)) : path
        blame = GitOperations.blame(path: relative, root: root).map { line, info in
          EditorBlameLine(
            line: line, author: info.author, time: info.authorTime.timeIntervalSince1970,
            summary: info.summary, sha: info.sha)
        }
      }
      DispatchQueue.main.async {
        editor.setGitBase(base, blame: blame)
      }
    }
  }

  /// Stage the hunk at `line` (from the editor's git peek) or open it in the
  /// review.
  func handleEditorGitAction(editor: EditorTab, action: String, line: Int) {
    guard let path = editor.filePath, let repository = windowModel.repository,
      path.hasPrefix(repository.root + "/")
    else { return }
    let relative = String(path.dropFirst(repository.root.count + 1))
    if action == "review" {
      gitOpenReview(scope: .unstaged, focusPath: relative)
      return
    }
    if action == "conflicts" { return }
    if action.hasPrefix("commit:") {
      tabManager.addHistoryTab(repository: repository, host: self, reveal: String(action.dropFirst(7)))
      return
    }
    if action == "conflicts-resolved" {
      // The last conflict marker in a conflicted file is gone.
      guard let change = repository.snapshot?.conflicted.first(where: { $0.path == relative }) else {
        return
      }
      let actions = GitActions(repository: repository, host: self)
      toasts.show(
        Toast(
          kind: .success, message: "No conflicts left in \((relative as NSString).lastPathComponent).",
          actionTitle: "Save & Mark Resolved",
          action: { [weak editor] in
            editor?.fetchContentAndSave { saved in
              if saved { actions.markResolved([change]) }
            }
          }, lifetime: 20))
      return
    }
    guard !editor.isModified else {
      toasts.show(
        Toast(
          kind: .info, message: "Save the file to stage this change.", actionTitle: "Save",
          action: { [weak self] in
            if let location = self?.tabManager.location(of: editor) {
              self?.tabManager.reveal(location)
            }
            NotificationCenter.default.post(name: .impulseSaveFile, object: nil)
          }))
      return
    }
    let root = repository.root
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let diff = try? GitClient.fileDiff(repoPath: root, path: relative, scope: .unstaged)
      DispatchQueue.main.async {
        guard let self, let diff else { return }
        guard
          let index = diff.hunks.firstIndex(where: { hunk in
            let start = Int(hunk.newStart)
            let count = hunk.lines.filter { $0.kind != .removed }.count
            return line >= start - 1 && line <= start + max(count, 1)
          })
        else {
          self.toasts.show(Toast(kind: .warning, message: "That change isn't in the unstaged diff."))
          return
        }
        let change = FileChange(path: relative, status: diff.status)
        GitActions(repository: repository, host: self).apply(
          .stage, selection: .wholeHunks([index]), change: change,
          expectedHunkIds: [index: diff.hunkIds[index]], options: DiffOptions()
        ) { [weak self] success in
          if success {
            self?.toasts.show(Toast(kind: .success, message: "Staged the change"))
            self?.applyGitDiffDecorations(editor: editor)
          }
        }
      }
    }
  }
}
