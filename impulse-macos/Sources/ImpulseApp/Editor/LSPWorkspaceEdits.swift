import AppKit
import Foundation
import ImpulseGit
import ImpulseKit

/// A code action Swift carries out when it's picked: the server's raw
/// CodeAction (or Command) and where it came from.
struct LspCodeAction {
  let raw: [String: Any]
  let language: String
  let uri: String
}

/// Workspace edits from language servers (rename, code actions,
/// `workspace/applyEdit`), commands, and requests Monaco asks for as is.
extension MainWindowController {

  // MARK: Workspace edits

  struct WorkspaceEditOutcome {
    /// Why it stopped (changes before the failure stay, as LSP's "abort").
    var failure: String?
    /// The user said no when asked: nothing changed.
    var declined = false
    /// Files touched.
    var files: Int
    /// Reverts the whole edit: open editors undo their step (if it's still
    /// the last one), files on disk go back (if nobody changed them since).
    var undo: () -> Void

    static func refused(_ failure: String) -> WorkspaceEditOutcome {
      WorkspaceEditOutcome(failure: failure, files: 0, undo: {})
    }
  }

  /// Apply `edit` across files: an open editor takes its part as one undo
  /// step, other files are rewritten on disk, and files are created, moved or
  /// trashed as asked. An edit that changes files outside `root` (the folder
  /// it may change without asking: the server's project root for a change a
  /// server makes on its own, `userEditBoundary` for one the user picked),
  /// or creates, moves or deletes files, is applied only once the user says
  /// yes (and `isAllowed` still holds then). `completion` always runs: with
  /// a declined outcome when the window goes before the user answers.
  func applyWorkspaceEdit(
    _ edit: WorkspaceEdit, root: String?, title: String, isAllowed: @escaping () -> Bool = { true },
    completion: @escaping (WorkspaceEditOutcome) -> Void
  ) {
    if let refusal = refusal(for: edit) { return completion(.refused(refusal)) }
    let changes = changesToConfirm(edit, root: root)
    guard !changes.isEmpty else { return completion(apply(edit)) }
    let declined = WorkspaceEditOutcome(failure: nil, declined: true, files: 0, undo: {})
    confirmWorkspaceEdit(title: title, root: root, changes: changes) { [weak self] approved in
      guard approved, let self else { return completion(declined) }
      guard isAllowed() else { return completion(.refused("The language server was stopped.")) }
      completion(self.apply(edit))
    }
  }

  /// Where a rename or code action the user picked in the file at `uri` may
  /// change files without asking (see `WorkspaceEdit.userEditBoundary`).
  func userEditBoundary(forFileUri uri: String) -> String? {
    let path = uriToFilePath(uri)
    return WorkspaceEdit.userEditBoundary(
      trustedFolder: Trust.shared.isEnabled ? Trust.shared.trustingFolder(path) : nil,
      repositoryRoot: GitClient.repoRoot(forPath: (path as NSString).deletingLastPathComponent),
      serverRoot: core.lspRootPath(fileUri: uri))
  }

  private func apply(_ edit: WorkspaceEdit) -> WorkspaceEditOutcome {
    var undoSteps: [() -> Void] = []
    var touched = Set<String>()
    func outcome(_ failure: String?) -> WorkspaceEditOutcome {
      let steps = undoSteps
      return WorkspaceEditOutcome(failure: failure, files: touched.count) {
        for step in steps.reversed() { step() }
      }
    }
    func name(_ path: String) -> String { (path as NSString).lastPathComponent }

    // Checked again: things may have changed while the user was asked.
    if let refusal = refusal(for: edit) { return .refused(refusal) }

    let files = FileManager.default
    for operation in edit.operations {
      switch operation {
      case .edit(let uri, let edits):
        let path = uriToFilePath(uri)
        touched.insert(path)
        guard !edits.isEmpty else { continue }
        if let editor = openEditor(forPath: path) {
          let token = UUID().uuidString
          editor.applyEdits(token: token, edits: edits.map(Self.monacoEdit))
          undoSteps.append { [weak editor] in editor?.undoEdits(token: token) }
        } else {
          // Through symlinks, keeping a byte-order mark; not UTF-8: skipped.
          guard let file = TextFile.read(path) else {
            return outcome("Couldn't read \(name(path)) as UTF-8 text.")
          }
          let original = file.text
          guard let updated = TextEditApplier.apply(edits, to: original) else {
            return outcome("The changes to \(name(path)) overlap.")
          }
          do {
            try TextFile.write(updated, bom: file.bom, to: path)
          } catch {
            return outcome("Couldn't write \(name(path)): \(error.localizedDescription)")
          }
          undoSteps.append {
            guard TextFile.read(path)?.text == updated else { return }
            try? TextFile.write(original, bom: file.bom, to: path)
          }
        }

      case .create(let uri, let overwrite, let ignoreIfExists):
        let path = uriToFilePath(uri)
        touched.insert(path)
        if files.fileExists(atPath: path), ignoreIfExists || !overwrite {
          if ignoreIfExists { continue }
          return outcome("\(name(path)) already exists.")
        }
        let previous = files.contents(atPath: path)
        do {
          try files.createDirectory(
            atPath: (path as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
          try Data().write(to: URL(fileURLWithPath: path))
        } catch {
          return outcome("Couldn't create \(name(path)): \(error.localizedDescription)")
        }
        undoSteps.append {
          if let previous {
            try? previous.write(to: URL(fileURLWithPath: path))
          } else if files.contents(atPath: path)?.isEmpty ?? false {
            try? files.removeItem(atPath: path)
          }
        }

      case .rename(let oldUri, let newUri, let overwrite, let ignoreIfExists):
        let from = uriToFilePath(oldUri)
        let to = uriToFilePath(newUri)
        touched.formUnion([from, to])
        if files.fileExists(atPath: to) {
          if ignoreIfExists { continue }
          guard overwrite else { return outcome("\(name(to)) already exists.") }
          try? files.trashItem(at: URL(fileURLWithPath: to), resultingItemURL: nil)
        }
        do {
          try files.createDirectory(
            atPath: (to as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
          try files.moveItem(atPath: from, toPath: to)
        } catch {
          return outcome("Couldn't move \(name(from)): \(error.localizedDescription)")
        }
        undoSteps.append {
          guard files.fileExists(atPath: to), !files.fileExists(atPath: from) else { return }
          try? files.moveItem(atPath: to, toPath: from)
        }

      case .delete(let uri, let recursive, let ignoreIfNotExists):
        let path = uriToFilePath(uri)
        touched.insert(path)
        var isDirectory: ObjCBool = false
        guard files.fileExists(atPath: path, isDirectory: &isDirectory) else {
          if ignoreIfNotExists { continue }
          return outcome("\(name(path)) doesn't exist.")
        }
        // A folder goes with its contents only when the server said so.
        if isDirectory.boolValue, !recursive, !((try? files.contentsOfDirectory(atPath: path))?.isEmpty ?? true) {
          return outcome("\(name(path)) isn't empty.")
        }
        var trashed: NSURL?
        do {
          try files.trashItem(at: URL(fileURLWithPath: path), resultingItemURL: &trashed)
        } catch {
          return outcome("Couldn't remove \(name(path)): \(error.localizedDescription)")
        }
        let trashedURL = trashed as URL?
        undoSteps.append {
          guard let trashedURL, !files.fileExists(atPath: path) else { return }
          try? files.moveItem(at: trashedURL, to: URL(fileURLWithPath: path))
        }
      }
    }
    if edit.hasResourceOperations { windowModel.onRefreshTree?() }
    return outcome(nil)
  }

  /// Why `edit` can't be applied now, if it can't: a document changed since
  /// the server worked out its edits, or a file the edit moves, replaces or
  /// removes is open in an editor (its tab would be stranded, its unsaved
  /// changes lost).
  private func refusal(for edit: WorkspaceEdit) -> String? {
    func name(_ path: String) -> String { (path as NSString).lastPathComponent }
    // Edits for a document that changed since the server computed them
    // would land in the wrong places.
    for (uri, version) in edit.versions {
      if let current = LSPDocuments.shared.version(uri), current != version {
        return "\(name(uriToFilePath(uri))) changed while the language server was working on it. Try again."
      }
    }
    let files = FileManager.default
    for operation in edit.operations {
      let replaced: [String]
      switch operation {
      case .edit:
        replaced = []
      case .create(let uri, let overwrite, let ignoreIfExists):
        let path = uriToFilePath(uri)
        replaced = overwrite && !ignoreIfExists && files.fileExists(atPath: path) ? [path] : []
      case .rename(let oldUri, let newUri, let overwrite, let ignoreIfExists):
        let to = uriToFilePath(newUri)
        replaced = [uriToFilePath(oldUri)] + (overwrite && !ignoreIfExists && files.fileExists(atPath: to) ? [to] : [])
      case .delete(let uri, _, _):
        replaced = [uriToFilePath(uri)]
      }
      for path in replaced {
        if let open = openFile(atOrUnder: path) {
          return "Close \(name(open)) first: the change moves, replaces or removes it."
        }
      }
    }
    return nil
  }

  /// What the user is asked about before `edit` applies, as lines (empty:
  /// nothing; see `WorkspaceEdit.confirmations`).
  private func changesToConfirm(_ edit: WorkspaceEdit, root: String?) -> [String] {
    let rootName = root.map { ($0 as NSString).lastPathComponent }
    func shown(_ path: String) -> String {
      if let root, let rootName, WorkspaceEdit.isInside(path, root) {
        let standardized = (path as NSString).standardizingPath
        let prefix = (root as NSString).standardizingPath + "/"
        if standardized.hasPrefix(prefix) { return rootName + "/" + standardized.dropFirst(prefix.count) }
      }
      return TabManager.abbreviateHomePath(WorkspaceEdit.resolvedPath(path))
    }
    return edit.confirmations(within: root, path: uriToFilePath).map { confirmation in
      switch confirmation {
      case .changes(let path): return "Changes \(shown(path))"
      case .creates(let path): return "Creates \(shown(path))"
      case .empties(let path): return "Replaces \(shown(path)) with an empty file"
      case .moves(let from, let to): return "Moves \(shown(from)) to \(shown(to))"
      case .deletes(let path, let withContents):
        return withContents ? "Deletes \(shown(path)) and everything in it" : "Deletes \(shown(path))"
      }
    }
  }

  /// Ask whether to apply a change that does more than edit files in
  /// `root`. `completion` runs once: with the answer, or false when there's
  /// no window, or it closes while the question waits for another sheet
  /// (or is up).
  private func confirmWorkspaceEdit(
    title: String, root: String?, changes: [String], completion: @escaping (Bool) -> Void
  ) {
    guard let window else { return completion(false) }
    let shownLimit = 12
    var list = changes.prefix(shownLimit).map { "• \($0)" }
    if changes.count > shownLimit { list.append("…and \(changes.count - shownLimit) more") }
    let place = root.map { "“\(($0 as NSString).lastPathComponent)”" } ?? "the project"
    let alert = NSAlert()
    alert.messageText = "Apply “\(title)”?"
    alert.informativeText = "It does more than edit files in \(place):\n\n" + list.joined(separator: "\n")
    alert.addButton(withTitle: "Apply")
    alert.addButton(withTitle: "Cancel")

    // Main thread only.
    var closeObserver: NSObjectProtocol?
    var answered = false
    func answer(_ approved: Bool) {
      guard !answered else { return }
      answered = true
      if let closeObserver { NotificationCenter.default.removeObserver(closeObserver) }
      completion(approved)
    }
    closeObserver = NotificationCenter.default.addObserver(
      forName: NSWindow.willCloseNotification, object: window, queue: .main
    ) { _ in answer(false) }
    presentWhenNoSheet { window in
      guard !answered else { return }
      alert.beginSheetModal(for: window) { response in
        answer(response == .alertFirstButtonReturn)
      }
    }
  }

  /// The editor showing `path` in any window (an open file is edited in its
  /// editor, never behind its back on disk).
  private func openEditor(forPath path: String) -> EditorTab? {
    if let editor = findEditorTab(forPath: path) { return editor }
    for window in NSApp.windows {
      guard let other = window.windowController as? MainWindowController, other !== self,
        let editor = other.findEditorTab(forPath: path)
      else { continue }
      return editor
    }
    return nil
  }

  /// A file open in an editor (in any window) that is `path`, or inside it
  /// when it's a folder.
  private func openFile(atOrUnder path: String) -> String? {
    let target = (path as NSString).standardizingPath
    let controllers = [self] + NSApp.windows.compactMap { $0.windowController as? MainWindowController }.filter { $0 !== self }
    for controller in controllers {
      for case .editor(let editor) in controller.tabManager.allSurfaces {
        guard let open = editor.filePath.map({ ($0 as NSString).standardizingPath }) else { continue }
        if open == target || open.hasPrefix(target + "/") { return open }
      }
    }
    return nil
  }

  /// Toast for an edit that reached past the current file (one-file edits
  /// undo with ⌘Z like any other).
  func reportWorkspaceEdit(_ outcome: WorkspaceEditOutcome, verb: String) {
    if let failure = outcome.failure {
      toasts.show(Toast(kind: .warning, message: failure))
    } else if !outcome.declined, outcome.files > 1 {
      toasts.show(
        Toast(
          kind: .success, message: "\(verb) in \(outcome.files) files", actionTitle: "Undo",
          action: outcome.undo))
    }
  }

  /// Whether `edit` only changes the file at `uri` (Monaco can apply it).
  func isLocalEdit(_ edit: WorkspaceEdit, uri: String) -> Bool {
    let path = uriToFilePath(uri)
    return !edit.hasResourceOperations && edit.uris.allSatisfy { uriToFilePath($0) == path }
  }

  static func monacoEdit(_ edit: LSPTextEdit) -> MonacoTextEdit {
    MonacoTextEdit(
      range: MonacoRange(
        startLine: UInt32(clamping: edit.startLine), startColumn: UInt32(clamping: edit.startCharacter),
        endLine: UInt32(clamping: edit.endLine), endColumn: UInt32(clamping: edit.endCharacter)),
      text: edit.newText)
  }

  /// A server's `workspace/applyEdit`: apply it (within its project root
  /// unless the user agrees) and answer. A server that has been stopped
  /// (its folder isn't trusted any more, say) changes nothing.
  func handleLspApplyEdit(clientKey: String, id: Any, label: String?, edit: Any) {
    let core = self.core
    func answer(_ result: [String: Any]) {
      let json = (try? JSONSerialization.data(withJSONObject: result)).flatMap { String(data: $0, encoding: .utf8) }
      core.lspRespond(clientKey: clientKey, id: id, resultJson: json ?? "{\"applied\":false}")
    }
    guard core.lspIsServing(clientKey: clientKey) else {
      return answer(["applied": false, "failureReason": "The language server was stopped."])
    }
    guard let parsed = WorkspaceEdit.parse(edit) else {
      return answer(["applied": false, "failureReason": "Malformed edit"])
    }
    applyWorkspaceEdit(
      parsed, root: core.lspRootPath(clientKey: clientKey), title: label ?? "Change from the language server",
      isAllowed: { core.lspIsServing(clientKey: clientKey) }
    ) { [weak self] outcome in
      self?.reportWorkspaceEdit(outcome, verb: label ?? "Changed")
      var result: [String: Any] = ["applied": outcome.failure == nil && !outcome.declined]
      if let failure = outcome.failure { result["failureReason"] = failure }
      if outcome.declined { result["failureReason"] = "The change was declined." }
      answer(result)
    }
  }

  // MARK: Code actions

  /// LSP code actions for Monaco. Actions it can apply itself (edits to this
  /// file) go as edits; the rest (commands, other files, unresolved) get a
  /// token and run here when picked.
  func monacoCodeActions(_ json: String, language: String, uri: String) -> (
    actions: [MonacoCodeAction], carried: [String: LspCodeAction]
  ) {
    guard let data = json.data(using: .utf8),
      let items = (try? JSONSerialization.jsonObject(with: data)) as? [[String: Any]]
    else { return ([], [:]) }
    var actions: [MonacoCodeAction] = []
    var carried: [String: LspCodeAction] = [:]
    for item in items {
      guard let title = item["title"] as? String, item["disabled"] == nil else { continue }
      let kind = item["kind"] as? String
      let preferred = (item["isPreferred"] as? NSNumber)?.boolValue ?? false
      let edit = WorkspaceEdit.parse(item["edit"])
      let needsSwift =
        item["command"] != nil || (item["edit"] == nil && item["data"] != nil)
        || edit.map { !isLocalEdit($0, uri: uri) } ?? false
      if needsSwift {
        let token = UUID().uuidString
        carried[token] = LspCodeAction(raw: item, language: language, uri: uri)
        actions.append(
          MonacoCodeAction(title: title, kind: kind, edits: [], isPreferred: preferred, commandToken: token))
      } else if let edit {
        let edits = edit.textEdits.flatMap { file in
          file.edits.map { textEdit in
            let monaco = Self.monacoEdit(textEdit)
            return MonacoWorkspaceTextEdit(uri: file.uri, range: monaco.range, text: monaco.text)
          }
        }
        actions.append(MonacoCodeAction(title: title, kind: kind, edits: edits, isPreferred: preferred))
      }
    }
    return (actions, carried)
  }

  /// A picked code action Swift carries out: resolve it if the server left
  /// its edit for later, apply the edit, then run its command.
  func runLspCodeAction(token: String) {
    guard let action = lspCodeActions[token] else { return }
    let raw = action.raw
    if let command = raw["command"] as? String {
      executeLspCommand(command, arguments: raw["arguments"], title: raw["title"] as? String, from: action)
      return
    }
    guard raw["edit"] == nil, raw["data"] != nil, let params = Self.jsonString(raw) else {
      carryOut(raw, from: action)
      return
    }
    enqueueLspRequest(
      languageId: action.language, fileUri: action.uri, method: "codeAction/resolve", params: params
    ) { [weak self] response in
      let resolved = response.flatMap { $0.data(using: .utf8) }
        .flatMap { (try? JSONSerialization.jsonObject(with: $0)) as? [String: Any] }
      DispatchQueue.main.async {
        guard let self else { return }
        guard let resolved, resolved["error"] == nil else {
          self.toasts.show(Toast(kind: .warning, message: "Couldn't apply “\(raw["title"] as? String ?? "action")”."))
          return
        }
        self.carryOut(resolved, from: action)
      }
    }
  }

  private func carryOut(_ codeAction: [String: Any], from action: LspCodeAction) {
    let title = codeAction["title"] as? String ?? "Action"
    let command = (codeAction["command"] as? [String: Any]).flatMap { command in
      (command["command"] as? String).map { (name: $0, arguments: command["arguments"]) }
    }
    guard let editJSON = codeAction["edit"], let edit = WorkspaceEdit.parse(editJSON) else {
      if let command { executeLspCommand(command.name, arguments: command.arguments, title: title, from: action) }
      return
    }
    applyWorkspaceEdit(edit, root: userEditBoundary(forFileUri: action.uri), title: title) { [weak self] outcome in
      guard let self else { return }
      self.reportWorkspaceEdit(outcome, verb: title)
      guard outcome.failure == nil, !outcome.declined, let command else { return }
      self.executeLspCommand(command.name, arguments: command.arguments, title: title, from: action)
    }
  }

  /// `workspace/executeCommand` on the server that registered `command`. Its
  /// edits come back as `workspace/applyEdit`.
  private func executeLspCommand(_ command: String, arguments: Any?, title: String?, from action: LspCodeAction) {
    var params: [String: Any] = ["command": command]
    if let arguments, !(arguments is NSNull) { params["arguments"] = arguments }
    guard let json = Self.jsonString(params) else { return }
    enqueueLspRequest(
      languageId: action.language, fileUri: action.uri, method: "workspace/executeCommand", params: json
    ) { [weak self] response in
      let object = response.flatMap { $0.data(using: .utf8) }
        .flatMap { (try? JSONSerialization.jsonObject(with: $0, options: .fragmentsAllowed)) as? [String: Any] }
      guard let error = object?["error"] as? String else { return }
      DispatchQueue.main.async {
        self?.toasts.show(
          Toast(kind: .warning, message: "Couldn't run “\(title ?? command)”.", detail: error))
      }
    }
  }

  // MARK: Pass-through requests

  /// Requests Monaco builds and reads in LSP terms; Swift adds the document
  /// and forwards them.
  static let passthroughMethods: Set<String> = [
    "textDocument/documentHighlight", "textDocument/inlayHint", "textDocument/typeDefinition",
    "textDocument/implementation", "textDocument/declaration",
  ]

  func handleLspPassthrough(editor: EditorTab, requestId: UInt64, method: String, params: String) {
    guard Self.passthroughMethods.contains(method), let path = editor.filePath,
      var object = params.data(using: .utf8).flatMap({ (try? JSONSerialization.jsonObject(with: $0)) as? [String: Any] })
    else {
      editor.resolveLspRequest(requestId: requestId, result: "null")
      return
    }
    let uri = URL(fileURLWithPath: path).absoluteString
    object["textDocument"] = ["uri": uri]
    guard let json = Self.jsonString(object) else { return }
    enqueueLspRequest(languageId: editor.lspLanguage, fileUri: uri, method: method, params: json) {
      [weak editor] response in
      var response = response ?? "null"
      // An {"error": …} envelope: no server, no support, or a failure.
      if response.hasPrefix("{\"error\"") { response = "null" }
      DispatchQueue.main.async {
        editor?.resolveLspRequest(requestId: requestId, result: response)
      }
    }
  }

  static func jsonString(_ value: Any) -> String? {
    guard JSONSerialization.isValidJSONObject(value),
      let data = try? JSONSerialization.data(withJSONObject: value)
    else { return nil }
    return String(data: data, encoding: .utf8)
  }
}
