import AppKit
import ImpulseKit
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
    var filePath: String?

    /// Current editor content, kept in sync via `ContentChanged` events.
    private(set) var content: String = "" {
        didSet { contentGeneration &+= 1 }
    }
    /// Bumped whenever `content` is set, to tell whether a copy taken for
    /// work off the main thread is still current.
    private var contentGeneration = 0
    /// `OpenFile` commands Monaco hasn't confirmed yet, and the text the
    /// last one sent. Content changes arriving meanwhile were made to text
    /// the open replaces, so they're dropped.
    private var opensInFlight = 0
    private var openSentText: String?
    /// Waiting for Monaco's whole buffer after a change didn't fit the copy.
    private var isResyncingContent = false
    /// The settings and theme last sent, to set up a reloaded page.
    private var editorOptions: EditorOptions?
    private var editorTheme: MonacoThemeDefinition?

    /// Monaco language identifier for the current file.
    private(set) var language: String = "plaintext"

    /// LSP language identifier (e.g. "typescriptreact" for .tsx, "javascriptreact" for .jsx).
    /// Falls back to `language` when not explicitly set.
    private(set) var lspLanguage: String = "plaintext"

    /// Whether the content has been modified since the last save.
    private(set) var isModified: Bool = false {
        didSet {
            guard isModified != oldValue else { return }
            NotificationCenter.default.post(name: .editorDirtyStateChanged, object: self)
        }
    }

    /// The sidebar root directory that was active when this editor tab was opened.
    /// Restored when the user switches back to this tab.
    var projectDirectory: String?

    /// CWD captured at the time of Cmd+N for untitled editors; used as the
    /// default directory in the save-as dialog.
    var untitledCwd: String?

    /// The WKWebView hosting Monaco. Nil after `cleanup()` has run.
    private(set) var webView: WKWebView?

    /// Whether the Monaco editor has fired its `Ready` event.
    private var isEditorReady: Bool = false

    /// When set, read-only mode is applied as soon as Monaco reports ready,
    /// before any content is rendered, to avoid a flash of editable content.
    private var pendingReadOnly: Bool = false

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

    /// The file starts with a UTF-8 byte-order mark (written back on save).
    private(set) var hasBOM = false
    /// What was on disk when the buffer was last loaded or saved, to tell
    /// someone else's change (an agent, git) from our own.
    private var diskStamp: TextFile.Stamp?
    private var diskTextHash: Int?
    /// The disk text a "changed on disk" notice was already shown for.
    private var noticedDiskHash: Int?
    /// The text the last successful save wrote.
    private(set) var lastWrittenText: String?
    /// Formatters and commands on save rewriting the file just saved: the
    /// watcher leaves their writes to `adoptDiskChanges`, though never past
    /// `rewritesMutedUntil` (should one not report back).
    private var pendingRewrites = 0
    private var rewritesMutedUntil = Date.distantPast
    /// Longer than the formatters' and reloading commands' own time limit.
    static let rewriteMuteLimit: TimeInterval = 90
    private var isRewriting: Bool { pendingRewrites > 0 && Date() < rewritesMutedUntil }
    /// Bumped whenever what's on disk is recorded (load, save, reload, a
    /// formatter's result), so a read that started before is known to be
    /// older; and the latest reload's number, so an older one gives way.
    private var diskGeneration = 0
    private var reloadSequence = 0
    /// When the page's content process ended lately. Past `maxPageReloads`
    /// in `pageReloadWindow`, the page waits (behind a notice) to be
    /// reloaded by hand or when the tab is shown again.
    private var pageTerminations: [Date] = []
    private static let maxPageReloads = 2
    private static let pageReloadWindow: TimeInterval = 60
    private var needsPageReload = false
    private var stoppedNotice: NSView?
    /// Files over this size open read-only, and a change on disk shows a
    /// notice instead of reloading them on every write.
    static let largeFileSize = 5 * 1024 * 1024
    /// Larger than that.
    var isLargeFile = false
    /// The "changed on disk" notice is up for a large file (until it's
    /// reloaded).
    private var largeFileNoticeShown = false
    /// Asked before a save would overwrite changes made on disk since the
    /// buffer was loaded; `proceed(true)` overwrites. Without it, saves
    /// overwrite.
    var resolveSaveConflict: ((EditorTab, _ proceed: @escaping (Bool) -> Void) -> Void)?

    /// Debounce work item for cursor move notifications.
    private var cursorDebounceWork: DispatchWorkItem?
    /// The last reported cursor position (1-based), saved with the session.
    private(set) var cursorPosition: (line: UInt32, column: UInt32)?

    /// Whether this editor is currently showing markdown preview instead of Monaco.
    private(set) var isPreviewing: Bool = false
    /// The preview shows beside the editor (and follows edits) rather than
    /// in place of it.
    private(set) var isPreviewBeside = false
    /// Monaco's diff editor is showing the file against its git base.
    private(set) var isDiffView = false
    private(set) var isDiffInline = false
    /// Theme the preview was last rendered with (for live refreshes).
    private var previewTheme: (json: String, bg: String)?
    private var previewRefreshWork: DispatchWorkItem?
    /// The editor's trailing edge: the view's, or the middle when beside.
    private var editorTrailing: NSLayoutConstraint?
    private var editorHalf: NSLayoutConstraint?

    /// Lazily created WKWebView used for markdown preview rendering.
    private var previewWebView: WKWebView?

    /// Navigation delegate for the preview WebView that blocks external URLs
    /// and opens them in the default browser instead.
    private let previewNavigationDelegate = PreviewNavigationDelegate()

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
            let trailing = warmed.trailingAnchor.constraint(equalTo: trailingAnchor)
            editorTrailing = trailing
            NSLayoutConstraint.activate([
                warmed.topAnchor.constraint(equalTo: topAnchor),
                warmed.bottomAnchor.constraint(equalTo: bottomAnchor),
                warmed.leadingAnchor.constraint(equalTo: leadingAnchor),
                trailing,
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

        let wv = EditorWebView(frame: bounds, configuration: config)
        wv.navigationDelegate = self
        wv.translatesAutoresizingMaskIntoConstraints = false
        wv.allowsMagnification = false

        // Make the WebView background transparent so it does not flash white
        // before Monaco renders its own background colour.
        wv.underPageBackgroundColor = .clear

        addSubview(wv)
        let trailing = wv.trailingAnchor.constraint(equalTo: trailingAnchor)
        editorTrailing = trailing
        NSLayoutConstraint.activate([
            wv.topAnchor.constraint(equalTo: topAnchor),
            wv.bottomAnchor.constraint(equalTo: bottomAnchor),
            wv.leadingAnchor.constraint(equalTo: leadingAnchor),
            trailing,
        ])

        self.webView = wv
    }

    // MARK: Loading

    /// Load the editor HTML from the bundled Monaco assets.
    /// This is a no-op if the editor is already ready (e.g. from a pre-warmed WebView).
    func loadEditor() {
        guard !isEditorReady else { return }

        guard let monacoDir = EditorAssets.monacoDirectory else {
            os_log(.error, log: Self.log, "Bundled Monaco assets missing; cannot load editor")
            return
        }
        let editorHTML = monacoDir.appendingPathComponent("editor.html")
        webView?.loadFileURL(editorHTML, allowingReadAccessTo: monacoDir)
    }

    // MARK: WKScriptMessageHandler

    func userContentController(
        _ userContentController: WKUserContentController,
        didReceive message: WKScriptMessage
    ) {
        if message.name == "impulseRun" {
            // A Run button in the markdown preview — only from the preview
            // page Impulse rendered, never from anything a link loaded.
            guard message.frameInfo.isMainFrame,
                previewNavigationDelegate.isPreviewPage(message.frameInfo.request.url),
                let command = message.body as? String, !command.isEmpty
            else { return }
            let directory = filePath.map { ($0 as NSString).deletingLastPathComponent }
            NotificationCenter.default.post(
                name: .impulseRunInTerminal, object: self,
                userInfo: ["command": command, "directory": directory ?? NSHomeDirectory()])
            return
        }
        // Only from the editor page's main frame.
        guard message.name == "impulse", message.frameInfo.isMainFrame,
            EditorAssets.isEditorPage(message.frameInfo.request.url)
        else { return }

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

            if pendingReadOnly {
                sendCommand(.setReadOnly(readOnly: true))
            }

        case let .fileOpened(normalized):
            opensInFlight = max(opensInFlight - 1, 0)
            if opensInFlight == 0 {
                // Monaco's text is the buffer now (line endings and all).
                if let text = normalized ?? openSentText { content = text }
                openSentText = nil
            }
            NotificationCenter.default.post(
                name: .editorFileOpened,
                object: self,
                userInfo: ["filePath": filePath ?? ""]
            )

        case let .contentChanged(newContent, changes, _):
            // Typed into text an open on its way replaces.
            guard opensInFlight == 0 else { return }
            if let newContent {
                content = newContent
            } else if !isResyncingContent {
                applyMonacoContentChanges(changes)
            }
            isModified = true
            if isPreviewing, isPreviewBeside { schedulePreviewRefresh() }
            NotificationCenter.default.post(
                name: .editorContentChanged,
                object: self,
                userInfo: ["filePath": filePath ?? "", "changes": changes]
            )

        case let .cursorMoved(line, column):
            cursorPosition = (line, column)
            cursorDebounceWork?.cancel()
            let work = DispatchWorkItem { [weak self] in
                guard let self else { return }
                NotificationCenter.default.post(
                    name: .editorCursorMoved,
                    object: self,
                    userInfo: ["line": line, "column": column]
                )
            }
            cursorDebounceWork = work
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05, execute: work)

        case let .gitAction(action, line):
            NotificationCenter.default.post(
                name: .editorGitAction, object: self,
                userInfo: ["action": action, "line": Int(line)])

        case let .diffViewChanged(active, inline):
            isDiffView = active
            isDiffInline = inline

        case let .codeActionChosen(token):
            NotificationCenter.default.post(
                name: .editorCodeActionChosen, object: self, userInfo: ["token": token])

        case let .lspRequested(requestId, method, params):
            NotificationCenter.default.post(
                name: .editorLspRequested, object: self,
                userInfo: ["requestId": requestId, "method": method, "params": params])

        case .saveRequested:
            // Route through the main save pipeline so format-on-save, LSP
            // notifications, and other post-save actions run correctly.
            NotificationCenter.default.post(name: .impulseSaveFile, object: self)

        case let .closeRequested(save):
            NotificationCenter.default.post(
                name: .editorCloseRequested, object: self, userInfo: ["save": save])

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
        // Only the editor page (and about:blank). Any other file — a dropped
        // or linked .html from a repository — would run in this web view.
        if EditorAssets.isEditorPage(url) || url.absoluteString == "about:blank" {
            decisionHandler(.allow)
        } else {
            os_log(.info, log: Self.log, "Blocked editor navigation to %{public}@", url.absoluteString)
            decisionHandler(.cancel)
        }
    }

    /// The editor's web content process ended (it crashed, or macOS
    /// reclaimed it). Load the page again and put the buffer back as this
    /// side last knew it, unsaved edits and all; the dirty state lives here
    /// and stays as it was.
    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        guard webView === self.webView else { return }
        isEditorReady = false
        opensInFlight = 0
        openSentText = nil
        isResyncingContent = false
        isDiffView = false
        let now = Date()
        pageTerminations = pageTerminations.filter { now.timeIntervalSince($0) < Self.pageReloadWindow } + [now]
        // It keeps ending (a page that can't hold this file?): stop
        // reloading on its own. The buffer is kept here, and saves still work.
        guard pageTerminations.count <= Self.maxPageReloads else {
            os_log(
                .error, log: Self.log, "Editor web content keeps ending; waiting to reload %{public}@",
                filePath ?? "an untitled editor")
            needsPageReload = true
            showStoppedNotice()
            return
        }
        os_log(
            .error, log: Self.log, "Editor web content ended; reloading %{public}@",
            filePath ?? "an untitled editor")
        reloadPage()
    }

    /// Load the page again and put the buffer back: settings and theme, the
    /// text with its cursor (`.ready` sends them, then read-only mode if it
    /// was on). Commands queued while it was gone are replaced by these.
    private func reloadPage() {
        needsPageReload = false
        stoppedNotice?.removeFromSuperview()
        stoppedNotice = nil
        pendingCommands = []
        if let editorOptions { pendingCommands.append(.updateSettings(options: editorOptions)) }
        if let editorTheme { pendingCommands.append(.setTheme(theme: editorTheme)) }
        pendingCommands.append(.openFile(filePath: filePath ?? "", content: content, language: language))
        if let cursorPosition {
            pendingCommands.append(.goToPosition(line: cursorPosition.line, column: cursorPosition.column))
        }
        loadEditor()
    }

    @objc private func reloadPageClicked(_ sender: Any?) {
        reloadPage()
    }

    /// Covers the dead page: the text is safe, and a button reloads it.
    private func showStoppedNotice() {
        guard stoppedNotice == nil, let webView else { return }
        let label = NSTextField(labelWithString: "This editor stopped working. Your text is kept.")
        let button = NSButton(title: "Reload Editor", target: self, action: #selector(reloadPageClicked(_:)))
        let stack = NSStackView(views: [label, button])
        stack.orientation = .vertical
        stack.spacing = 10
        let notice = NSView()
        notice.wantsLayer = true
        notice.layer?.backgroundColor = editorTheme.map { NSColor(hex: $0.colors.editorBackground) }?.cgColor
            ?? NSColor.textBackgroundColor.cgColor
        if let theme = editorTheme { label.textColor = NSColor(hex: theme.colors.editorForeground) }
        notice.translatesAutoresizingMaskIntoConstraints = false
        stack.translatesAutoresizingMaskIntoConstraints = false
        notice.addSubview(stack)
        // Inside the web view, so it shows and hides with it.
        webView.addSubview(notice)
        NSLayoutConstraint.activate([
            notice.topAnchor.constraint(equalTo: webView.topAnchor),
            notice.bottomAnchor.constraint(equalTo: webView.bottomAnchor),
            notice.leadingAnchor.constraint(equalTo: webView.leadingAnchor),
            notice.trailingAnchor.constraint(equalTo: webView.trailingAnchor),
            stack.centerXAnchor.constraint(equalTo: notice.centerXAnchor),
            stack.centerYAnchor.constraint(equalTo: notice.centerYAnchor),
        ])
        stoppedNotice = notice
    }

    /// Coming back to a tab whose page stopped: try it again.
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil, needsPageReload { reloadPage() }
    }

    private func applyMonacoContentChanges(_ changes: [MonacoContentChange]) {
        let edits = changes.map {
            BufferEdits.Change(offset: Int($0.rangeOffset), length: Int($0.rangeLength), text: $0.text)
        }
        guard !BufferEdits.apply(edits, to: &content) else { return }
        os_log(.error, log: Self.log, "Editor change didn't fit the buffer copy; fetching it whole")
        resyncContent()
    }

    /// Replace the copy with Monaco's whole buffer. Changes made before it
    /// answers are in the answer (and dropped meanwhile); later ones arrive
    /// after it and apply on top.
    private func resyncContent() {
        guard isEditorReady, let webView else { return }
        isResyncingContent = true
        webView.evaluateJavaScript("editor.getValue()") { [weak self] result, error in
            guard let self else { return }
            self.isResyncingContent = false
            if let text = result as? String {
                self.content = text
            } else {
                os_log(
                    .error, log: Self.log, "editor.getValue() failed: %{public}@",
                    error?.localizedDescription ?? "non-string result")
            }
        }
    }

    // MARK: Command Sending

    /// Send a command to the Monaco editor.
    ///
    /// If the editor is not yet ready, the command is queued and will be sent
    /// once the `Ready` event is received.
    /// JSON Schema text for files the app knows the shape of (settings.json).
    static var jsonSchemaProvider: ((String) -> String?)?

    func sendCommand(_ command: EditorCommand) {
        guard isEditorReady else {
            pendingCommands.append(command)
            return
        }
        // Register the schema first so the file validates as it opens.
        if case .openFile(let path, _, _) = command, !path.isEmpty,
           let schema = Self.jsonSchemaProvider?(path) {
            sendCommand(.setJsonSchema(fileMatch: path, schema: schema))
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

        guard let webView else { return }
        var isOpen = false
        if case .openFile(_, let text, _) = command {
            isOpen = true
            opensInFlight += 1
            openSentText = text
        }
        webView.callAsyncJavaScript(
            "window.impulseReceiveCommand(msg);",
            arguments: ["msg": jsonString],
            in: nil,
            in: .page,
            completionHandler: { [weak self] result in
                if case .failure(let error) = result {
                    os_log(.error, log: Self.log, "callAsyncJavaScript failed: %{public}@", error.localizedDescription)
                    // No FileOpened is coming for it.
                    if isOpen, let self { self.opensInFlight = max(self.opensInFlight - 1, 0) }
                }
            }
        )
    }

    // MARK: Public API

    /// Open a file in the editor: `content` as read from disk, and the
    /// file's `stamp` from that same read (nil when it isn't known; then the
    /// disk is checked against `content` once the watcher is up).
    func openFile(
        path: String, content: String, language: String, bom: Bool = false, stamp: TextFile.Stamp? = nil
    ) {
        self.filePath = path
        self.content = content
        self.language = language
        self.lspLanguage = Self.lspLanguageForPath(path, monacoLanguage: language)
        self.isModified = false
        self.hasBOM = bom
        recordDiskState(hash: content.hashValue, stamp: stamp)

        sendCommand(.openFile(filePath: path, content: content, language: language))
        startFileWatching()
        checkDiskSinceRead(path: path, stamp: stamp)
    }

    /// The watcher only sees writes from when it starts. One that landed
    /// between the read and then shows as a stamp that moved: treat it as a
    /// change on disk.
    private func checkDiskSinceRead(path: String, stamp: TextFile.Stamp?) {
        DispatchQueue.global(qos: .utility).async { [weak self] in
            if let stamp, TextFile.stamp(path) == stamp { return }
            DispatchQueue.main.async {
                guard let self, self.filePath == path else { return }
                self.reloadFromDisk(force: false)
            }
        }
    }

    /// Remember what's on disk now (after a load or save): its text's hash
    /// and its stamp when it had that text (nil if unknown).
    private func recordDiskState(hash: Int, stamp: TextFile.Stamp?) {
        diskTextHash = hash
        diskStamp = stamp
        diskGeneration &+= 1
        noticedDiskHash = nil
        largeFileNoticeShown = false
    }

    /// Open a blank untitled editor (no file on disk).
    func openBlank() {
        self.filePath = nil
        self.content = ""
        self.language = "plaintext"
        self.lspLanguage = "plaintext"
        self.isModified = false
        sendCommand(.openFile(filePath: "", content: "", language: "plaintext"))
    }

    /// Returns the LSP language ID for a file path, which may differ from the Monaco language.
    /// For example, `.tsx` files use "typescript" in Monaco but "typescriptreact" for LSP.
    private static func lspLanguageForPath(_ path: String, monacoLanguage: String) -> String {
        // LSP language ids where they differ from Monaco's.
        let ext = (path as NSString).pathExtension.lowercased()
        switch ext {
        case "tsx": return "typescriptreact"
        case "jsx": return "javascriptreact"
        case "vue": return "vue"
        case "svelte": return "svelte"
        case "jsonc": return "jsonc"
        // Highlighted as shell, but bash-language-server can't parse fish.
        case "fish": return "fish"
        default: return monacoLanguage == "shell" ? "shellscript" : monacoLanguage
        }
    }

    /// Fetch the latest content from Monaco and save it, then call
    /// `completion`. Monaco's own buffer is what's written; the copy kept
    /// here is only the fallback when the page can't answer (it's loading,
    /// or its web content process ended).
    func fetchContentAndSave(completion: @escaping (Bool) -> Void) {
        guard let path = filePath else {
            completion(false)
            return
        }

        guard isEditorReady, let webView else {
            // Editor not ready: save what we have, still checking the disk.
            writeCheckingDisk(content, to: path, completion: completion)
            return
        }

        webView.evaluateJavaScript("editor.getValue()") { [weak self] result, error in
            guard let self else { completion(false); return }
            if let latest = result as? String {
                self.content = latest
            } else {
                // Monaco failed to return the buffer — fall back to the
                // content accumulated from ContentChanged events, but make
                // the failure visible in the log so a stale save is traceable.
                os_log(
                    .error, log: Self.log,
                    "editor.getValue() failed for %{public}@ (error: %{public}@); saving last-known content",
                    path, error?.localizedDescription ?? "non-string result")
            }
            let contentToSave = self.content
            self.writeCheckingDisk(contentToSave, to: path, completion: completion)
        }
    }

    /// Write the buffer, first asking (via `resolveSaveConflict`) when the
    /// file changed on disk since it was loaded or last saved.
    private func writeCheckingDisk(_ text: String, to path: String, completion: @escaping (Bool) -> Void) {
        let expectedHash = diskTextHash
        let expectedStamp = diskStamp
        let bom = hasBOM
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            // Changed elsewhere: the stamp moved and the text isn't what we
            // last saw (or what we're about to write).
            var conflict = false
            if let expectedHash, TextFile.stamp(path) != expectedStamp, let disk = TextFile.read(path),
                disk.text.hashValue != expectedHash, disk.text != text
            {
                conflict = true
            }
            let write = {
                DispatchQueue.global(qos: .userInitiated).async { [weak self] in
                    do {
                        try TextFile.write(text, bom: bom, to: path)
                        // The stamp right after our write, not whenever main gets to it.
                        let stamp = TextFile.stamp(path)
                        let hash = text.hashValue
                        DispatchQueue.main.async {
                            guard let self else { return completion(true) }
                            self.recordDiskState(hash: hash, stamp: stamp)
                            self.lastWrittenText = text
                            // Typing that arrived while writing stays unsaved.
                            if self.content == text { self.isModified = false }
                            completion(true)
                        }
                    } catch {
                        os_log(
                            .error, log: Self.log, "Failed to save file %{public}@: %{public}@", path,
                            error.localizedDescription)
                        DispatchQueue.main.async { completion(false) }
                    }
                }
            }
            DispatchQueue.main.async {
                guard conflict, let self, let resolve = self.resolveSaveConflict else { return write() }
                resolve(self) { overwrite in
                    if overwrite { write() } else { completion(false) }
                }
            }
        }
    }

    /// A formatter or command on save is about to rewrite the file just
    /// saved: until `adoptDiskChanges` runs, the watcher leaves the file to
    /// it (a half-written result mustn't replace the buffer).
    func beginDiskRewrite() {
        pendingRewrites += 1
        rewritesMutedUntil = Date().addingTimeInterval(Self.rewriteMuteLimit)
    }

    /// After something rewrote the file just saved (a formatter, a command
    /// on save): show its result, as one undoable edit, when it `succeeded`
    /// and the buffer still holds what was saved. Newer typing is kept
    /// instead (unsaved). A failed run, or one that emptied the file, isn't
    /// shown: the buffer keeps the saved text, marked unsaved, and what the
    /// file holds isn't taken as known (another program may have written it
    /// meanwhile), so the next save asks before replacing it. Ends a
    /// `beginDiskRewrite`.
    func adoptDiskChanges(afterSaving saved: String, succeeded: Bool, completion: (() -> Void)? = nil) {
        guard let path = filePath else {
            pendingRewrites = max(pendingRewrites - 1, 0)
            completion?()
            return
        }
        let buffer = content
        let generation = contentGeneration
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let read = TextFile.readStamped(path)
            let hash = read?.contents.text.hashValue
            let outcome = read.map {
                SaveCommand.rewriteOutcome(saved: saved, disk: $0.contents.text, buffer: buffer, succeeded: succeeded)
            }
            DispatchQueue.main.async {
                defer { completion?() }
                guard let self else { return }
                self.pendingRewrites = max(self.pendingRewrites - 1, 0)
                guard let read, let hash, var outcome else { return }
                // Typing since the copy was taken: decide with what's there now.
                if generation != self.contentGeneration {
                    outcome = SaveCommand.rewriteOutcome(
                        saved: saved, disk: read.contents.text, buffer: self.content, succeeded: succeeded)
                }
                switch outcome {
                case .adopt:
                    self.lastWrittenText = read.contents.text
                    self.replaceContent(with: read.contents, hash: hash, stamp: read.stamp)
                case .record(let dirty):
                    // A later command on save starts from what's on disk now.
                    self.lastWrittenText = read.contents.text
                    self.recordDiskState(hash: hash, stamp: read.stamp)
                    if dirty { self.isModified = true }
                case .keepSavedState:
                    self.isModified = true
                }
            }
        }
    }

    /// Apply a Monaco theme to the editor.
    func applyTheme(_ theme: MonacoThemeDefinition) {
        editorTheme = theme
        sendCommand(.setTheme(theme: theme))
    }

    /// Apply editor settings (font, tab size, etc.).
    func applySettings(_ options: EditorOptions) {
        editorOptions = options
        sendCommand(.updateSettings(options: options))
    }

    /// Resolve a pending definition request. Pass nil uri for "not found".
    func resolveDefinition(requestId: UInt64, uri: String?, line: UInt32?, column: UInt32?) {
        sendCommand(.resolveDefinition(requestId: requestId, uri: uri, line: line, column: column))
    }

    /// Navigate the editor cursor to the given line and column.
    /// Move the cursor to a 1-based line and column and reveal it.
    func goToPosition(line: UInt32, column: UInt32) {
        sendCommand(.goToPosition(line: line, column: column))
    }

    /// Set the editor to read-only or read-write mode.
    func setReadOnly(_ readOnly: Bool) {
        pendingReadOnly = readOnly
        sendCommand(.setReadOnly(readOnly: readOnly))
    }

    /// Apply git diff decorations in the gutter.
    func applyDiffDecorations(_ decorations: [DiffDecoration]) {
        sendCommand(.applyDiffDecorations(decorations: decorations))
    }

    /// Give Monaco the file's git base (index version) and blame so it can
    /// mark changes against the live buffer.
    /// Show (or leave) the diff against the file's git base. The state
    /// follows Monaco's reply, since the view can also be closed from inside.
    func setDiffView(_ enabled: Bool, inline: Bool? = nil) {
        sendCommand(.setDiffView(enabled: enabled, inline: inline ?? isDiffInline))
    }

    func setGitBase(_ base: String?, blame: [EditorBlameLine]) {
        sendCommand(.setGitBase(base: base, blame: blame))
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
    func applyEdits(token: String, edits: [MonacoTextEdit]) {
        sendCommand(.applyEdits(token: token, edits: edits))
    }

    func undoEdits(token: String) {
        sendCommand(.undoEdits(token: token))
    }

    func resolveLspRequest(requestId: UInt64, result: String) {
        sendCommand(.resolveLspRequest(requestId: requestId, result: result))
    }

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
            eventMask: [.write, .rename, .delete],
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
            guard let self else { return }
            if self.isLargeFile { self.noteLargeFileChange() } else { self.reloadFromDisk(force: false) }
        }
        fileWatchDebounce = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1, execute: work)
    }

    /// A large file isn't read again on every write (a growing log would
    /// keep the editor busy reloading): when its stamp moves, a notice
    /// offers to reload it.
    private func noteLargeFileChange() {
        guard let path = filePath else { return }
        startFileWatching()
        guard !largeFileNoticeShown else { return }
        let known = diskStamp
        DispatchQueue.global(qos: .utility).async { [weak self] in
            guard known == nil || TextFile.stamp(path) != known else { return }
            DispatchQueue.main.async {
                guard let self, self.filePath == path, !self.largeFileNoticeShown else { return }
                self.largeFileNoticeShown = true
                NotificationCenter.default.post(name: .editorChangedOnDisk, object: self)
            }
        }
    }

    /// Bring in what's on disk. A clean buffer reloads; a buffer with unsaved
    /// edits (or a large file) is left alone and a "changed on disk" notice
    /// goes up instead (unless `force`, which discards the edits). The file
    /// is read and compared off the main thread.
    func reloadFromDisk(force: Bool) {
        guard let path = filePath else { return }
        // Re-arm the watcher first: after an atomic write (temp → rename)
        // the old descriptor watches a deleted inode, and a write from here
        // on has to be seen.
        startFileWatching()
        // A formatter or command on save is rewriting it: its result is
        // `adoptDiskChanges`'s to take or leave.
        if !force, isRewriting { return }
        let buffer = content
        let generation = contentGeneration
        let diskState = diskGeneration
        reloadSequence &+= 1
        let sequence = reloadSequence
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            // Too large for the editor: not even read.
            if let size = TextFile.stamp(path)?.size, size > TabManager.maxEditorFileSize {
                DispatchQueue.main.async {
                    guard let self, self.filePath == path else { return }
                    self.diskTooLarge(force: force)
                }
                return
            }
            guard let read = TextFile.readStamped(path) else { return }
            let hash = read.contents.text.hashValue
            let matchesBuffer = read.contents.text == buffer
            DispatchQueue.main.async {
                guard let self, self.filePath == path else { return }
                if read.contents.text.utf8.count > TabManager.maxEditorFileSize {
                    return self.diskTooLarge(force: force)
                }
                if !force {
                    // A newer read is on its way, with the later disk.
                    guard sequence == self.reloadSequence else { return }
                    // What's on disk was recorded since this read began (a
                    // save, a formatter's result), so it may be older: read
                    // again, unless it's what was recorded.
                    if diskState != self.diskGeneration {
                        if hash != self.diskTextHash { self.reloadFromDisk(force: false) }
                        return
                    }
                }
                // Typing since the copy was taken: compare with what's there now.
                let matches = generation == self.contentGeneration ? matchesBuffer : read.contents.text == self.content
                self.takeDiskRead(read.contents, hash: hash, stamp: read.stamp, matchesBuffer: matches, force: force)
            }
        }
    }

    private func takeDiskRead(
        _ disk: TextFile.Contents, hash: Int, stamp: TextFile.Stamp?, matchesBuffer: Bool, force: Bool
    ) {
        if matchesBuffer {
            recordDiskState(hash: hash, stamp: stamp)
            if force { isModified = false }
            return
        }
        // It grew large while open: from now on it's treated like a file
        // opened that large.
        let grewLarge = !isLargeFile && disk.text.utf8.count > Self.largeFileSize
        if !force {
            // Unchanged since we last saw it (a touch, or our own save).
            if hash == diskTextHash || isRewriting { return }
            if grewLarge { becomeLargeFile() }
            if isModified || isLargeFile {
                guard noticedDiskHash != hash else { return }
                noticedDiskHash = hash
                largeFileNoticeShown = isLargeFile
                NotificationCenter.default.post(name: .editorChangedOnDisk, object: self)
                return
            }
        }
        replaceContent(with: disk, hash: hash, stamp: stamp)
        if grewLarge { becomeLargeFile() }
    }

    /// Read-only, and changes on disk show a notice instead of reloading.
    /// (A buffer with unsaved edits stays editable, so they can be saved.)
    private func becomeLargeFile() {
        isLargeFile = true
        if !isModified { setReadOnly(true) }
    }

    /// The file on disk is now too large for the editor: the buffer stays
    /// as it is, and a notice says so (once, or whenever asked to reload).
    private func diskTooLarge(force: Bool) {
        if !isLargeFile { becomeLargeFile() }
        guard force || !largeFileNoticeShown else { return }
        largeFileNoticeShown = true
        NotificationCenter.default.post(name: .editorChangedOnDisk, object: self, userInfo: ["tooLarge": true])
    }

    /// Show `disk` as the buffer. Monaco edits the open file in place, as
    /// one step Undo takes back, keeping the cursor and scroll; the window
    /// resyncs the language server when it reports FileOpened.
    private func replaceContent(with disk: TextFile.Contents, hash: Int, stamp: TextFile.Stamp?) {
        guard let path = filePath else { return }
        content = disk.text
        hasBOM = disk.bom
        isModified = false
        recordDiskState(hash: hash, stamp: stamp)
        sendCommand(.openFile(filePath: path, content: disk.text, language: language))
        // Force WebView repaint immediately — WKWebView may defer visual updates
        // when the view isn't first responder (e.g. user is focused elsewhere).
        if let wv = webView { wv.setNeedsDisplay(wv.bounds) }
    }

    // MARK: Cleanup

    // MARK: - Preview (Markdown / SVG)

    /// Check whether a file path is a markdown file.
    static func isMarkdownFile(_ path: String) -> Bool {
        return MarkdownPreview.isMarkdownFile(path)
    }

    /// Check whether a file path is an SVG file.
    static func isSvgFile(_ path: String) -> Bool {
        return SVGPreview.isSVGFile(path)
    }

    /// Check whether a file path is a previewable type (markdown or SVG).
    static func isPreviewableFile(_ path: String) -> Bool {
        return MarkdownPreview.isPreviewableFile(path)
    }

    /// Toggle between Monaco editor and rendered preview (markdown or SVG).
    ///
    /// Returns the new `isPreviewing` state, or `nil` if the file is not previewable.
    /// - Parameters:
    ///   - themeJSON: JSON string with markdown theme colors.
    ///   - bgColor: Background color hex string for SVG preview (avoids re-parsing themeJSON).
    func togglePreview(themeJSON: String, bgColor: String) -> Bool? {
        guard let fp = filePath, EditorTab.isPreviewableFile(fp) else { return nil }

        if isPreviewing {
            // Switch back to editor
            closePreview()
            return false
        }
        return showPreview(beside: false, themeJSON: themeJSON, bgColor: bgColor) ? true : nil
    }

    /// Show or hide the preview beside the editor, re-rendered as you type.
    func togglePreviewBeside(themeJSON: String, bgColor: String) -> Bool? {
        guard let fp = filePath, EditorTab.isPreviewableFile(fp) else { return nil }
        if isPreviewing {
            let wasBeside = isPreviewBeside
            closePreview()
            if wasBeside { return false }
        }
        return showPreview(beside: true, themeJSON: themeJSON, bgColor: bgColor) ? true : nil
    }

    private func closePreview() {
        previewWebView?.isHidden = true
        webView?.isHidden = false
        if isPreviewBeside {
            editorHalf?.isActive = false
            editorTrailing?.isActive = true
        }
        isPreviewing = false
        isPreviewBeside = false
        previewRefreshWork?.cancel()
    }

    private func showPreview(beside: Bool, themeJSON: String, bgColor: String) -> Bool {
        guard let fp = filePath,
            let html = renderPreviewHTML(filePath: fp, themeJSON: themeJSON, bgColor: bgColor)
        else { return false }
        previewTheme = (themeJSON, bgColor)

        // Create or reuse preview WebView. `allowFileAccessFromFileURLs` is
        // intentionally *not* enabled — it would let JS in rendered markdown
        // XHR other files under the granted read-access root. Images loaded
        // via `<img src>` still work without that preference.
        if previewWebView == nil {
            let config = WKWebViewConfiguration()
            config.userContentController.add(WeakScriptMessageHandler(delegate: self), name: "impulseRun")
            let wv = WKWebView(frame: bounds, configuration: config)
            wv.navigationDelegate = previewNavigationDelegate
            wv.translatesAutoresizingMaskIntoConstraints = false
            wv.underPageBackgroundColor = .clear
            addSubview(wv)
            previewLeading = wv.leadingAnchor.constraint(equalTo: leadingAnchor)
            NSLayoutConstraint.activate([
                wv.topAnchor.constraint(equalTo: topAnchor),
                wv.bottomAnchor.constraint(equalTo: bottomAnchor),
                previewLeading!,
                wv.trailingAnchor.constraint(equalTo: trailingAnchor),
            ])
            previewWebView = wv
        }

        loadPreviewHTML(html, filePath: fp)
        previewWebView?.isHidden = false
        if beside, let webView, let preview = previewWebView {
            // Editor on the left half, preview on the right.
            editorTrailing?.isActive = false
            editorHalf?.isActive = false
            editorHalf = webView.trailingAnchor.constraint(equalTo: centerXAnchor)
            editorHalf?.isActive = true
            previewLeading?.isActive = false
            previewLeading = preview.leadingAnchor.constraint(equalTo: centerXAnchor, constant: 1)
            previewLeading?.isActive = true
            webView.isHidden = false
        } else {
            previewLeading?.isActive = false
            previewLeading = previewWebView?.leadingAnchor.constraint(equalTo: leadingAnchor)
            previewLeading?.isActive = true
            webView?.isHidden = true
        }
        isPreviewing = true
        isPreviewBeside = beside
        return true
    }

    private var previewLeading: NSLayoutConstraint?

    /// Re-render a beside preview shortly after typing stops, keeping its
    /// scroll position.
    private func schedulePreviewRefresh() {
        previewRefreshWork?.cancel()
        let work = DispatchWorkItem { [weak self] in
            guard let self, self.isPreviewing, let theme = self.previewTheme, let fp = self.filePath,
                let html = self.renderPreviewHTML(filePath: fp, themeJSON: theme.json, bgColor: theme.bg)
            else { return }
            self.previewWebView?.evaluateJavaScript("window.scrollY") { [weak self] value, _ in
                self?.previewNavigationDelegate.restoreScrollY = value as? Double
                self?.loadPreviewHTML(html, filePath: fp)
            }
        }
        previewRefreshWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3, execute: work)
    }

    /// Re-render the preview with new theme colors (for theme changes).
    func refreshPreview(themeJSON: String, bgColor: String) {
        guard isPreviewing, let fp = filePath else { return }
        previewTheme = (themeJSON, bgColor)
        guard let html = renderPreviewHTML(filePath: fp, themeJSON: themeJSON, bgColor: bgColor) else { return }
        loadPreviewHTML(html, filePath: fp)
    }

    /// Load preview HTML with the source file's parent as the base URL so
    /// relative image links in markdown resolve without navigating WKWebView
    /// to a temp file outside that read context.
    private func loadPreviewHTML(_ html: String, filePath fp: String) {
        let parentDir = URL(fileURLWithPath: (fp as NSString).deletingLastPathComponent, isDirectory: true)
        previewNavigationDelegate.pageURL = parentDir
        previewNavigationDelegate.openFile = { path in
            NotificationCenter.default.post(name: .impulseOpenFile, object: nil, userInfo: ["path": path])
        }
        previewWebView?.loadHTMLString(html, baseURL: parentDir)
    }

    /// Render preview HTML for a file (markdown or SVG). Returns `nil` on failure
    /// or if the source exceeds size limits.
    private func renderPreviewHTML(filePath fp: String, themeJSON: String, bgColor: String) -> String? {
        if EditorTab.isSvgFile(fp) {
            return SVGPreview.render(source: content, bgColor: bgColor)
        }
        // Markdown preview
        let hljs =
            EditorAssets.monacoDirectory?
            .appendingPathComponent("highlight/highlight.min.js")
            .absoluteString ?? ""
        let theme =
            (try? JSONDecoder().decode(MarkdownThemeColors.self, from: Data(themeJSON.utf8)))
            ?? MarkdownThemeColors.fallback
        return MarkdownPreview.render(
            source: content,
            theme: theme,
            highlightJSPath: hljs,
            runButtons: true
        )
    }

    /// Explicitly release resources held by the WebView. Must be called before
    /// the tab is removed from the tab list to ensure the WKWebView and its
    /// associated JavaScript context are torn down promptly.
    func cleanup() {
        stopFileWatching()
        cursorDebounceWork?.cancel()
        cursorDebounceWork = nil
        webView?.configuration.userContentController.removeScriptMessageHandler(forName: "impulse")
        webView?.navigationDelegate = nil
        webView?.stopLoading()
        webView?.removeFromSuperview()
        webView = nil
        previewWebView?.navigationDelegate = nil
        previewWebView?.stopLoading()
        previewWebView?.removeFromSuperview()
        previewWebView = nil
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

// MARK: - Preview Navigation Delegate

/// WKNavigationDelegate for the markdown preview WebView.
/// Allows file:// and about: navigations (needed for the preview itself).
/// External URLs (http/https) are opened in the default browser instead.
private class PreviewNavigationDelegate: NSObject, WKNavigationDelegate {
    /// Scroll back here after the next load (live refresh).
    var restoreScrollY: Double?

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        guard let y = restoreScrollY, y > 0 else { return }
        restoreScrollY = nil
        webView.evaluateJavaScript("window.scrollTo(0, \(y))")
    }

    /// The rendered page's base URL (the file's folder).
    var pageURL: URL?
    /// Opens a linked local file in an editor tab.
    var openFile: ((String) -> Void)?

    /// `url` is the preview page itself (any #anchor aside).
    func isPreviewPage(_ url: URL?) -> Bool {
        guard let url, let pageURL else { return false }
        var components = URLComponents(url: url, resolvingAgainstBaseURL: true)
        components?.fragment = nil
        return components?.url?.standardizedFileURL == pageURL.standardizedFileURL
    }

    /// The preview only ever shows the page Impulse rendered. Links open
    /// elsewhere — web pages in the browser, local files in an editor tab —
    /// so nothing a repository links to can run in this web view (which can
    /// post Run commands to a terminal).
    func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationAction: WKNavigationAction,
        decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
    ) {
        guard let url = navigationAction.request.url else {
            decisionHandler(.cancel)
            return
        }
        // Our own load (loadHTMLString), and jumps to #anchors in it.
        if isPreviewPage(url), navigationAction.navigationType == .other || url.fragment != nil {
            decisionHandler(.allow)
            return
        }
        if navigationAction.navigationType == .linkActivated {
            switch url.scheme?.lowercased() {
            case "http", "https", "mailto":
                NSWorkspace.shared.open(url)
            case "file":
                openFile?(url.path)
            default:
                break
            }
        }
        decisionHandler(.cancel)
    }
}
