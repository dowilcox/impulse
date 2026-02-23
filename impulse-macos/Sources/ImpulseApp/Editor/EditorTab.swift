import AppKit
import WebKit
import os.log

// MARK: - WeakScriptMessageHandler

/// A thin proxy that prevents WKUserContentController from creating a strong
/// retain cycle with its message handler.  WKUserContentController retains its
/// handlers strongly; by interposing this proxy, the real handler (EditorTab)
/// is held only weakly and can be deallocated normally.
private class WeakScriptMessageHandler: NSObject, WKScriptMessageHandler {
    weak var delegate: WKScriptMessageHandler?

    init(delegate: WKScriptMessageHandler) {
        self.delegate = delegate
    }

    func userContentController(
        _ userContentController: WKUserContentController,
        didReceive message: WKScriptMessage
    ) {
        delegate?.userContentController(userContentController, didReceive: message)
    }
}

// MARK: - EditorTab

/// Wraps a WKWebView hosting the Monaco code editor.
///
/// Communication with the embedded editor uses the bidirectional JSON protocol
/// defined in `EditorProtocol.swift`, matching the Rust `impulse-editor` crate.
class EditorTab: NSView, WKScriptMessageHandler, WKNavigationDelegate {

    // MARK: Properties

    /// Absolute path to the file currently open in this editor tab, or nil if untitled.
    private(set) var filePath: String?

    /// Current editor content, kept in sync via `ContentChanged` events.
    private(set) var content: String = ""

    /// Monaco language identifier for the current file.
    private(set) var language: String = "plaintext"

    /// Whether the content has been modified since the last save.
    private(set) var isModified: Bool = false

    /// The sidebar root directory that was active when this editor tab was opened.
    /// Restored when the user switches back to this tab.
    var projectDirectory: String?

    /// The WKWebView hosting Monaco.
    private(set) var webView: WKWebView!

    /// Whether the Monaco editor has fired its `Ready` event.
    private var isEditorReady: Bool = false

    /// Commands queued before the editor was ready.
    private var pendingCommands: [EditorCommand] = []

    /// JSON encoder configured for the protocol wire format.
    private let jsonEncoder: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [] // compact JSON
        return encoder
    }()

    /// JSON decoder for incoming events.
    private let jsonDecoder = JSONDecoder()

    private static let log = OSLog(subsystem: "dev.impulse.Impulse", category: "EditorTab")

    // File watching for external changes
    private var fileWatchDescriptor: Int32 = -1
    private var fileWatchSource: DispatchSourceFileSystemObject?
    private var fileWatchDebounce: DispatchWorkItem?
    /// When true, the next ContentChanged event will not mark the file as modified.
    private var suppressNextModify: Bool = false

    // MARK: Initialisation

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        setupWebView()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        setupWebView()
    }

    private func setupWebView() {
        // Try to claim a pre-warmed WebView from the pool. If available,
        // Monaco is already loaded and we can skip loadEditor() entirely.
        let proxy = WeakScriptMessageHandler(delegate: self)
        if let warmed = EditorWebViewPool.shared.claim(newHandler: self, weakProxy: proxy) {
            warmed.translatesAutoresizingMaskIntoConstraints = false
            addSubview(warmed)
            NSLayoutConstraint.activate([
                warmed.topAnchor.constraint(equalTo: topAnchor),
                warmed.bottomAnchor.constraint(equalTo: bottomAnchor),
                warmed.leadingAnchor.constraint(equalTo: leadingAnchor),
                warmed.trailingAnchor.constraint(equalTo: trailingAnchor),
            ])
            self.webView = warmed
            self.isEditorReady = true
            return
        }

        // Fall back to creating a new WebView.
        let config = WKWebViewConfiguration()

        // Register the script message handler on the "impulse" channel via
        // a weak proxy to avoid a retain cycle (WKUserContentController
        // retains its handlers strongly).
        config.userContentController.add(WeakScriptMessageHandler(delegate: self), name: "impulse")

        let pagePrefs = WKWebpagePreferences()
        pagePrefs.allowsContentJavaScript = true
        config.defaultWebpagePreferences = pagePrefs

        let wv = WKWebView(frame: bounds, configuration: config)
        wv.navigationDelegate = self
        wv.translatesAutoresizingMaskIntoConstraints = false

        // Make the WebView background transparent so it does not flash white
        // before Monaco renders its own background colour.
        wv.underPageBackgroundColor = .clear

        addSubview(wv)
        NSLayoutConstraint.activate([
            wv.topAnchor.constraint(equalTo: topAnchor),
            wv.bottomAnchor.constraint(equalTo: bottomAnchor),
            wv.leadingAnchor.constraint(equalTo: leadingAnchor),
            wv.trailingAnchor.constraint(equalTo: trailingAnchor),
        ])

        self.webView = wv
    }

    // MARK: Loading

    /// Extract Monaco assets via the FFI bridge and load the editor HTML.
    /// This is a no-op if the editor is already ready (e.g. from a pre-warmed WebView).
    func loadEditor() {
        guard !isEditorReady else { return }

        switch ImpulseCore.ensureMonacoExtracted() {
        case .failure(let error):
            os_log(.error, log: Self.log, "Failed to extract Monaco: %{public}@", error.message)
            return
        case .success(let pathString):
            let monacoDir = URL(fileURLWithPath: pathString, isDirectory: true)
            let editorHTML = monacoDir.appendingPathComponent("editor.html")
            webView.loadFileURL(editorHTML, allowingReadAccessTo: monacoDir)
        }
    }

    // MARK: WKScriptMessageHandler

    func userContentController(
        _ userContentController: WKUserContentController,
        didReceive message: WKScriptMessage
    ) {
        guard message.name == "impulse" else { return }

        guard let body = message.body as? String,
              let data = body.data(using: .utf8) else {
            os_log(.error, log: Self.log, "Received non-string message from Monaco")
            return
        }

        let event: EditorEvent
        do {
            event = try jsonDecoder.decode(EditorEvent.self, from: data)
        } catch {
            os_log(.error, log: Self.log, "Failed to decode EditorEvent: %{public}@", error.localizedDescription)
            return
        }

        handleEvent(event)
    }

    private func handleEvent(_ event: EditorEvent) {
        switch event {
        case .ready:
            isEditorReady = true

            // Flush any commands that were queued before the editor was ready.
            // This includes any openFile command from openFile() called before ready.
            let queued = pendingCommands
            pendingCommands.removeAll()

            // If a file was set before the editor was ready AND no openFile command
            // is already queued, send it now.
            let hasQueuedOpen = queued.contains { cmd in
                if case .openFile = cmd { return true }
                return false
            }

            for cmd in queued {
                sendCommand(cmd)
            }

            if !hasQueuedOpen, let path = filePath {
                sendCommand(.openFile(filePath: path, content: content, language: language))
            }

        case .fileOpened:
            NotificationCenter.default.post(
                name: .editorFileOpened,
                object: self,
                userInfo: ["filePath": filePath ?? ""]
            )

        case let .contentChanged(newContent, _):
            content = newContent
            if suppressNextModify {
                suppressNextModify = false
            } else {
                isModified = true
            }
            NotificationCenter.default.post(
                name: .editorContentChanged,
                object: self,
                userInfo: ["filePath": filePath ?? ""]
            )

        case let .cursorMoved(line, column):
            NotificationCenter.default.post(
                name: .editorCursorMoved,
                object: self,
                userInfo: ["line": line, "column": column]
            )

        case .saveRequested:
            // Route through the main save pipeline so format-on-save, LSP
            // notifications, and other post-save actions run correctly.
            NotificationCenter.default.post(name: .impulseSaveFile, object: self)

        case let .completionRequested(requestId, line, character):
            NotificationCenter.default.post(
                name: .editorCompletionRequested,
                object: self,
                userInfo: [
                    "requestId": requestId,
                    "line": line,
                    "character": character,
                ]
            )

        case let .hoverRequested(requestId, line, character):
            NotificationCenter.default.post(
                name: .editorHoverRequested,
                object: self,
                userInfo: [
                    "requestId": requestId,
                    "line": line,
                    "character": character,
                ]
            )

        case let .definitionRequested(requestId, line, character):
            NotificationCenter.default.post(
                name: .editorDefinitionRequested,
                object: self,
                userInfo: ["requestId": requestId, "line": line, "character": character]
            )

        case let .openFileRequested(uri, line, character):
            NotificationCenter.default.post(
                name: .editorOpenFileRequested,
                object: self,
                userInfo: ["uri": uri, "line": line, "character": character]
            )

        case let .focusChanged(focused):
            NotificationCenter.default.post(
                name: .editorFocusChanged,
                object: self,
                userInfo: ["focused": focused]
            )

        case let .formattingRequested(requestId, tabSize, insertSpaces):
            NotificationCenter.default.post(
                name: .editorFormattingRequested,
                object: self,
                userInfo: [
                    "requestId": requestId,
                    "tabSize": tabSize,
                    "insertSpaces": insertSpaces,
                ]
            )

        case let .signatureHelpRequested(requestId, line, character):
            NotificationCenter.default.post(
                name: .editorSignatureHelpRequested,
                object: self,
                userInfo: [
                    "requestId": requestId,
                    "line": line,
                    "character": character,
                ]
            )

        case let .referencesRequested(requestId, line, character):
            NotificationCenter.default.post(
                name: .editorReferencesRequested,
                object: self,
                userInfo: [
                    "requestId": requestId,
                    "line": line,
                    "character": character,
                ]
            )

        case let .codeActionRequested(requestId, startLine, startColumn, endLine, endColumn, diagnostics):
            let diagDicts: [[String: Any]] = diagnostics.map { d in
                [
                    "severity": d.severity,
                    "startLine": d.startLine,
                    "startColumn": d.startColumn,
                    "endLine": d.endLine,
                    "endColumn": d.endColumn,
                    "message": d.message,
                    "source": d.source as Any,
                ]
            }
            NotificationCenter.default.post(
                name: .editorCodeActionRequested,
                object: self,
                userInfo: [
                    "requestId": requestId,
                    "startLine": startLine,
                    "startColumn": startColumn,
                    "endLine": endLine,
                    "endColumn": endColumn,
                    "diagnostics": diagDicts,
                ]
            )

        case let .renameRequested(requestId, line, character, newName):
            NotificationCenter.default.post(
                name: .editorRenameRequested,
                object: self,
                userInfo: [
                    "requestId": requestId,
                    "line": line,
                    "character": character,
                    "newName": newName,
                ]
            )

        case let .prepareRenameRequested(requestId, line, character):
            NotificationCenter.default.post(
                name: .editorPrepareRenameRequested,
                object: self,
                userInfo: [
                    "requestId": requestId,
                    "line": line,
                    "character": character,
                ]
            )
        }
    }

    // MARK: WKNavigationDelegate

    func webView(
        _ webView: WKWebView,
        didFinish navigation: WKNavigation!
    ) {
        os_log(.info, log: Self.log, "Monaco WebView finished loading")
    }

    func webView(
        _ webView: WKWebView,
        didFail navigation: WKNavigation!,
        withError error: Error
    ) {
        os_log(.error, log: Self.log, "Monaco WebView navigation failed: %{public}@", error.localizedDescription)
    }

    func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        guard let url = navigationAction.request.url else {
            decisionHandler(.cancel)
            return
        }
        // Only allow file:// navigations (Monaco assets) and about:blank
        if url.scheme == "file" || url.scheme == "about" {
            decisionHandler(.allow)
        } else {
            os_log(.info, log: Self.log, "Blocked navigation to non-file URL: %{public}@", url.absoluteString)
            decisionHandler(.cancel)
        }
    }

    // MARK: Command Sending

    /// Send a command to the Monaco editor.
    ///
    /// If the editor is not yet ready, the command is queued and will be sent
    /// once the `Ready` event is received.
    func sendCommand(_ command: EditorCommand) {
        guard isEditorReady else {
            pendingCommands.append(command)
            return
        }

        let jsonData: Data
        do {
            jsonData = try jsonEncoder.encode(command)
        } catch {
            os_log(.error, log: Self.log, "Failed to encode EditorCommand: %{public}@", error.localizedDescription)
            return
        }

        guard let jsonString = String(data: jsonData, encoding: .utf8) else {
            os_log(.error, log: Self.log, "Failed to convert command JSON to string")
            return
        }

        // Escape characters that are special inside a JS single-quoted string literal.
        var escaped = jsonString
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "'", with: "\\'")
            .replacingOccurrences(of: "\n", with: "\\n")
            .replacingOccurrences(of: "\r", with: "\\r")
            .replacingOccurrences(of: "\t", with: "\\t")
            .replacingOccurrences(of: "\0", with: "\\0")
            .replacingOccurrences(of: "\u{2028}", with: "\\u2028")
            .replacingOccurrences(of: "\u{2029}", with: "\\u2029")

        // Escape any remaining ASCII control characters (below U+0020) that
        // were not covered above. These are invalid inside JS string literals
        // and could cause syntax errors or injection issues.
        var sanitized = ""
        sanitized.reserveCapacity(escaped.count)
        for scalar in escaped.unicodeScalars {
            if scalar.value < 0x20 && scalar != "\n" && scalar != "\r" && scalar != "\t" {
                sanitized += String(format: "\\u%04x", scalar.value)
            } else {
                sanitized += String(scalar)
            }
        }
        escaped = sanitized

        let script = "impulseReceiveCommand('\(escaped)')"
        guard let webView else { return }
        webView.evaluateJavaScript(script) { _, error in
            if let error = error {
                os_log(.error, log: Self.log, "evaluateJavaScript failed: %{public}@", error.localizedDescription)
            }
        }
    }

    // MARK: Public API

    /// Open a file in the editor.
    func openFile(path: String, content: String, language: String) {
        self.filePath = path
        self.content = content
        self.language = language
        self.isModified = false

        sendCommand(.openFile(filePath: path, content: content, language: language))
        startFileWatching()
    }

    /// Save the current content to the file at `filePath`.
    @discardableResult
    func saveFile() -> Bool {
        guard let path = filePath else {
            os_log(.error, log: Self.log, "Cannot save: no file path set")
            return false
        }

        let contentToSave = content
        do {
            try contentToSave.write(toFile: path, atomically: true, encoding: .utf8)
            isModified = false
            return true
        } catch {
            os_log(.error, log: Self.log, "Failed to save file %{public}@: %{public}@", path, error.localizedDescription)
            return false
        }
    }

    /// Fetch the latest content from Monaco and then call `completion`.
    /// This is necessary because content changes are debounced in JS, so
    /// the Swift `content` property may be stale when a save is triggered
    /// via the menu (Cmd+S) rather than through Monaco's own save handler.
    func fetchContentAndSave(completion: @escaping (Bool) -> Void) {
        guard let path = filePath else {
            completion(false)
            return
        }

        guard isEditorReady else {
            // Editor not ready, save whatever we have
            completion(saveFile())
            return
        }

        webView.evaluateJavaScript("editor.getValue()") { [weak self] result, error in
            guard let self else { completion(false); return }
            if let latest = result as? String {
                self.content = latest
            }
            let contentToSave = self.content
            DispatchQueue.global(qos: .userInitiated).async { [weak self] in
                do {
                    try contentToSave.write(toFile: path, atomically: true, encoding: .utf8)
                    DispatchQueue.main.async {
                        self?.isModified = false
                        completion(true)
                    }
                } catch {
                    os_log(.error, log: Self.log, "Failed to save file %{public}@: %{public}@", path, error.localizedDescription)
                    DispatchQueue.main.async {
                        completion(false)
                    }
                }
            }
        }
    }

    /// Apply a Monaco theme to the editor.
    func applyTheme(_ theme: MonacoThemeDefinition) {
        sendCommand(.setTheme(theme: theme))
    }

    /// Apply editor settings (font, tab size, etc.).
    func applySettings(_ options: EditorOptions) {
        sendCommand(.updateSettings(options: options))
    }

    /// Resolve a pending definition request. Pass nil uri for "not found".
    func resolveDefinition(requestId: UInt64, uri: String?, line: UInt32?, column: UInt32?) {
        sendCommand(.resolveDefinition(requestId: requestId, uri: uri, line: line, column: column))
    }

    /// Navigate the editor cursor to the given line and column.
    func goToPosition(line: UInt32, column: UInt32) {
        sendCommand(.goToPosition(line: line, column: column))
    }

    /// Set the editor to read-only or read-write mode.
    func setReadOnly(_ readOnly: Bool) {
        sendCommand(.setReadOnly(readOnly: readOnly))
    }

    /// Apply git diff decorations in the gutter.
    func applyDiffDecorations(_ decorations: [DiffDecoration]) {
        sendCommand(.applyDiffDecorations(decorations: decorations))
    }

    /// Apply LSP diagnostics (errors, warnings) as markers.
    func applyDiagnostics(uri: String, markers: [MonacoDiagnostic]) {
        sendCommand(.applyDiagnostics(uri: uri, markers: markers))
    }

    /// Resolve an in-flight completion request with items from the LSP server.
    func resolveCompletions(requestId: UInt64, items: [MonacoCompletionItem]) {
        sendCommand(.resolveCompletions(requestId: requestId, items: items))
    }

    /// Resolve an in-flight hover request with content from the LSP server.
    func resolveHover(requestId: UInt64, contents: [MonacoHoverContent]) {
        sendCommand(.resolveHover(requestId: requestId, contents: contents))
    }

    /// Resolve an in-flight formatting request with text edits from the LSP server.
    func resolveFormatting(requestId: UInt64, edits: [MonacoTextEdit]) {
        sendCommand(.resolveFormatting(requestId: requestId, edits: edits))
    }

    /// Resolve an in-flight signature help request.
    func resolveSignatureHelp(requestId: UInt64, signatureHelp: MonacoSignatureHelp?) {
        sendCommand(.resolveSignatureHelp(requestId: requestId, signatureHelp: signatureHelp))
    }

    /// Resolve an in-flight references request with locations from the LSP server.
    func resolveReferences(requestId: UInt64, locations: [MonacoLocation]) {
        sendCommand(.resolveReferences(requestId: requestId, locations: locations))
    }

    /// Resolve an in-flight code action request with actions from the LSP server.
    func resolveCodeActions(requestId: UInt64, actions: [MonacoCodeAction]) {
        sendCommand(.resolveCodeActions(requestId: requestId, actions: actions))
    }

    /// Resolve an in-flight rename request with workspace edits from the LSP server.
    func resolveRename(requestId: UInt64, edits: [MonacoWorkspaceTextEdit]) {
        sendCommand(.resolveRename(requestId: requestId, edits: edits))
    }

    /// Resolve an in-flight prepare rename request with range and placeholder.
    func resolvePrepareRename(requestId: UInt64, range: MonacoRange?, placeholder: String?) {
        sendCommand(.resolvePrepareRename(requestId: requestId, range: range, placeholder: placeholder))
    }

    /// Make the WebView the first responder to accept keyboard input.
    func focus() {
        guard let webView else { return }
        window?.makeFirstResponder(webView)
    }

    // MARK: - File Watching

    /// Start watching the current file for external modifications.
    private func startFileWatching() {
        stopFileWatching()

        guard let path = filePath else { return }

        let fd = open(path, O_EVTONLY)
        guard fd >= 0 else {
            os_log(.info, log: Self.log, "Cannot watch file %{public}@ (errno %d)", path, errno)
            return
        }
        fileWatchDescriptor = fd

        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: fd,
            eventMask: [.write, .rename],
            queue: .main
        )

        source.setEventHandler { [weak self] in
            self?.handleFileChangeEvent()
        }

        source.setCancelHandler { [fd] in
            close(fd)
        }

        fileWatchSource = source
        source.resume()
    }

    /// Stop the current file watcher.
    private func stopFileWatching() {
        fileWatchDebounce?.cancel()
        fileWatchDebounce = nil

        if let source = fileWatchSource {
            source.cancel()
            fileWatchSource = nil
            fileWatchDescriptor = -1
        } else if fileWatchDescriptor >= 0 {
            close(fileWatchDescriptor)
            fileWatchDescriptor = -1
        }
    }

    /// Debounced handler for file change events.
    private func handleFileChangeEvent() {
        fileWatchDebounce?.cancel()
        let work = DispatchWorkItem { [weak self] in
            self?.reloadIfUnmodified()
        }
        fileWatchDebounce = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3, execute: work)
    }

    /// Reload the file content if the editor has no unsaved changes.
    private func reloadIfUnmodified() {
        guard !isModified, let path = filePath else { return }

        let newContent: String
        do {
            newContent = try String(contentsOfFile: path, encoding: .utf8)
        } catch {
            os_log(.error, log: Self.log, "Failed to reload file '%{public}@': %{public}@", path, error.localizedDescription)
            return
        }
        guard newContent != content else {
            // Content unchanged but the inode may have been replaced (atomic write).
            // Restart the watcher so the fd tracks the current inode.
            startFileWatching()
            return
        }

        suppressNextModify = true
        content = newContent
        sendCommand(.openFile(filePath: path, content: newContent, language: language))
        // Restart the watcher: after an atomic write (temp → rename) the old fd
        // points to a stale inode.  Re-opening gives us the new one.
        startFileWatching()
    }

    // MARK: Cleanup

    /// Explicitly release resources held by the WebView. Must be called before
    /// the tab is removed from the tab list to ensure the WKWebView and its
    /// associated JavaScript context are torn down promptly.
    func cleanup() {
        stopFileWatching()
        webView?.configuration.userContentController.removeScriptMessageHandler(forName: "impulse")
        webView?.navigationDelegate = nil
        webView?.stopLoading()
        webView?.removeFromSuperview()
        webView = nil
    }

    deinit {
        // Belt-and-suspenders: clean up anything that wasn't already handled
        // by an explicit cleanup() call.
        stopFileWatching()
        if let wv = webView {
            wv.configuration.userContentController.removeScriptMessageHandler(forName: "impulse")
            wv.navigationDelegate = nil
        }
    }
}
