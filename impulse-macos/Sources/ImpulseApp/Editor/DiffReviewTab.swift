import AppKit
import Observation
import SwiftUI
import WebKit
import os.log

// MARK: - WeakReviewScriptMessageHandler

/// Proxy that prevents WKUserContentController from retaining its message
/// handler strongly (mirrors EditorTab's WeakScriptMessageHandler). The real
/// handler (DiffReviewTab) is held only weakly so it can deallocate normally.
private final class WeakReviewScriptMessageHandler: NSObject, WKScriptMessageHandler {
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

// MARK: - Review Chrome Model

/// Observable state for the native SwiftUI header + commit bar around the
/// review WebView. DiffReviewTab mutates it; the SwiftUI views observe it.
@Observable
final class ReviewChromeModel {
    var theme: Theme
    var branch: String?
    var fileCount: Int = 0
    var totalAdded: UInt32 = 0
    var totalRemoved: UInt32 = 0
    var repoName: String = ""
    var commitMessage: String = ""
    /// Brief inline confirmation shown after a successful commit.
    var commitConfirmation: String?
    /// Bumped to focus the commit message field.
    var commitFocusToken: Int = 0

    /// Callbacks into the AppKit DiffReviewTab.
    var onRefresh: (() -> Void)?
    var onCommit: (() -> Void)?

    init(theme: Theme) {
        self.theme = theme
    }
}

// MARK: - DiffReviewTab

/// A tab that reviews uncommitted git changes. Layout (top to bottom):
///   - Native SwiftUI header: repo + branch + file count + aggregate +/- and a
///     Refresh button.
///   - A WKWebView hosting review.html (Monaco diff editors per file).
///   - Native SwiftUI commit bar: a message field + Commit button (Cmd+Return).
///
/// Communication with review.js uses the JSON protocol in `ReviewProtocol.swift`
/// over the "impulseReview" message channel, matching the Rust impulse-editor
/// `ReviewCommand` / `ReviewEvent` types.
final class DiffReviewTab: NSView, WKScriptMessageHandler, WKNavigationDelegate {

    // MARK: Properties

    /// The git repository root being reviewed.
    let repoRoot: String

    private(set) var webView: WKWebView?

    /// Inline error view shown when the review editor can't be loaded (e.g.
    /// Monaco asset extraction failed). Replaces the WebView's "Loading…"
    /// dead-end with an actionable message.
    private var errorView: NSView?

    /// Whether review.js has fired its `Ready` event.
    private var isReady = false

    /// Theme used for native chrome + Monaco diff editors.
    private var theme: Theme

    /// Observable chrome model shared with the SwiftUI header + commit bar.
    private let chrome: ReviewChromeModel

    /// Generation counter to drop stale async results (mirrors SearchPanelView).
    private var loadGeneration = 0

    /// Cached set of repo-relative paths from the latest render, used to
    /// validate discard targets.
    private var knownPaths: Set<String> = []

    private let jsonEncoder: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.outputFormatting = []
        return encoder
    }()
    private let jsonDecoder = JSONDecoder()

    private static let log = OSLog(subsystem: "dev.impulse.Impulse", category: "DiffReviewTab")

    private static let messageHandlerName = "impulseReview"

    // MARK: Init

    init(repoRoot: String, theme: Theme) {
        self.repoRoot = repoRoot
        self.theme = theme
        self.chrome = ReviewChromeModel(theme: theme)
        super.init(frame: NSRect(x: 0, y: 0, width: 800, height: 600))
        self.chrome.repoName = (repoRoot as NSString).lastPathComponent
        wantsLayer = true
        layer?.backgroundColor = theme.bgColor.cgColor
        setupViews()
        loadReview()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    // MARK: View Setup

    private func setupViews() {
        chrome.onRefresh = { [weak self] in self?.reloadAndRender() }
        chrome.onCommit = { [weak self] in self?.performCommit() }

        // --- Header (SwiftUI) ---
        let headerView = NSHostingView(rootView: ReviewHeaderView(model: chrome))
        headerView.translatesAutoresizingMaskIntoConstraints = false

        // --- WebView ---
        let config = WKWebViewConfiguration()
        config.userContentController.add(
            WeakReviewScriptMessageHandler(delegate: self),
            name: Self.messageHandlerName
        )
        let pagePrefs = WKWebpagePreferences()
        pagePrefs.allowsContentJavaScript = true
        config.defaultWebpagePreferences = pagePrefs

        let wv = WKWebView(frame: bounds, configuration: config)
        wv.navigationDelegate = self
        wv.translatesAutoresizingMaskIntoConstraints = false
        wv.allowsMagnification = false
        wv.underPageBackgroundColor = .clear
        self.webView = wv

        // --- Commit bar (SwiftUI) ---
        let commitView = NSHostingView(rootView: ReviewCommitBarView(model: chrome))
        commitView.translatesAutoresizingMaskIntoConstraints = false

        addSubview(headerView)
        addSubview(wv)
        addSubview(commitView)

        NSLayoutConstraint.activate([
            headerView.topAnchor.constraint(equalTo: topAnchor),
            headerView.leadingAnchor.constraint(equalTo: leadingAnchor),
            headerView.trailingAnchor.constraint(equalTo: trailingAnchor),

            wv.topAnchor.constraint(equalTo: headerView.bottomAnchor),
            wv.leadingAnchor.constraint(equalTo: leadingAnchor),
            wv.trailingAnchor.constraint(equalTo: trailingAnchor),

            commitView.topAnchor.constraint(equalTo: wv.bottomAnchor),
            commitView.leadingAnchor.constraint(equalTo: leadingAnchor),
            commitView.trailingAnchor.constraint(equalTo: trailingAnchor),
            commitView.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    // MARK: Loading

    private func loadReview() {
        switch ImpulseCore.ensureMonacoExtracted() {
        case .failure(let error):
            os_log(.error, log: Self.log, "Failed to extract Monaco: %{public}@", error.message)
            // Surface the failure inline instead of leaving a perpetual
            // "Loading changes…" dead-end (the WebView never gets a URL, so
            // review.js never becomes ready).
            showLoadError(message: error.message)
        case .success(let pathString):
            let monacoDir = URL(fileURLWithPath: pathString, isDirectory: true)
            let reviewHTML = monacoDir.appendingPathComponent("review.html")
            webView?.loadFileURL(reviewHTML, allowingReadAccessTo: monacoDir)
        }
    }

    /// Replace the WebView with a visible, themed error message describing why
    /// the review editor could not be loaded. Self.window may be nil during
    /// init, so this uses an inline view rather than a sheet/alert.
    private func showLoadError(message: String) {
        // Hide the (never-loaded) WebView so it doesn't sit blank behind the error.
        webView?.isHidden = true

        let container = NSView()
        container.translatesAutoresizingMaskIntoConstraints = false
        container.wantsLayer = true
        container.layer?.backgroundColor = theme.bgColor.cgColor

        let icon = NSImageView()
        icon.translatesAutoresizingMaskIntoConstraints = false
        icon.image = NSImage(
            systemSymbolName: "exclamationmark.triangle", accessibilityDescription: nil)
        icon.contentTintColor = theme.redColor
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 28, weight: .regular)

        let titleLabel = NSTextField(labelWithString: "Could not load the review editor")
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        titleLabel.font = .systemFont(ofSize: 14, weight: .semibold)
        titleLabel.textColor = theme.fgColor
        titleLabel.alignment = .center

        let detailLabel = NSTextField(wrappingLabelWithString: message)
        detailLabel.translatesAutoresizingMaskIntoConstraints = false
        detailLabel.font = .systemFont(ofSize: 12)
        detailLabel.textColor = theme.fgMutedColor
        detailLabel.alignment = .center
        detailLabel.isSelectable = true

        let stack = NSStackView(views: [icon, titleLabel, detailLabel])
        stack.translatesAutoresizingMaskIntoConstraints = false
        stack.orientation = .vertical
        stack.alignment = .centerX
        stack.spacing = 10

        container.addSubview(stack)
        addSubview(container)
        errorView = container

        guard let webView else { return }
        NSLayoutConstraint.activate([
            // Occupy the same region the WebView would have filled.
            container.topAnchor.constraint(equalTo: webView.topAnchor),
            container.leadingAnchor.constraint(equalTo: webView.leadingAnchor),
            container.trailingAnchor.constraint(equalTo: webView.trailingAnchor),
            container.bottomAnchor.constraint(equalTo: webView.bottomAnchor),

            stack.centerXAnchor.constraint(equalTo: container.centerXAnchor),
            stack.centerYAnchor.constraint(equalTo: container.centerYAnchor),
            stack.leadingAnchor.constraint(
                greaterThanOrEqualTo: container.leadingAnchor, constant: 24),
            stack.trailingAnchor.constraint(
                lessThanOrEqualTo: container.trailingAnchor, constant: -24),
        ])
    }

    // MARK: WKScriptMessageHandler

    func userContentController(
        _ userContentController: WKUserContentController,
        didReceive message: WKScriptMessage
    ) {
        guard message.name == Self.messageHandlerName else { return }
        guard let body = message.body as? String,
              let data = body.data(using: .utf8) else {
            os_log(.error, log: Self.log, "Received non-string review message")
            return
        }
        let event: ReviewEvent
        do {
            event = try jsonDecoder.decode(ReviewEvent.self, from: data)
        } catch {
            os_log(
                .error, log: Self.log, "Failed to decode ReviewEvent: %{public}@",
                error.localizedDescription)
            return
        }
        handleEvent(event)
    }

    private func handleEvent(_ event: ReviewEvent) {
        switch event {
        case .ready:
            isReady = true
            applyTheme(theme)
            reloadAndRender()

        case let .requestDiff(path):
            loadDiff(forPath: path)

        case let .discard(path):
            confirmAndDiscard(path: path)

        case .toggleFile:
            // Expansion is tracked client-side; nothing to do natively.
            break

        case .refresh:
            reloadAndRender()
        }
    }

    // MARK: WKNavigationDelegate

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        os_log(.info, log: Self.log, "Review WebView finished loading")
    }

    func webView(
        _ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error
    ) {
        os_log(
            .error, log: Self.log, "Review WebView navigation failed: %{public}@",
            error.localizedDescription)
    }

    func webView(
        _ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,
        decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
    ) {
        guard let url = navigationAction.request.url else {
            decisionHandler(.cancel)
            return
        }
        if url.scheme == "file" || url.scheme == "about" {
            decisionHandler(.allow)
        } else {
            decisionHandler(.cancel)
        }
    }

    // MARK: Command Sending

    /// Sends a ReviewCommand to review.js via `window.__applyReviewCommand`.
    /// Passing the encoded JSON object literal works because review.js accepts
    /// either a parsed object or a JSON string.
    private func sendCommand(_ command: ReviewCommand) {
        guard isReady, let webView else { return }
        let jsonData: Data
        do {
            jsonData = try jsonEncoder.encode(command)
        } catch {
            os_log(
                .error, log: Self.log, "Failed to encode ReviewCommand: %{public}@",
                error.localizedDescription)
            return
        }
        guard let jsonString = String(data: jsonData, encoding: .utf8) else { return }
        let script = "window.__applyReviewCommand(\(jsonString));"
        webView.evaluateJavaScript(script) { _, error in
            if let error {
                os_log(
                    .error, log: Self.log, "__applyReviewCommand failed: %{public}@",
                    error.localizedDescription)
            }
        }
    }

    // MARK: Data Flow

    /// Reload the changed-file list off the main thread and push a Render.
    /// Uses a generation counter so a stale async result is dropped.
    func reloadAndRender() {
        loadGeneration += 1
        let generation = loadGeneration
        let repo = repoRoot
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let changeSet = ImpulseCore.listChangedFiles(repoPath: repo)
            DispatchQueue.main.async { [weak self] in
                guard let self, generation == self.loadGeneration else { return }
                guard let changeSet else {
                    // Repo disappeared / error — render an empty set.
                    self.knownPaths = []
                    self.chrome.fileCount = 0
                    self.chrome.totalAdded = 0
                    self.chrome.totalRemoved = 0
                    self.sendCommand(.render(files: []))
                    return
                }
                self.applyChangeSet(changeSet)
            }
        }
    }

    private func applyChangeSet(_ changeSet: ImpulseCore.ChangeSet) {
        let entries = changeSet.files.map { f in
            ReviewFileEntry(
                path: f.path, status: f.status, oldPath: f.oldPath,
                added: f.added, removed: f.removed, isBinary: f.isBinary)
        }
        knownPaths = Set(changeSet.files.map { $0.path })
        chrome.branch = changeSet.branch
        chrome.fileCount = changeSet.files.count
        chrome.totalAdded = changeSet.totalAdded
        chrome.totalRemoved = changeSet.totalRemoved
        sendCommand(.render(files: entries))
    }

    /// Fetch diff contents for a repo-relative path off the main thread and
    /// push a SetDiff command.
    private func loadDiff(forPath path: String) {
        let repo = repoRoot
        let generation = loadGeneration
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let diff = ImpulseCore.fileDiffContents(repoPath: repo, filePath: path)
            DispatchQueue.main.async { [weak self] in
                guard let self, generation == self.loadGeneration else { return }
                guard let diff else {
                    // Send an empty diff so review.js stops showing the spinner.
                    self.sendCommand(
                        .setDiff(
                            path: path, original: "", modified: "",
                            language: "plaintext", isBinary: false, tooLarge: false))
                    return
                }
                self.sendCommand(
                    .setDiff(
                        path: path,
                        original: diff.original,
                        modified: diff.modified,
                        language: diff.language,
                        isBinary: diff.isBinary,
                        tooLarge: diff.tooLarge))
            }
        }
    }

    // MARK: Discard

    private func confirmAndDiscard(path: String) {
        guard knownPaths.contains(path) else { return }
        let alert = NSAlert()
        alert.messageText = "Discard changes to \(path)?"
        alert.informativeText =
            "This reverts the file to its last committed state. This cannot be undone."
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Discard")
        alert.addButton(withTitle: "Cancel")
        // Mark the destructive button.
        if let discardButton = alert.buttons.first {
            discardButton.hasDestructiveAction = true
        }

        let runAndApply: (NSApplication.ModalResponse) -> Void = { [weak self] response in
            guard let self, response == .alertFirstButtonReturn else { return }
            let repo = self.repoRoot
            DispatchQueue.global(qos: .userInitiated).async { [weak self] in
                let ok = ImpulseCore.discardPath(repoPath: repo, filePath: path)
                DispatchQueue.main.async { [weak self] in
                    guard let self else { return }
                    if !ok {
                        let failAlert = NSAlert()
                        failAlert.messageText = "Discard Failed"
                        failAlert.informativeText = "Could not discard changes to \(path)."
                        failAlert.alertStyle = .warning
                        failAlert.addButton(withTitle: "OK")
                        if let window = self.window {
                            failAlert.beginSheetModal(for: window, completionHandler: nil)
                        } else {
                            failAlert.runModal()
                        }
                    }
                    // Reload + re-render regardless: even on partial failure the
                    // file list should reflect the current state.
                    self.reloadAndRender()
                }
            }
        }

        if let window = self.window {
            alert.beginSheetModal(for: window, completionHandler: runAndApply)
        } else {
            runAndApply(alert.runModal())
        }
    }

    // MARK: Commit

    private func performCommit() {
        let message = chrome.commitMessage.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !message.isEmpty else {
            NSSound.beep()
            chrome.commitFocusToken += 1
            return
        }
        let repo = repoRoot
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let result = ImpulseCore.commitAll(repoPath: repo, message: message)
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                if result.ok {
                    self.chrome.commitMessage = ""
                    let shortOid = result.oid.map { String($0.prefix(7)) } ?? ""
                    self.chrome.commitConfirmation =
                        shortOid.isEmpty ? "Committed" : "Committed \(shortOid)"
                    // Clear the confirmation after a moment.
                    DispatchQueue.main.asyncAfter(deadline: .now() + 3) { [weak self] in
                        self?.chrome.commitConfirmation = nil
                    }
                    self.reloadAndRender()
                } else {
                    let alert = NSAlert()
                    alert.messageText = "Commit Failed"
                    alert.informativeText = result.error ?? "Unknown error."
                    alert.alertStyle = .warning
                    alert.addButton(withTitle: "OK")
                    if let window = self.window {
                        alert.beginSheetModal(for: window, completionHandler: nil)
                    } else {
                        alert.runModal()
                    }
                }
            }
        }
    }

    // MARK: Theming

    /// Apply a new theme to both the Monaco diff editors and the native chrome.
    func applyTheme(_ theme: Theme) {
        self.theme = theme
        chrome.theme = theme
        layer?.backgroundColor = theme.bgColor.cgColor
        // Keep the inline load-error view (if shown) on theme.
        errorView?.layer?.backgroundColor = theme.bgColor.cgColor
        sendCommand(.setTheme(theme: ThemeManager.monacoTheme(forName: theme.id)))
    }

    // MARK: Focus

    func focus() {
        guard let webView else { return }
        window?.makeFirstResponder(webView)
    }

    // MARK: Cleanup

    /// Tear down the WebView and its message handler. Must be called before the
    /// tab is removed so the JS context is released promptly.
    func cleanup() {
        webView?.configuration.userContentController.removeScriptMessageHandler(
            forName: Self.messageHandlerName)
        webView?.navigationDelegate = nil
        webView?.stopLoading()
        webView?.removeFromSuperview()
        webView = nil
    }

    deinit {
        if let wv = webView {
            wv.configuration.userContentController.removeScriptMessageHandler(
                forName: Self.messageHandlerName)
            wv.navigationDelegate = nil
        }
    }
}

// MARK: - SwiftUI Header

private struct ReviewHeaderView: View {
    @Bindable var model: ReviewChromeModel

    var body: some View {
        let theme = model.theme
        HStack(spacing: 10) {
            Image(systemName: "arrow.triangle.branch")
                .foregroundStyle(theme.colorFgMuted)
            Text(model.repoName.isEmpty ? "Review Changes" : model.repoName)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(theme.colorFg)
            if let branch = model.branch, !branch.isEmpty {
                Text(branch)
                    .font(.system(size: 12))
                    .foregroundStyle(theme.colorAccent)
            }
            Text(fileCountText)
                .font(.system(size: 12))
                .foregroundStyle(theme.colorFgMuted)

            Spacer()

            HStack(spacing: 6) {
                Text("+\(model.totalAdded)")
                    .foregroundStyle(theme.colorGitAdded)
                Text("-\(model.totalRemoved)")
                    .foregroundStyle(theme.colorGitDeleted)
            }
            .font(.system(size: 12, design: .monospaced))

            Button {
                model.onRefresh?()
            } label: {
                Image(systemName: "arrow.clockwise")
            }
            .buttonStyle(.borderless)
            .foregroundStyle(theme.colorFgMuted)
            .help("Refresh")
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .frame(maxWidth: .infinity)
        .background(theme.colorBgSurface)
        .overlay(alignment: .bottom) {
            Rectangle().fill(theme.colorBorder).frame(height: 1)
        }
    }

    private var fileCountText: String {
        model.fileCount == 1 ? "1 file" : "\(model.fileCount) files"
    }
}

// MARK: - SwiftUI Commit Bar

private struct ReviewCommitBarView: View {
    @Bindable var model: ReviewChromeModel
    @FocusState private var messageFocused: Bool

    var body: some View {
        let theme = model.theme
        VStack(spacing: 0) {
            Rectangle().fill(theme.colorBorder).frame(height: 1)
            HStack(spacing: 10) {
                TextField("Commit message", text: $model.commitMessage)
                    .textFieldStyle(.roundedBorder)
                    .font(.system(size: 13))
                    .focused($messageFocused)
                    .onSubmit { model.onCommit?() }

                if let confirmation = model.commitConfirmation {
                    Text(confirmation)
                        .font(.system(size: 12))
                        .foregroundStyle(theme.colorGitAdded)
                        .transition(.opacity)
                }

                Button {
                    model.onCommit?()
                } label: {
                    Text("Commit")
                }
                .keyboardShortcut(.return, modifiers: .command)
                .disabled(model.commitMessage.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .frame(maxWidth: .infinity)
            .background(theme.colorBgSurface)
        }
        .onChange(of: model.commitFocusToken) { _, _ in
            messageFocused = true
        }
    }
}
