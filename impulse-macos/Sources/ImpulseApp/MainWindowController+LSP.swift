import AppKit
import Foundation
import ImpulseKit
import os.log

/// Serializes a JSON-compatible value (dictionaries/arrays of `String`, numbers,
/// `Bool`) to a compact string for LSP params. Uses `JSONSerialization` so all
/// strings are escaped correctly — no hand-rolled escaping.
///
/// Returns an empty object on failure so the LSP call still produces valid JSON.
private func encodeLspJSON(_ value: Any) -> String {
  guard JSONSerialization.isValidJSONObject(value),
    let data = try? JSONSerialization.data(withJSONObject: value, options: []),
    let string = String(data: data, encoding: .utf8)
  else {
    NSLog("encodeLspJSON: failed to serialize value")
    return "{}"
  }
  return string
}

// MARK: - LSP Integration

/// What the (app-wide) language servers have open, across windows. Main
/// thread only.
enum LSPDocuments {
  static var shared = LSPOpenDocuments()
}

/// A language-server request a window waits on (see `enqueueLspRequest`).
/// Cancelling it before it's sent drops it; after, the server is told to
/// stop, and its answer is dropped.
final class LSPRequestTicket {
  private let lock = NSLock()
  private var cancelled = false
  private var cancelSent: (() -> Void)?

  var isCancelled: Bool { lock.withLock { cancelled } }

  func cancel() {
    let cancelSent = lock.withLock { () -> (() -> Void)? in
      cancelled = true
      defer { self.cancelSent = nil }
      return self.cancelSent
    }
    cancelSent?()
  }

  /// It went to the registry: `cancel` now withdraws it there.
  fileprivate func sent(cancel: @escaping () -> Void) {
    let alreadyCancelled = lock.withLock { () -> Bool in
      if !cancelled { cancelSent = cancel }
      return cancelled
    }
    if alreadyCancelled { cancel() }
  }
}

extension MainWindowController {

  func applyLspDiagnostics(uri: String, diagnosticsArray: [[String: Any]]) {
    recordProblems(path: uriToFilePath(uri), diagnosticsArray)
    // A server's trailing `diagnostics: []` can arrive after lspDidClose;
    // applying it to a freshly-reopened or still-closing tab clobbers valid
    // state.
    guard lspOpenFiles.contains(uri) else { return }
    let filePath = uriToFilePath(uri)
    guard let editorTab = findEditorTab(forPath: filePath) else { return }

    let markers: [MonacoDiagnostic] = diagnosticsArray.compactMap { d in
      guard let severity = (d["severity"] as? NSNumber)?.uint8Value,
        let startLine = (d["startLine"] as? NSNumber)?.uint32Value,
        let startColumn = (d["startColumn"] as? NSNumber)?.uint32Value,
        let endLine = (d["endLine"] as? NSNumber)?.uint32Value,
        let endColumn = (d["endColumn"] as? NSNumber)?.uint32Value,
        let message = d["message"] as? String
      else { return nil }
      // LSP's zero-based positions: editor.js converts them, as it does for
      // every command (adding one here too drew each mark a line and a
      // column late).
      return MonacoDiagnostic(
        severity: diagnosticSeverityToMonaco(severity),
        startLine: startLine,
        startColumn: startColumn,
        endLine: endLine,
        endColumn: endColumn,
        message: message,
        source: d["source"] as? String
      )
    }
    editorTab.applyDiagnostics(uri: uri, markers: markers)
  }

  /// The symbols in an editor's file, from its language server (nil when
  /// there's no server, or it doesn't answer).
  func documentSymbols(for editor: EditorTab, completion: @escaping ([OutlineSymbol]?) -> Void) {
    guard let path = editor.filePath else { return completion(nil) }
    lspDidOpenIfNeeded(path: path)
    let uri = filePathToUri(path)
    let language = editor.lspLanguage
    let params = encodeLspJSON(["textDocument": ["uri": uri]])
    enqueueLspRequest(
      languageId: language, fileUri: uri, method: "textDocument/documentSymbol", params: params
    ) { response in
      // An {"error": …} envelope means no server (or it failed): nil.
      let data = response.flatMap { $0.data(using: .utf8) }
      let failed = data.flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] }?["error"] != nil
      let symbols = failed ? nil : data.map(DocumentSymbols.parse)
      DispatchQueue.main.async { completion(symbols) }
    }
  }

  /// Project-wide symbols matching `query`, from the language server of
  /// `editor`'s file.
  func workspaceSymbols(
    query: String, editor: EditorTab, completion: @escaping ([(symbol: OutlineSymbol, path: String)]?) -> Void
  ) {
    guard let path = editor.filePath else { return completion(nil) }
    lspDidOpenIfNeeded(path: path)
    let uri = filePathToUri(path)
    let language = editor.lspLanguage
    let params = encodeLspJSON(["query": query])
    enqueueLspRequest(languageId: language, fileUri: uri, method: "workspace/symbol", params: params) { response in
      let data = response.flatMap { $0.data(using: .utf8) }
      let failed = data.flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] }?["error"] != nil
      let results = failed ? nil : data.map(DocumentSymbols.parseWorkspace)
      DispatchQueue.main.async { completion(results) }
    }
  }

  /// Keep every file's diagnostics for the Problems panel, open or not
  /// (servers like rust-analyzer report the whole workspace).
  private func recordProblems(path: String, _ diagnosticsArray: [[String: Any]]) {
    let problems: [Problem] = diagnosticsArray.compactMap { d in
      guard let severity = (d["severity"] as? NSNumber)?.intValue,
        let line = (d["startLine"] as? NSNumber)?.intValue,
        let column = (d["startColumn"] as? NSNumber)?.intValue,
        let message = d["message"] as? String
      else { return nil }
      let code = (d["code"] as? String) ?? (d["code"] as? NSNumber)?.stringValue
      return Problem(
        path: path, line: line + 1, column: column + 1,
        severity: Problem.Severity(rawValue: severity) ?? .info, message: message,
        source: d["source"] as? String, code: code)
    }
    if problems.isEmpty {
      if windowModel.problemsByPath[path] != nil { windowModel.problemsByPath[path] = nil }
    } else if windowModel.problemsByPath[path] != problems {
      windowModel.problemsByPath[path] = problems
    }
  }

  /// Sends LSP didOpen for a file if not already tracked. Only files a
  /// server is configured for are opened (and then mirrored on every edit).
  func lspDidOpenIfNeeded(path: String) {
    let uri = filePathToUri(path)
    guard !lspOpenFiles.contains(uri) else { return }

    guard let editorTab = findEditorTab(forPath: path) else {
      // Tab not created yet (async). didOpen will be sent when the
      // editor fires FileOpened via the notification observer.
      return
    }
    guard core.lspHasServers(languageId: editorTab.lspLanguage, fileUri: uri) else { return }
    // Servers can run the project's code: trusted folders only.
    guard Trust.shared.isTrusted(path) else {
      noteRestrictedFile(path, language: editorTab.lspLanguage)
      return
    }
    lspOpenFiles.insert(uri)
    // Another window has it open already: the servers know it.
    guard LSPDocuments.shared.open(uri) else { return }

    let language = editorTab.lspLanguage
    let content = editorTab.content

    // Servers that aren't running yet start in the background and get the
    // document once they're up; this doesn't wait for them.
    lspQueue.async { [weak self] in
      guard let self else { return }
      let params = encodeLspJSON([
        "textDocument": [
          "uri": uri,
          "languageId": language,
          "version": 1,
          "text": content,
        ]
      ])
      self.core.lspNotify(
        languageId: language, fileUri: uri, method: "textDocument/didOpen", paramsJson: params)
    }
  }

  /// Sends LSP didChange for a content update.
  func lspDidChange(editor: EditorTab, changes: [MonacoContentChange] = []) {
    guard let path = editor.filePath else { return }
    let uri = filePathToUri(path)
    guard lspOpenFiles.contains(uri), let version = LSPDocuments.shared.nextVersion(uri) else { return }
    // Shown in two windows, each with its own buffer: incremental changes
    // from one wouldn't apply to the other's text, so send it whole.
    let whole = LSPDocuments.shared.holders(uri) > 1

    let language = editor.lspLanguage
    // The whole text only when it's needed: while the copy is held, the
    // editor's next change copies its text.
    let fullText = changes.isEmpty || whole ? editor.content : nil

    lspQueue.async { [weak self] in
      guard let self else { return }
      self.core.lspDidChange(
        languageId: language,
        fileUri: uri,
        version: version,
        fullText: fullText,
        changes: fullText == nil ? changes : []
      )
    }
  }

  /// Sends LSP didClose when an editor tab is closed.
  func lspDidClose(editor: EditorTab) {
    guard let path = editor.filePath else { return }
    let uri = filePathToUri(path)
    guard lspOpenFiles.contains(uri) else { return }
    lspOpenFiles.remove(uri)
    clearPendingLSPRequests(for: uri)
    // Another window still shows it.
    guard LSPDocuments.shared.close(uri) else { return }

    let language = editor.lspLanguage
    // The app-wide core, not this window: it may be closing.
    let core = self.core
    lspQueue.async {
      let params = encodeLspJSON(["textDocument": ["uri": uri]])
      core.lspNotify(
        languageId: language, fileUri: uri, method: "textDocument/didClose", paramsJson: params)
    }
  }

  /// Cancels any in-flight per-URI LSP work items and drops latest-request
  /// IDs so responses that arrive after close are ignored. Without this, the
  /// per-URI dictionaries grow unbounded over a session.
  private func clearPendingLSPRequests(for uri: String) {
    completionWorkItems[uri]?.cancel()
    completionWorkItems.removeValue(forKey: uri)
    hoverWorkItems[uri]?.cancel()
    hoverWorkItems.removeValue(forKey: uri)
    formattingWorkItems[uri]?.cancel()
    formattingWorkItems.removeValue(forKey: uri)
    signatureHelpWorkItems[uri]?.cancel()
    signatureHelpWorkItems.removeValue(forKey: uri)
    referencesWorkItems[uri]?.cancel()
    referencesWorkItems.removeValue(forKey: uri)
    codeActionWorkItems[uri]?.cancel()
    codeActionWorkItems.removeValue(forKey: uri)
    renameWorkItems[uri]?.cancel()
    renameWorkItems.removeValue(forKey: uri)
    prepareRenameWorkItems[uri]?.cancel()
    prepareRenameWorkItems.removeValue(forKey: uri)

    latestCompletionReq.removeValue(forKey: uri)
    latestHoverReq.removeValue(forKey: uri)
    latestFormattingReq.removeValue(forKey: uri)
    latestSignatureHelpReq.removeValue(forKey: uri)
    latestReferencesReq.removeValue(forKey: uri)
    latestCodeActionReq.removeValue(forKey: uri)
    latestRenameReq.removeValue(forKey: uri)
  }

  /// Sends LSP didSave after a file is saved.
  func lspDidSave(editor: EditorTab) {
    guard let path = editor.filePath else { return }
    let uri = filePathToUri(path)
    guard lspOpenFiles.contains(uri) else { return }

    let language = editor.lspLanguage
    lspQueue.async { [weak self] in
      guard let self else { return }
      let params = encodeLspJSON(["textDocument": ["uri": uri]])
      self.core.lspNotify(
        languageId: language, fileUri: uri, method: "textDocument/didSave", paramsJson: params)
    }
  }

  /// Handles a completion request from the editor by forwarding it to the LSP.
  func handleCompletionRequest(
    editor: EditorTab, requestId: UInt64, line: UInt32, character: UInt32
  ) {
    guard let path = editor.filePath else { return }
    let uri = filePathToUri(path)

    // Withdraw any previous completion request for this URI BEFORE setting
    // the new request ID, so a stale answer can't race the new one.
    completionWorkItems[uri]?.cancel()
    latestCompletionReq[uri] = requestId

    let params = encodeLspJSON(positionParams(uri: uri, line: line, character: character))
    completionWorkItems[uri] = enqueueLspRequest(
      languageId: editor.lspLanguage, fileUri: uri, method: "textDocument/completion", params: params
    ) { [weak self] response in
      guard let self, let response else { return }
      let items = self.parseCompletionResponse(response)
      DispatchQueue.main.async { [weak self] in
        guard let self else { return }
        // Only resolve if this is still the latest request for this URI.
        guard self.latestCompletionReq[uri] == requestId else { return }
        editor.resolveCompletions(requestId: requestId, items: items)
      }
    }
  }

  /// Handles a hover request from the editor by forwarding it to the LSP.
  func handleHoverRequest(editor: EditorTab, requestId: UInt64, line: UInt32, character: UInt32) {
    guard let path = editor.filePath else { return }
    let uri = filePathToUri(path)
    latestHoverReq[uri] = requestId

    // Withdraw any previous hover request for this URI.
    hoverWorkItems[uri]?.cancel()

    let params = encodeLspJSON(positionParams(uri: uri, line: line, character: character))
    hoverWorkItems[uri] = enqueueLspRequest(
      languageId: editor.lspLanguage, fileUri: uri, method: "textDocument/hover", params: params
    ) { [weak self] response in
      guard let self, let response else { return }
      let contents = self.parseHoverResponse(response)
      DispatchQueue.main.async { [weak self] in
        guard let self else { return }
        guard self.latestHoverReq[uri] == requestId else { return }
        editor.resolveHover(requestId: requestId, contents: contents)
      }
    }
  }

  /// Handles a go-to-definition request from the editor.
  ///
  /// This is called on Cmd+hover (to show the underline) AND on Cmd+click.
  /// We only resolve the Monaco promise here — actual navigation for
  /// cross-file definitions happens via ``handleOpenFileRequested`` when
  /// Monaco's editor opener fires on click.
  func handleDefinitionRequest(
    editor: EditorTab, requestId: UInt64, line: UInt32, character: UInt32
  ) {
    guard let path = editor.filePath else {
      editor.resolveDefinition(requestId: requestId, uri: nil, line: nil, column: nil)
      return
    }
    let uri = filePathToUri(path)
    let params = encodeLspJSON(positionParams(uri: uri, line: line, character: character))

    enqueueLspRequest(
      languageId: editor.lspLanguage, fileUri: uri, method: "textDocument/definition", params: params
    ) { [weak self] response in
      guard let self, let response, let def = self.parseDefinitionResponse(response) else {
        DispatchQueue.main.async {
          editor.resolveDefinition(requestId: requestId, uri: nil, line: nil, column: nil)
        }
        return
      }

      DispatchQueue.main.async {
        // Resolve the Monaco promise with the definition location.
        // Monaco shows an underline on hover and navigates on click.
        // Same-file: Monaco navigates directly.
        // Cross-file: Monaco calls registerEditorOpener → OpenFileRequested.
        editor.resolveDefinition(
          requestId: requestId, uri: def.uri, line: def.line, column: def.character)
      }
    }
  }

  /// Handles a formatting request from the editor by forwarding it to the LSP.
  func handleFormattingRequest(
    editor: EditorTab, requestId: UInt64, tabSize: UInt32, insertSpaces: Bool
  ) {
    guard let path = editor.filePath else { return }
    let uri = filePathToUri(path)
    latestFormattingReq[uri] = requestId

    formattingWorkItems[uri]?.cancel()

    let params = encodeLspJSON([
      "textDocument": ["uri": uri],
      "options": ["tabSize": tabSize, "insertSpaces": insertSpaces] as [String: Any],
    ])

    formattingWorkItems[uri] = enqueueLspRequest(
      languageId: editor.lspLanguage, fileUri: uri, method: "textDocument/formatting", params: params
    ) { [weak self] response in
      guard let self, let response else { return }
      let edits = self.parseFormattingResponse(response)
      DispatchQueue.main.async { [weak self] in
        guard let self else { return }
        guard self.latestFormattingReq[uri] == requestId else { return }
        editor.resolveFormatting(requestId: requestId, edits: edits)
      }
    }
  }

  /// Handles a signature help request from the editor by forwarding it to the LSP.
  func handleSignatureHelpRequest(
    editor: EditorTab, requestId: UInt64, line: UInt32, character: UInt32
  ) {
    guard let path = editor.filePath else { return }
    let uri = filePathToUri(path)
    latestSignatureHelpReq[uri] = requestId

    signatureHelpWorkItems[uri]?.cancel()

    let params = encodeLspJSON(positionParams(uri: uri, line: line, character: character))
    signatureHelpWorkItems[uri] = enqueueLspRequest(
      languageId: editor.lspLanguage, fileUri: uri, method: "textDocument/signatureHelp", params: params
    ) { [weak self] response in
      guard let self, let response else { return }
      let signatureHelp = self.parseSignatureHelpResponse(response)
      DispatchQueue.main.async { [weak self] in
        guard let self else { return }
        guard self.latestSignatureHelpReq[uri] == requestId else { return }
        editor.resolveSignatureHelp(requestId: requestId, signatureHelp: signatureHelp)
      }
    }
  }

  /// Handles a references request from the editor by forwarding it to the LSP.
  func handleReferencesRequest(
    editor: EditorTab, requestId: UInt64, line: UInt32, character: UInt32
  ) {
    guard let path = editor.filePath else { return }
    let uri = filePathToUri(path)
    latestReferencesReq[uri] = requestId

    referencesWorkItems[uri]?.cancel()

    let params = encodeLspJSON([
      "textDocument": ["uri": uri] as [String: Any],
      "position": ["line": line, "character": character],
      "context": ["includeDeclaration": true],
    ])

    referencesWorkItems[uri] = enqueueLspRequest(
      languageId: editor.lspLanguage, fileUri: uri, method: "textDocument/references", params: params
    ) { [weak self] response in
      guard let self, let response else { return }
      let locations = self.parseReferencesResponse(response)
      DispatchQueue.main.async { [weak self] in
        guard let self else { return }
        guard self.latestReferencesReq[uri] == requestId else { return }
        editor.resolveReferences(requestId: requestId, locations: locations)
      }
    }
  }

  /// Handles a code action request from the editor by forwarding it to the LSP.
  func handleCodeActionRequest(
    editor: EditorTab, requestId: UInt64, startLine: UInt32, startColumn: UInt32, endLine: UInt32,
    endColumn: UInt32, diagnostics: [[String: Any]]
  ) {
    guard let path = editor.filePath else { return }
    let uri = filePathToUri(path)
    latestCodeActionReq[uri] = requestId

    codeActionWorkItems[uri]?.cancel()

    let language = editor.lspLanguage

    // Build diagnostics array from the Monaco diagnostics passed through.
    // Values are already 0-based (JS converts from Monaco 1-based before sending).
    var diagObjects: [[String: Any]] = []
    for d in diagnostics {
      guard let startL = (d["startLine"] as? NSNumber)?.uint32Value,
        let startC = (d["startColumn"] as? NSNumber)?.uint32Value,
        let endL = (d["endLine"] as? NSNumber)?.uint32Value,
        let endC = (d["endColumn"] as? NSNumber)?.uint32Value,
        let message = d["message"] as? String
      else { continue }
      let severity = (d["severity"] as? NSNumber)?.uint8Value ?? 1
      // Convert Monaco severity back to LSP severity
      let lspSeverity: UInt8
      switch severity {
      case 8: lspSeverity = 1  // Error
      case 4: lspSeverity = 2  // Warning
      case 2: lspSeverity = 3  // Information
      case 1: lspSeverity = 4  // Hint
      default: lspSeverity = 1
      }
      var diag: [String: Any] = [
        "range": [
          "start": ["line": startL, "character": startC],
          "end": ["line": endL, "character": endC],
        ],
        "severity": lspSeverity,
        "message": message,
      ]
      if let src = d["source"] as? String {
        diag["source"] = src
      }
      diagObjects.append(diag)
    }

    let params = encodeLspJSON([
      "textDocument": ["uri": uri] as [String: Any],
      "range": [
        "start": ["line": startLine, "character": startColumn],
        "end": ["line": endLine, "character": endColumn],
      ],
      "context": ["diagnostics": diagObjects],
    ])

    codeActionWorkItems[uri] = enqueueLspRequest(
      languageId: language, fileUri: uri, method: "textDocument/codeAction", params: params
    ) { [weak self] response in
      guard let response else { return }
      DispatchQueue.main.async { [weak self] in
        guard let self else { return }
        guard self.latestCodeActionReq[uri] == requestId else { return }
        let (actions, carried) = self.monacoCodeActions(response, language: language, uri: uri)
        // Only the latest menu's actions can be picked.
        self.lspCodeActions = self.lspCodeActions.filter { $0.value.uri != uri }.merging(carried) { $1 }
        editor.resolveCodeActions(requestId: requestId, actions: actions)
      }
    }
  }

  /// Handles a rename request from the editor by forwarding it to the LSP.
  func handleRenameRequest(
    editor: EditorTab, requestId: UInt64, line: UInt32, character: UInt32, newName: String
  ) {
    guard let path = editor.filePath else { return }
    let uri = filePathToUri(path)
    latestRenameReq[uri] = requestId

    renameWorkItems[uri]?.cancel()

    let language = editor.lspLanguage
    let params = encodeLspJSON([
      "textDocument": ["uri": uri] as [String: Any],
      "position": ["line": line, "character": character],
      "newName": newName,
    ])

    renameWorkItems[uri] = enqueueLspRequest(
      languageId: language, fileUri: uri, method: "textDocument/rename", params: params
    ) { [weak self] response in
      guard let self, let response else { return }
      let workspaceEdit = response.data(using: .utf8)
        .flatMap { try? JSONSerialization.jsonObject(with: $0) }
        .flatMap(WorkspaceEdit.parse)
      let edits = self.parseRenameResponse(response)
      DispatchQueue.main.async { [weak self] in
        guard let self else { return }
        guard self.latestRenameReq[uri] == requestId else { return }
        // Monaco can only edit its own file: anything wider is applied here.
        if let workspaceEdit, !self.isLocalEdit(workspaceEdit, uri: uri) {
          editor.resolveRename(requestId: requestId, edits: [])
          self.applyWorkspaceEdit(workspaceEdit, root: self.userEditBoundary(forFileUri: uri), title: "Rename") {
            [weak self] outcome in
            self?.reportWorkspaceEdit(outcome, verb: "Renamed")
          }
        } else {
          editor.resolveRename(requestId: requestId, edits: edits)
        }
      }
    }
  }

  /// Handles a prepare rename request from the editor by forwarding it to the LSP.
  func handlePrepareRenameRequest(
    editor: EditorTab, requestId: UInt64, line: UInt32, character: UInt32
  ) {
    guard let path = editor.filePath else { return }
    let uri = filePathToUri(path)

    prepareRenameWorkItems[uri]?.cancel()

    let params = encodeLspJSON(positionParams(uri: uri, line: line, character: character))
    prepareRenameWorkItems[uri] = enqueueLspRequest(
      languageId: editor.lspLanguage, fileUri: uri, method: "textDocument/prepareRename", params: params
    ) { [weak self] response in
      let result = response.flatMap { self?.parsePrepareRenameResponse($0) }
      DispatchQueue.main.async {
        editor.resolvePrepareRename(
          requestId: requestId, range: result?.range, placeholder: result?.placeholder)
      }
    }
  }

  /// Handles Monaco's request to open a different file (cross-file go-to-definition).
  /// Fired by registerEditorOpener when the user actually Cmd+clicks.
  func handleOpenFileRequested(uri: String, line: UInt32, character: UInt32) {
    // Only open file:// URIs or plain paths (no scheme)
    guard uri.hasPrefix("file://") || !uri.contains("://") else {
      NSLog("Blocked opening non-file URI: \(uri)")
      return
    }
    let targetPath = uriToFilePath(uri)
    tabManager.addEditorTab(
      path: targetPath,
      projectDirectory: fileTreeRootPath,
      goToLine: line + 1,
      goToColumn: character + 1
    )
    if let targetEditor = findEditorTab(forPath: targetPath) {
      trackEditorTab(targetEditor, forPath: targetPath)
      lspDidOpenIfNeeded(path: targetPath)
    }
  }

  /// Send a request once everything already queued (didOpen, didChange) has
  /// been sent. Nothing waits for the answer: `completion` gets it (on a
  /// background thread), or nil without language servers. The ticket
  /// withdraws it; a withdrawn request's answer never arrives.
  @discardableResult
  func enqueueLspRequest(
    languageId: String, fileUri: String, method: String, params: String,
    completion: @escaping (String?) -> Void
  ) -> LSPRequestTicket {
    let ticket = LSPRequestTicket()
    let core = self.core
    lspQueue.async {
      guard !ticket.isCancelled else { return }
      let cancel = core.lspRequest(languageId: languageId, fileUri: fileUri, method: method, paramsJson: params) {
        response in
        guard !ticket.isCancelled else { return }
        completion(response)
      }
      ticket.sent(cancel: cancel)
    }
    return ticket
  }

  // MARK: LSP Response Parsing

  /// Completion items for Monaco: each with the text and range its
  /// `textEdit` (or the list's defaults) gives, snippet or not, its
  /// auto-import edits, and the server's sort and filter text.
  private func parseCompletionResponse(_ json: String) -> [MonacoCompletionItem] {
    guard let data = json.data(using: .utf8),
      let response = try? JSONSerialization.jsonObject(with: data)
    else { return [] }

    func monacoRange(_ range: LSPRange) -> MonacoRange {
      MonacoRange(
        startLine: UInt32(clamping: range.startLine), startColumn: UInt32(clamping: range.startCharacter),
        endLine: UInt32(clamping: range.endLine), endColumn: UInt32(clamping: range.endCharacter))
    }
    return LSPCompletion.parse(response).map { item in
      MonacoCompletionItem(
        label: item.label,
        kind: lspCompletionKindFromInt(item.kind),
        detail: item.detail,
        insertText: item.text,
        // Monaco's CompletionItemInsertTextRule.InsertAsSnippet.
        insertTextRules: item.isSnippet ? 4 : nil,
        range: item.range.map(monacoRange),
        replaceRange: item.replaceRange.map(monacoRange),
        additionalTextEdits: item.additionalTextEdits.map(Self.monacoEdit),
        sortText: item.sortText,
        filterText: item.filterText
      )
    }
  }

  private func parseHoverResponse(_ json: String) -> [MonacoHoverContent] {
    guard let data = json.data(using: .utf8),
      let response = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
      let contents = response["contents"]
    else { return [] }

    if let markup = contents as? [String: Any], let value = markup["value"] as? String {
      return [MonacoHoverContent(value: value)]
    } else if let str = contents as? String {
      return [MonacoHoverContent(value: str)]
    } else if let array = contents as? [[String: Any]] {
      return array.compactMap { item in
        guard let value = item["value"] as? String else { return nil }
        return MonacoHoverContent(value: value)
      }
    }
    return []
  }

  private func parseDefinitionResponse(_ json: String) -> (
    uri: String, line: UInt32, character: UInt32
  )? {
    guard let data = json.data(using: .utf8),
      let response = try? JSONSerialization.jsonObject(with: data)
    else { return nil }

    let location: [String: Any]?
    if let loc = response as? [String: Any], loc["uri"] != nil {
      location = loc
    } else if let array = response as? [[String: Any]], let first = array.first {
      if first["targetUri"] != nil {
        // LocationLink format
        guard let uri = first["targetUri"] as? String,
          let range = (first["targetSelectionRange"] as? [String: Any])
            ?? (first["targetRange"] as? [String: Any]),
          let start = range["start"] as? [String: Any],
          let line = (start["line"] as? NSNumber)?.uint32Value,
          let character = (start["character"] as? NSNumber)?.uint32Value
        else { return nil }
        return (uri: uri, line: line, character: character)
      }
      location = first
    } else {
      return nil
    }

    guard let loc = location,
      let uri = loc["uri"] as? String,
      let range = loc["range"] as? [String: Any],
      let start = range["start"] as? [String: Any],
      let line = (start["line"] as? NSNumber)?.uint32Value,
      let character = (start["character"] as? NSNumber)?.uint32Value
    else { return nil }
    return (uri: uri, line: line, character: character)
  }

  private func parseFormattingResponse(_ json: String) -> [MonacoTextEdit] {
    guard let data = json.data(using: .utf8),
      let array = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]]
    else { return [] }

    return array.compactMap { item in
      guard let range = item["range"] as? [String: Any],
        let start = range["start"] as? [String: Any],
        let end = range["end"] as? [String: Any],
        let startLine = (start["line"] as? NSNumber)?.uint32Value,
        let startChar = (start["character"] as? NSNumber)?.uint32Value,
        let endLine = (end["line"] as? NSNumber)?.uint32Value,
        let endChar = (end["character"] as? NSNumber)?.uint32Value,
        let newText = item["newText"] as? String
      else { return nil }
      return MonacoTextEdit(
        range: MonacoRange(
          startLine: startLine,
          startColumn: startChar,
          endLine: endLine,
          endColumn: endChar
        ),
        text: newText
      )
    }
  }

  private func parseSignatureHelpResponse(_ json: String) -> MonacoSignatureHelp? {
    guard let data = json.data(using: .utf8),
      let response = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
      let signatures = response["signatures"] as? [[String: Any]],
      !signatures.isEmpty
    else { return nil }

    let activeSignature = (response["activeSignature"] as? NSNumber)?.uint32Value ?? 0
    let activeParameter = (response["activeParameter"] as? NSNumber)?.uint32Value ?? 0

    let sigs: [MonacoSignatureInfo] = signatures.compactMap { sig in
      guard let label = sig["label"] as? String else { return nil }
      let doc: String?
      if let markup = sig["documentation"] as? [String: Any] {
        doc = markup["value"] as? String
      } else {
        doc = sig["documentation"] as? String
      }
      let params: [MonacoParameterInfo] =
        (sig["parameters"] as? [[String: Any]])?.compactMap { p in
          let pLabel: String
          if let labelStr = p["label"] as? String {
            pLabel = labelStr
          } else {
            pLabel = ""
          }
          let pDoc: String?
          if let markup = p["documentation"] as? [String: Any] {
            pDoc = markup["value"] as? String
          } else {
            pDoc = p["documentation"] as? String
          }
          return MonacoParameterInfo(label: pLabel, documentation: pDoc)
        } ?? []
      return MonacoSignatureInfo(label: label, documentation: doc, parameters: params)
    }

    return MonacoSignatureHelp(
      signatures: sigs, activeSignature: activeSignature, activeParameter: activeParameter)
  }

  private func parseReferencesResponse(_ json: String) -> [MonacoLocation] {
    guard let data = json.data(using: .utf8),
      let array = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]]
    else { return [] }

    return array.compactMap { item in
      guard let uri = item["uri"] as? String,
        let range = item["range"] as? [String: Any],
        let start = range["start"] as? [String: Any],
        let end = range["end"] as? [String: Any],
        let startLine = (start["line"] as? NSNumber)?.uint32Value,
        let startChar = (start["character"] as? NSNumber)?.uint32Value,
        let endLine = (end["line"] as? NSNumber)?.uint32Value,
        let endChar = (end["character"] as? NSNumber)?.uint32Value
      else { return nil }
      return MonacoLocation(
        uri: uri,
        range: MonacoRange(
          startLine: startLine,
          startColumn: startChar,
          endLine: endLine,
          endColumn: endChar
        )
      )
    }
  }

  private func parseRenameResponse(_ json: String) -> [MonacoWorkspaceTextEdit] {
    guard let data = json.data(using: .utf8),
      let response = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
    else { return [] }

    var edits: [MonacoWorkspaceTextEdit] = []

    // Handle WorkspaceEdit.changes format.
    if let changes = response["changes"] as? [String: [[String: Any]]] {
      for (changeUri, textEdits) in changes {
        for te in textEdits {
          guard let range = te["range"] as? [String: Any],
            let start = range["start"] as? [String: Any],
            let end = range["end"] as? [String: Any],
            let startLine = (start["line"] as? NSNumber)?.uint32Value,
            let startChar = (start["character"] as? NSNumber)?.uint32Value,
            let endLine = (end["line"] as? NSNumber)?.uint32Value,
            let endChar = (end["character"] as? NSNumber)?.uint32Value,
            let newText = te["newText"] as? String
          else { continue }
          edits.append(
            MonacoWorkspaceTextEdit(
              uri: changeUri,
              range: MonacoRange(
                startLine: startLine,
                startColumn: startChar,
                endLine: endLine,
                endColumn: endChar
              ),
              text: newText
            ))
        }
      }
    }

    // Also handle documentChanges format.
    if edits.isEmpty, let docChanges = response["documentChanges"] as? [[String: Any]] {
      for change in docChanges {
        guard let textDoc = change["textDocument"] as? [String: Any],
          let changeUri = textDoc["uri"] as? String,
          let textEdits = change["edits"] as? [[String: Any]]
        else { continue }
        for te in textEdits {
          guard let range = te["range"] as? [String: Any],
            let start = range["start"] as? [String: Any],
            let end = range["end"] as? [String: Any],
            let startLine = (start["line"] as? NSNumber)?.uint32Value,
            let startChar = (start["character"] as? NSNumber)?.uint32Value,
            let endLine = (end["line"] as? NSNumber)?.uint32Value,
            let endChar = (end["character"] as? NSNumber)?.uint32Value,
            let newText = te["newText"] as? String
          else { continue }
          edits.append(
            MonacoWorkspaceTextEdit(
              uri: changeUri,
              range: MonacoRange(
                startLine: startLine,
                startColumn: startChar,
                endLine: endLine,
                endColumn: endChar
              ),
              text: newText
            ))
        }
      }
    }

    return edits
  }

  private func parsePrepareRenameResponse(_ json: String) -> (
    range: MonacoRange, placeholder: String
  )? {
    guard let data = json.data(using: .utf8),
      let response = try? JSONSerialization.jsonObject(with: data)
    else { return nil }

    // Format 1: { range: {...}, placeholder: "..." }
    if let obj = response as? [String: Any],
      let placeholder = obj["placeholder"] as? String,
      let range = obj["range"] as? [String: Any],
      let start = range["start"] as? [String: Any],
      let end = range["end"] as? [String: Any],
      let startLine = (start["line"] as? NSNumber)?.uint32Value,
      let startChar = (start["character"] as? NSNumber)?.uint32Value,
      let endLine = (end["line"] as? NSNumber)?.uint32Value,
      let endChar = (end["character"] as? NSNumber)?.uint32Value
    {
      return (
        range: MonacoRange(
          startLine: startLine, startColumn: startChar, endLine: endLine, endColumn: endChar),
        placeholder: placeholder
      )
    }

    // Format 2: plain Range { start: {...}, end: {...} }
    if let obj = response as? [String: Any],
      let start = obj["start"] as? [String: Any],
      let end = obj["end"] as? [String: Any],
      let startLine = (start["line"] as? NSNumber)?.uint32Value,
      let startChar = (start["character"] as? NSNumber)?.uint32Value,
      let endLine = (end["line"] as? NSNumber)?.uint32Value,
      let endChar = (end["character"] as? NSNumber)?.uint32Value
    {
      return (
        range: MonacoRange(
          startLine: startLine, startColumn: startChar, endLine: endLine, endColumn: endChar),
        placeholder: ""
      )
    }

    // Format 3: { defaultBehavior: true } — server supports rename but no range info
    if let obj = response as? [String: Any],
      let defaultBehavior = obj["defaultBehavior"] as? Bool,
      defaultBehavior
    {
      return nil
    }

    return nil
  }

  // MARK: LSP Helpers

  /// Maps an LSP CompletionItemKind integer to a Monaco CompletionItemKind value.
  private func lspCompletionKindFromInt(_ kind: Int) -> UInt32 {
    switch kind {
    case 1: return 18  // Text
    case 2: return 0  // Method
    case 3: return 1  // Function
    case 4: return 2  // Constructor
    case 5: return 3  // Field
    case 6: return 4  // Variable
    case 7: return 5  // Class
    case 8: return 7  // Interface
    case 9: return 8  // Module
    case 10: return 9  // Property
    case 11: return 12  // Unit
    case 12: return 13  // Value
    case 13: return 15  // Enum
    case 14: return 17  // Keyword
    case 15: return 27  // Snippet
    case 16: return 19  // Color
    case 17: return 20  // File
    case 18: return 21  // Reference
    case 19: return 23  // Folder
    case 20: return 16  // EnumMember
    case 21: return 14  // Constant
    case 22: return 6  // Struct
    case 23: return 10  // Event
    case 24: return 11  // Operator
    case 25: return 24  // TypeParameter
    default: return 18  // Text
    }
  }

  /// Converts an absolute file path to a file:// URI.
  func filePathToUri(_ path: String) -> String {
    return URL(fileURLWithPath: path).absoluteString
  }

  /// Extracts the file path from a file:// URI.
  func uriToFilePath(_ uri: String) -> String {
    if let url = URL(string: uri), url.scheme == "file" {
      return url.path
    }
    return uri
  }

  /// Convenience builder for `{textDocument, position}` request params.
  fileprivate func positionParams(uri: String, line: UInt32, character: UInt32) -> [String: Any] {
    return [
      "textDocument": ["uri": uri] as [String: Any],
      "position": ["line": line, "character": character],
    ]
  }

  /// Finds the editor tab that has the given file path open.
  /// Uses the editorTabsByPath dictionary for O(1) lookup, falling back
  /// to a linear scan if the dictionary is out of sync.
  func findEditorTab(forPath path: String) -> EditorTab? {
    if let editor = editorTabsByPath[path] {
      guard tabManager.ownsEditor(editor), editor.filePath == path else {
        editorTabsByPath.removeValue(forKey: path)
        return findEditorTab(forPath: path)
      }
      return editor
    }
    // Fallback: linear scan (keeps behavior correct if dictionary is stale).
    for tab in tabManager.allSurfaces {
      if case .editor(let editor) = tab, editor.filePath == path {
        // Re-track it for future lookups.
        editorTabsByPath[path] = editor
        return editor
      }
    }
    return nil
  }
}
