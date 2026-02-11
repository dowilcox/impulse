import AppKit
import WebKit
import os.log

// MARK: - Notifications

extension Notification.Name {
    /// Posted when the cursor position changes. The `userInfo` dictionary contains
    /// `"line"` and `"column"` as `UInt32` values.
    static let editorCursorMoved = Notification.Name("impulse.editorCursorMoved")

    /// Posted when the editor content is modified. The `userInfo` dictionary contains
    /// `"filePath"` as a `String`.
    static let editorContentChanged = Notification.Name("impulse.editorContentChanged")

    /// Posted when a completion request is received from Monaco. The `userInfo`
    /// dictionary contains `"requestId"`, `"line"`, and `"character"`.
    static let editorCompletionRequested = Notification.Name("impulse.editorCompletionRequested")

    /// Posted when a hover request is received from Monaco. The `userInfo`
    /// dictionary contains `"requestId"`, `"line"`, and `"character"`.
    static let editorHoverRequested = Notification.Name("impulse.editorHoverRequested")

    /// Posted when a go-to-definition request is received. The `userInfo`
    /// dictionary contains `"line"` and `"character"`.
    static let editorDefinitionRequested = Notification.Name("impulse.editorDefinitionRequested")

    /// Posted when the editor focus state changes. The `userInfo` dictionary
    /// contains `"focused"` as a `Bool`.
    static let editorFocusChanged = Notification.Name("impulse.editorFocusChanged")
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
        let config = WKWebViewConfiguration()

        // Register the script message handler on the "impulse" channel.
        // Monaco posts events via: window.webkit.messageHandlers.impulse.postMessage(json)
        config.userContentController.add(self, name: "impulse")

        let preferences = WKPreferences()
        preferences.setValue(true, forKey: "javaScriptEnabled")
        config.preferences = preferences

        // Allow file:// access for loading Monaco assets.
        config.preferences.setValue(true, forKey: "allowFileAccessFromFileURLs")

        let wv = WKWebView(frame: bounds, configuration: config)
        wv.navigationDelegate = self
        wv.translatesAutoresizingMaskIntoConstraints = false

        // Make the WebView background transparent so it does not flash white
        // before Monaco renders its own background colour.
        wv.setValue(false, forKey: "drawsBackground")

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
    func loadEditor() {
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

        case let .contentChanged(newContent, _):
            content = newContent
            isModified = true
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
            saveFile()

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

        case let .definitionRequested(line, character):
            NotificationCenter.default.post(
                name: .editorDefinitionRequested,
                object: self,
                userInfo: ["line": line, "character": character]
            )

        case let .focusChanged(focused):
            NotificationCenter.default.post(
                name: .editorFocusChanged,
                object: self,
                userInfo: ["focused": focused]
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

        // Escape single quotes and backslashes for embedding in the JS string literal.
        let escaped = jsonString
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "'", with: "\\'")

        let script = "impulseReceiveCommand('\(escaped)')"
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
    }

    /// Save the current content to the file at `filePath`.
    @discardableResult
    func saveFile() -> Bool {
        guard let path = filePath else {
            os_log(.error, log: Self.log, "Cannot save: no file path set")
            return false
        }

        do {
            try content.write(toFile: path, atomically: true, encoding: .utf8)
            isModified = false
            return true
        } catch {
            os_log(.error, log: Self.log, "Failed to save file %{public}@: %{public}@", path, error.localizedDescription)
            return false
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

    /// Make the WebView the first responder to accept keyboard input.
    func focus() {
        window?.makeFirstResponder(webView)
    }

    // MARK: Cleanup

    deinit {
        webView?.configuration.userContentController.removeScriptMessageHandler(forName: "impulse")
    }
}
