import AppKit
import Foundation
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
    /// Files touched.
    var files: Int
    /// Reverts the whole edit: open editors undo their step (if it's still
    /// the last one), files on disk go back (if nobody changed them since).
    var undo: () -> Void
  }

  /// Apply `edit` across files: an open editor takes its part as one undo
  /// step, other files are rewritten on disk, and files are created, moved or
  /// trashed as asked.
  func applyWorkspaceEdit(_ edit: WorkspaceEdit) -> WorkspaceEditOutcome {
    var undoSteps: [() -> Void] = []
    var touched = Set<String>()
    func outcome(_ failure: String?) -> WorkspaceEditOutcome {
      let steps = undoSteps
      return WorkspaceEditOutcome(failure: failure, files: touched.count) {
        for step in steps.reversed() { step() }
      }
    }
    func name(_ path: String) -> String { (path as NSString).lastPathComponent }

    // Moving or trashing a file that's open would strand its tab.
    for operation in edit.operations {
      switch operation {
      case .rename(let uri, _, _, _), .delete(let uri, _, _):
        let path = uriToFilePath(uri)
        if openEditor(forPath: path) != nil {
          return outcome("Close \(name(path)) first: the change moves or removes it.")
        }
      default:
        break
      }
    }

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

      case .delete(let uri, _, let ignoreIfNotExists):
        let path = uriToFilePath(uri)
        touched.insert(path)
        guard files.fileExists(atPath: path) else {
          if ignoreIfNotExists { continue }
          return outcome("\(name(path)) doesn't exist.")
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

  /// Toast for an edit that reached past the current file (one-file edits
  /// undo with ⌘Z like any other).
  func reportWorkspaceEdit(_ outcome: WorkspaceEditOutcome, verb: String) {
    if let failure = outcome.failure {
      toasts.show(Toast(kind: .warning, message: failure))
    } else if outcome.files > 1 {
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
        startLine: UInt32(max(0, edit.startLine)), startColumn: UInt32(max(0, edit.startCharacter)),
        endLine: UInt32(max(0, edit.endLine)), endColumn: UInt32(max(0, edit.endCharacter))),
      text: edit.newText)
  }

  /// A server's `workspace/applyEdit`: apply it and answer.
  func handleLspApplyEdit(clientKey: String, id: Any, label: String?, edit: Any) {
    var result: [String: Any]
    if let parsed = WorkspaceEdit.parse(edit) {
      let outcome = applyWorkspaceEdit(parsed)
      reportWorkspaceEdit(outcome, verb: label ?? "Changed")
      result = ["applied": outcome.failure == nil]
      if let failure = outcome.failure { result["failureReason"] = failure }
    } else {
      result = ["applied": false, "failureReason": "Malformed edit"]
    }
    let json = (try? JSONSerialization.data(withJSONObject: result)).flatMap { String(data: $0, encoding: .utf8) }
    core.lspRespond(clientKey: clientKey, id: id, resultJson: json ?? "{\"applied\":false}")
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
      DispatchWorkItem { [weak self] in
        guard let self else { return }
        let response = self.core.lspRequest(
          languageId: action.language, fileUri: action.uri, method: "codeAction/resolve", paramsJson: params)
        let resolved = response.flatMap { $0.data(using: .utf8) }
          .flatMap { (try? JSONSerialization.jsonObject(with: $0)) as? [String: Any] }
        DispatchQueue.main.async {
          guard let resolved, resolved["error"] == nil else {
            self.toasts.show(Toast(kind: .warning, message: "Couldn't apply “\(raw["title"] as? String ?? "action")”."))
            return
          }
          self.carryOut(resolved, from: action)
        }
      })
  }

  private func carryOut(_ codeAction: [String: Any], from action: LspCodeAction) {
    let title = codeAction["title"] as? String ?? "Action"
    if let editJSON = codeAction["edit"], let edit = WorkspaceEdit.parse(editJSON) {
      let outcome = applyWorkspaceEdit(edit)
      reportWorkspaceEdit(outcome, verb: title)
      if outcome.failure != nil { return }
    }
    if let command = codeAction["command"] as? [String: Any], let name = command["command"] as? String {
      executeLspCommand(name, arguments: command["arguments"], title: title, from: action)
    }
  }

  /// `workspace/executeCommand` on the server that registered `command`. Its
  /// edits come back as `workspace/applyEdit`.
  private func executeLspCommand(_ command: String, arguments: Any?, title: String?, from action: LspCodeAction) {
    var params: [String: Any] = ["command": command]
    if let arguments, !(arguments is NSNull) { params["arguments"] = arguments }
    guard let json = Self.jsonString(params) else { return }
    enqueueLspRequest(
      DispatchWorkItem { [weak self] in
        guard let self else { return }
        let response = self.core.lspRequest(
          languageId: action.language, fileUri: action.uri, method: "workspace/executeCommand", paramsJson: json)
        let object = response.flatMap { $0.data(using: .utf8) }
          .flatMap { (try? JSONSerialization.jsonObject(with: $0, options: .fragmentsAllowed)) as? [String: Any] }
        guard let error = object?["error"] as? String else { return }
        DispatchQueue.main.async {
          self.toasts.show(
            Toast(kind: .warning, message: "Couldn't run “\(title ?? command)”.", detail: error))
        }
      })
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
    let language = editor.lspLanguage
    guard let json = Self.jsonString(object) else { return }
    enqueueLspRequest(
      DispatchWorkItem { [weak self] in
        guard let self else { return }
        var response = self.core.lspRequest(
          languageId: language, fileUri: uri, method: method, paramsJson: json) ?? "null"
        // An {"error": …} envelope: no server, no support, or a failure.
        if response.hasPrefix("{\"error\"") { response = "null" }
        DispatchQueue.main.async { [weak editor] in
          editor?.resolveLspRequest(requestId: requestId, result: response)
        }
      })
  }

  static func jsonString(_ value: Any) -> String? {
    guard JSONSerialization.isValidJSONObject(value),
      let data = try? JSONSerialization.data(withJSONObject: value)
    else { return nil }
    return String(data: data, encoding: .utf8)
  }
}
