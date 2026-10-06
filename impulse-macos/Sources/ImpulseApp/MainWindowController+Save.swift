import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI
import os.log

extension MainWindowController {

  // MARK: - Save Pipeline

  /// Unified save pipeline for editor tabs. Handles:
  /// 1. Format on save (if configured)
  /// 2. Actual file save
  /// 3. LSP didSave notification
  /// 4. Commands on save
  /// 5. Git diff decoration refresh
  func saveEditorTab(_ editor: EditorTab) {
    guard let path = editor.filePath else {
      showSaveAsDialog(for: editor)
      return
    }

    // Fetch the latest content from Monaco (content changes are debounced
    // in JS, so the Swift property may be stale when saving via menu Cmd+S).
    editor.fetchContentAndSave { [weak self, weak editor] success in
      guard let self, let editor else { return }
      guard success else {
        self.toasts.show(
          Toast(kind: .warning, message: "Couldn't save \((path as NSString).lastPathComponent)."))
        return
      }
      let saved = editor.lastWrittenText ?? editor.content

      // Format on save — find applicable formatter. Formatters load the
      // project's own config (prettier runs .prettierrc.js): trusted only.
      let formatter = Trust.shared.isTrusted(path) ? self.resolveFormatOnSave(forPath: path) : nil
      if let fmt = formatter, !fmt.command.isEmpty {
        self.runExternalCommand(
          command: fmt.command, args: fmt.args, cwd: (path as NSString).deletingLastPathComponent
        ) { [weak self, weak editor] in
          guard let editor else { return }
          // The formatter rewrote the file: show its result, unless typing
          // has moved on since the save (then it's kept, unsaved).
          editor.adoptDiskChanges(afterSaving: saved) { [weak self, weak editor] in
            guard let self, let editor else { return }
            self.postSaveActions(editor: editor, path: path)
          }
        }
      } else {
        self.postSaveActions(editor: editor, path: path)
      }
    }
  }

  /// The file changed on disk since the editor loaded or saved it: save
  /// over it, take the disk version instead, or don't save.
  func confirmOverwrite(_ editor: EditorTab, proceed: @escaping (Bool) -> Void) {
    guard let window, let path = editor.filePath else { return proceed(true) }
    let alert = NSAlert()
    alert.alertStyle = .warning
    alert.messageText = "“\((path as NSString).lastPathComponent)” changed on disk"
    alert.informativeText =
      "It was changed outside this editor (by an agent, git or another app) since you opened it. Saving replaces those changes with your version."
    alert.addButton(withTitle: "Save Anyway")
    alert.addButton(withTitle: "Reload from Disk")
    alert.addButton(withTitle: "Cancel")
    alert.buttons.first?.hasDestructiveAction = true
    alert.beginSheetModal(for: window) { [weak editor] response in
      switch response {
      case .alertFirstButtonReturn:
        proceed(true)
      case .alertSecondButtonReturn:
        editor?.reloadFromDisk(force: true)
        proceed(false)
      default:
        proceed(false)
      }
    }
  }

  /// Actions that run after saving and optional formatting.
  private func postSaveActions(editor: EditorTab, path: String) {
    tabManager.refreshSegmentLabels()
    lspDidSave(editor: editor)
    applyGitDiffDecorations(editor: editor)
    // Direct git status refresh (skip the debounce — saves are explicit
    // user actions that warrant immediate feedback).
    let nodes = fileTreeData.rootNodes
    let root = fileTreeData.rootPath
    if !root.isEmpty {
      DispatchQueue.global(qos: .userInitiated).async {
        FileTreeNode.refreshGitStatus(nodes: nodes, repoPath: root, dirPath: root)
      }
    }

    // Commands on save: run any matching commands (in trusted folders:
    // they work on the project's files with its tools).
    for cmd in settings.commandsOnSave where Trust.shared.isTrusted(path) {
      guard !cmd.command.isEmpty else { continue }
      guard Settings.matchesFilePattern(path, pattern: cmd.filePattern) else { continue }
      let cwd = (path as NSString).deletingLastPathComponent
      if cmd.reloadFile {
        let saved = editor.lastWrittenText ?? editor.content
        runExternalCommand(command: cmd.command, args: cmd.args, cwd: cwd) { [weak editor] in
          // Show what the command wrote, unless typing has moved on.
          editor?.adoptDiskChanges(afterSaving: saved)
        }
      } else {
        runExternalCommand(command: cmd.command, args: cmd.args, cwd: cwd, completion: nil)
      }
    }
  }

  /// Shows a save-as dialog for an untitled editor tab, then transitions it
  /// to a file-backed editor on successful save. The optional completion is
  /// called with `true` if the user saved, `false` if the panel was cancelled
  /// or the save failed.
  func showSaveAsDialog(for editor: EditorTab, completion: ((Bool) -> Void)? = nil) {
    let panel = NSSavePanel()
    panel.nameFieldStringValue = "Untitled"
    panel.canCreateDirectories = true

    if let cwd = editor.untitledCwd ?? editor.projectDirectory {
      panel.directoryURL = URL(fileURLWithPath: cwd)
    }

    guard let window = self.window else {
      completion?(false)
      return
    }
    panel.beginSheetModal(for: window) { [weak self, weak editor] response in
      guard let self, let editor, response == .OK, let url = panel.url else {
        completion?(false)
        return
      }

      let chosenPath = url.path

      // Set filePath first so fetchContentAndSave writes to the correct location.
      editor.filePath = chosenPath

      editor.fetchContentAndSave { [weak self, weak editor] success in
        guard let self, let editor, success else {
          completion?(false)
          return
        }

        // Transition to file-backed editor: re-open in Monaco with correct URI and language
        let language = self.tabManager.detectLanguage(forPath: chosenPath)
        editor.untitledCwd = nil
        editor.projectDirectory = (chosenPath as NSString).deletingLastPathComponent
        editor.openFile(path: chosenPath, content: editor.content, language: language)

        // Register in dedup set
        self.tabManager.registerOpenFilePath(chosenPath)

        // Post-save actions (refresh tab bar, LSP didOpen, git diff, etc.)
        self.postSaveActions(editor: editor, path: chosenPath)

        // Track the editor tab
        self.trackEditorTab(editor, forPath: chosenPath)
        self.lspDidOpenIfNeeded(path: chosenPath)

        completion?(true)
      }
    }
  }

  /// Resolves the `FormatOnSave` configuration for a file path, checking
  /// file-type overrides first, then falling back to the global setting.
  private func resolveFormatOnSave(forPath path: String) -> FormatOnSave? {
    // Check file-type-specific overrides first
    for override_ in settings.fileTypeOverrides {
      if Settings.matchesFilePattern(path, pattern: override_.pattern),
        let fmt = override_.formatOnSave, !fmt.command.isEmpty
      {
        return fmt
      }
    }
    return nil
  }

  /// Runs an external command asynchronously. Calls `completion` on the main
  /// thread when the process finishes.
  ///
  /// The command name is validated to be either an absolute path or a plain
  /// executable name (letters, digits, `-`, `_`, `.` only). Arguments and
  /// `cwd` must not contain null bytes. Failures are logged and `completion`
  /// is still invoked so the caller's control flow continues.
  private func runExternalCommand(
    command: String, args: [String], cwd: String,
    completion: (() -> Void)?
  ) {
    guard Self.isSafeExternalCommand(command) else {
      NSLog("Refusing to run command with unsafe name: %@", command)
      if let completion = completion { DispatchQueue.main.async { completion() } }
      return
    }
    guard !cwd.contains("\0"), args.allSatisfy({ !$0.contains("\0") }) else {
      NSLog("Refusing to run command with null byte in args/cwd")
      if let completion = completion { DispatchQueue.main.async { completion() } }
      return
    }

    DispatchQueue.global(qos: .userInitiated).async {
      let process = Process()
      if command.hasPrefix("/") {
        // Absolute path: invoke directly, skip PATH lookup via env.
        process.executableURL = URL(fileURLWithPath: command)
        process.arguments = args
      } else {
        // Bare name: use env to honor PATH. Name has been validated.
        process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        process.arguments = [command] + args
      }
      process.currentDirectoryURL = URL(fileURLWithPath: cwd)
      process.standardOutput = FileHandle.nullDevice
      process.standardError = FileHandle.nullDevice

      do {
        try process.run()
        process.waitUntilExit()
      } catch {
        NSLog("Failed to run command '\(command)': \(error)")
      }

      if let completion = completion {
        DispatchQueue.main.async { completion() }
      }
    }
  }

  /// Validates a command name for `runExternalCommand`. Absolute paths are
  /// permitted (but must not contain `..`); bare names must be a plain
  /// identifier — no slashes, shell metacharacters, or leading dashes.
  private static func isSafeExternalCommand(_ command: String) -> Bool {
    if command.isEmpty || command.contains("\0") { return false }
    if command.hasPrefix("/") {
      return !command.contains("..")
    }
    if command == "." || command == ".." { return false }
    let allowed = CharacterSet(
      charactersIn:
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_.")
    return command.unicodeScalars.allSatisfy { allowed.contains($0) }
  }
}
