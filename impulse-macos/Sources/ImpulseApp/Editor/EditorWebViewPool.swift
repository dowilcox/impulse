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
final class EditorWebViewPool: NSObject, WKScriptMessageHandler, WKNavigationDelegate {

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

    // MARK: - WKNavigationDelegate (warm-up)

    /// While the WebView is warming up (before `claim(...)`), it may only
    /// show the editor page (or about:blank), the same rule as `EditorTab`.
    func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationAction: WKNavigationAction,
        decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
    ) {
        guard let url = navigationAction.request.url else {
            decisionHandler(.cancel)
            return
        }
        if EditorAssets.isEditorPage(url) || url.absoluteString == "about:blank" {
            decisionHandler(.allow)
        } else {
            os_log(.info, log: Self.log, "Pool WebView blocked navigation to: %{public}@", url.absoluteString)
            decisionHandler(.cancel)
        }
    }

    /// When warming WebViews' content processes ended lately.
    private var terminations: [Date] = []

    /// The warming WebView's content process ended: drop it and warm another
    /// (a claimed one would open as a blank page). When it keeps happening,
    /// wait longer each time (30 s, 1 min, … up to 5 min) instead of
    /// spinning up page after page.
    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        guard webView === warmWebView else { return }
        warmWebView = nil
        isReady = false
        let now = Date()
        terminations = terminations.filter { now.timeIntervalSince($0) < 600 } + [now]
        let delay = terminations.count < 2 ? 0 : min(30 * pow(2, Double(terminations.count - 2)), 300)
        os_log(
            .error, log: Self.log, "Pre-warmed WebView's content process ended; warming another in %.0f s", delay)
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
            self?.warmUp()
        }
    }

    // MARK: - Pre-warming

    /// Extract Monaco assets (if needed) and start loading a WebView in the
    /// background. This is idempotent — calling it while a warm-up is already
    /// in progress is a no-op.
    func warmUp() {
        guard warmWebView == nil else { return }

        // Monaco ships in the app bundle.
        if monacoDir == nil {
            guard let bundled = EditorAssets.monacoDirectory else {
                os_log(.error, log: Self.log, "Bundled Monaco assets missing; cannot pre-warm")
                return
            }
            monacoDir = bundled
        }

        guard let monacoDir = monacoDir else { return }

        let config = WKWebViewConfiguration()
        config.userContentController.add(self, name: "impulse")

        let pagePrefs = WKWebpagePreferences()
        pagePrefs.allowsContentJavaScript = true
        config.defaultWebpagePreferences = pagePrefs

        let wv = EditorWebView(frame: NSRect(x: 0, y: 0, width: 800, height: 600), configuration: config)
        wv.allowsMagnification = false
        wv.underPageBackgroundColor = .clear
        wv.navigationDelegate = self

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
        guard message.name == "impulse", message.frameInfo.isMainFrame,
            EditorAssets.isEditorPage(message.frameInfo.request.url)
        else { return }
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
