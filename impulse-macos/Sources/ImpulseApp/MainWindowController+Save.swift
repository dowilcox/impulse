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
  ///
  /// `completion` gets whether the file was written, after the formatter
  /// (if any) has run and the post-save steps have started. Untitled
  /// editors ask for a name first.
  func saveEditorTab(_ editor: EditorTab, completion: ((Bool) -> Void)? = nil) {
    guard let path = editor.filePath else {
      showSaveAsDialog(for: editor, completion: completion)
      return
    }

    // Fetch the latest content from Monaco (content changes are debounced
    // in JS, so the Swift property may be stale when saving via menu Cmd+S).
    editor.fetchContentAndSave { [weak self, weak editor] success in
      guard let self, let editor else {
        completion?(success)
        return
      }
      guard success else {
        self.toasts.show(
          Toast(kind: .warning, message: "Couldn't save \((path as NSString).lastPathComponent)."))
        completion?(false)
        return
      }
      let saved = editor.lastWrittenText ?? editor.content

      // Format on save — find applicable formatter. Formatters load the
      // project's own config (prettier runs .prettierrc.js): trusted only.
      let formatter = Trust.shared.isTrusted(path) ? self.resolveFormatOnSave(forPath: path) : nil
      if let fmt = formatter, !fmt.command.isEmpty {
        self.runExternalCommand(
          command: fmt.command, args: SaveCommand.expandArguments(fmt.args, file: path),
          cwd: (path as NSString).deletingLastPathComponent, timeout: Self.formatterTimeout,
          failure: "Formatter “\(fmt.command)” failed on \((path as NSString).lastPathComponent)."
        ) { [weak self, weak editor] in
          guard let editor else {
            completion?(true)
            return
          }
          // The formatter rewrote the file: show its result, unless typing
          // has moved on since the save (then it's kept, unsaved).
          editor.adoptDiskChanges(afterSaving: saved) { [weak self, weak editor] in
            if let self, let editor { self.postSaveActions(editor: editor, path: path) }
            completion?(true)
          }
        }
      } else {
        self.postSaveActions(editor: editor, path: path)
        completion?(true)
      }
    }
  }

  /// A formatter that runs longer is stopped (closing and quitting wait
  /// for it).
  static let formatterTimeout: TimeInterval = 60

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
      let args = SaveCommand.expandArguments(cmd.args, file: path)
      let failure =
        "Command on save “\(cmd.name.isEmpty ? cmd.command : cmd.name)” failed on \((path as NSString).lastPathComponent)."
      Self.commandsOnSave.enter()
      if cmd.reloadFile {
        let saved = editor.lastWrittenText ?? editor.content
        runExternalCommand(command: cmd.command, args: args, cwd: cwd, failure: failure) { [weak editor] in
          // Show what the command wrote, unless typing has moved on.
          editor?.adoptDiskChanges(afterSaving: saved)
          Self.commandsOnSave.leave()
        }
      } else {
        runExternalCommand(command: cmd.command, args: args, cwd: cwd, failure: failure) {
          Self.commandsOnSave.leave()
        }
      }
    }
  }

  /// Commands on save still running, in every window: quitting waits for
  /// them (see `AppDelegate.terminateAfterCommandsOnSave`).
  static let commandsOnSave = DispatchGroup()

  /// Whether a command on save is still running.
  static var commandsOnSaveRunning: Bool { commandsOnSave.wait(timeout: .now()) == .timedOut }

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
        // Its file type can set the indentation.
        editor.applySettings(self.tabManager.editorOptionsFromSettings(forPath: chosenPath))

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
  /// executable name (letters, digits, `-`, `_`, `.` only), looked up on the
  /// login shell's `PATH` (the one language servers get). Arguments
  /// and `cwd` must not contain null bytes. When it can't run or fails, a
  /// toast shows `failure` with the first line of its error output;
  /// `completion` is still invoked so the caller's control flow continues.
  private func runExternalCommand(
    command: String, args: [String], cwd: String, timeout: TimeInterval? = nil, failure: String,
    completion: (() -> Void)?
  ) {
    let refuse = { [weak self] (reason: String) in
      NSLog("Not running command '%@': %@", command, reason)
      DispatchQueue.main.async {
        self?.toasts.show(Toast(kind: .warning, message: failure, detail: reason, lifetime: 10))
        completion?()
      }
    }
    guard Self.isSafeExternalCommand(command) else {
      return refuse("“\(command)” must be a program name or an absolute path.")
    }
    guard !cwd.contains("\0"), args.allSatisfy({ !$0.contains("\0") }) else {
      return refuse("Its arguments contain a null byte.")
    }

    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      var problem: String?
      do {
        // The login shell's PATH, as language servers and project scripts
        // get: launched from the Dock, the app's own lacks Homebrew, npm…
        var environment = ProcessInfo.processInfo.environment
        environment["PATH"] = LoginShell.loginPath()
        // A bare name goes through env to honor PATH (the name has been
        // validated); an absolute path runs directly.
        let output = try ChildProcess.run(
          command.hasPrefix("/") ? command : "/usr/bin/env",
          command.hasPrefix("/") ? args : [command] + args,
          in: cwd, environment: environment, timeout: timeout)
        if output.status != 0 || output.timedOut {
          problem = SaveCommand.failureSummary(
            status: output.status, stdout: output.stdout, stderr: output.stderr, timedOut: output.timedOut)
        }
      } catch {
        problem = error.localizedDescription
      }
      if let problem { NSLog("Command '%@' failed: %@", command, problem) }

      DispatchQueue.main.async {
        if let problem {
          self?.toasts.show(Toast(kind: .warning, message: failure, detail: problem, lifetime: 10))
        }
        completion?()
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
    if command == "." || command == ".." || command.hasPrefix("-") { return false }
    let allowed = CharacterSet(
      charactersIn:
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_.")
    return command.unicodeScalars.allSatisfy { allowed.contains($0) }
  }
}
