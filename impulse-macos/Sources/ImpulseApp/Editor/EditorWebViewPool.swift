import AppKit
import WebKit
import os.log

/// Maintains a single pre-warmed WKWebView with Monaco already loaded so that
/// editor tabs open instantly instead of waiting for Monaco to initialize.
///
/// Usage:
///   1. Call `warmUp()` at app launch.
///   2. When creating an `EditorTab`, call `claim(newHandler:)` to get a ready
///      WebView. If one is available, the tab can skip `loadEditor()` entirely.
///   3. After a WebView is claimed, the pool automatically starts warming the
///      next one.
final class EditorWebViewPool: NSObject, WKScriptMessageHandler {

    static let shared = EditorWebViewPool()

    private static let log = OSLog(subsystem: "dev.impulse.Impulse", category: "EditorWebViewPool")

    /// The pre-warmed WebView, ready to be claimed.
    private var warmWebView: WKWebView?

    /// Whether the pre-warmed WebView has received the Monaco "Ready" event.
    private var isReady = false

    /// Cached Monaco directory URL (persists after first extraction).
    private(set) var monacoDir: URL?

    /// JSON decoder for checking the Ready event.
    private let jsonDecoder = JSONDecoder()

    // MARK: - Pre-warming

    /// Extract Monaco assets (if needed) and start loading a WebView in the
    /// background. This is idempotent — calling it while a warm-up is already
    /// in progress is a no-op.
    func warmUp() {
        guard warmWebView == nil else { return }

        // Ensure Monaco assets are extracted (cached after first run).
        if monacoDir == nil {
            switch ImpulseCore.ensureMonacoExtracted() {
            case .failure(let error):
                os_log(.error, log: Self.log, "Failed to extract Monaco for pre-warm: %{public}@", error.message)
                return
            case .success(let pathString):
                monacoDir = URL(fileURLWithPath: pathString, isDirectory: true)
            }
        }

        guard let monacoDir = monacoDir else { return }

        let config = WKWebViewConfiguration()
        config.userContentController.add(self, name: "impulse")

        let preferences = WKPreferences()
        preferences.setValue(true, forKey: "javaScriptEnabled")
        config.preferences = preferences

        let wv = WKWebView(frame: NSRect(x: 0, y: 0, width: 800, height: 600), configuration: config)
        wv.setValue(false, forKey: "drawsBackground")

        warmWebView = wv
        isReady = false

        let editorHTML = monacoDir.appendingPathComponent("editor.html")
        wv.loadFileURL(editorHTML, allowingReadAccessTo: monacoDir)
        os_log(.info, log: Self.log, "Started pre-warming a WebView")
    }

    // MARK: - Claiming

    /// Claim the pre-warmed WebView, transferring message handler ownership.
    ///
    /// - Parameter newHandler: The `EditorTab` that will own the WebView.
    /// - Parameter weakProxy: A `WeakScriptMessageHandler` wrapping `newHandler`
    ///   to avoid the WKUserContentController strong-retain cycle.
    /// - Returns: A ready-to-use `WKWebView` with Monaco loaded, or `nil` if
    ///   none is available yet.
    func claim(newHandler: WKScriptMessageHandler & WKNavigationDelegate,
               weakProxy: WKScriptMessageHandler) -> WKWebView? {
        guard isReady, let wv = warmWebView else { return nil }

        warmWebView = nil
        isReady = false

        // Swap the message handler from the pool proxy to the EditorTab's
        // weak proxy so WKUserContentController does not strongly retain
        // the EditorTab.
        wv.configuration.userContentController.removeScriptMessageHandler(forName: "impulse")
        wv.configuration.userContentController.add(weakProxy, name: "impulse")
        wv.navigationDelegate = newHandler

        os_log(.info, log: Self.log, "Claimed pre-warmed WebView")

        // Start warming the next one.
        DispatchQueue.main.async { [weak self] in
            self?.warmUp()
        }

        return wv
    }

    // MARK: - WKScriptMessageHandler

    func userContentController(
        _ userContentController: WKUserContentController,
        didReceive message: WKScriptMessage
    ) {
        guard message.name == "impulse" else { return }
        guard let body = message.body as? String,
              let data = body.data(using: .utf8) else { return }

        // We only care about the Ready event during warm-up.
        struct ReadyCheck: Decodable { let type: String }
        if let check = try? jsonDecoder.decode(ReadyCheck.self, from: data),
           check.type == "Ready" {
            isReady = true
            os_log(.info, log: Self.log, "Pre-warmed WebView is ready")
        }
    }
}
