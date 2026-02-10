// ---------------------------------------------------------------------------
// macOS Editor Integration (WKWebView + Monaco)
// ---------------------------------------------------------------------------
//
// This module will embed Monaco editor in a WKWebView, using the shared
// HTML/JS from the impulse-editor crate.
//
// Architecture:
//
//   Rust (impulse-macos)           WKWebView (Monaco)
//       |                              |
//       |-- evaluateJavaScript() ----->|  (EditorCommand)
//       |                              |
//       |<-- WKScriptMessageHandler ---|  (EditorEvent)
//       |                              |
//
// Setup:
//
//   1. Create WKWebViewConfiguration
//   2. Get WKUserContentController from config
//   3. Add script message handler for "impulse" name
//   4. Create WKWebView with configuration
//   5. Load HTML: webView.loadHTMLString(EDITOR_HTML, baseURL: URL(string: "file:///"))
//   6. Wait for "Ready" event
//   7. Send OpenFile, SetTheme, UpdateSettings commands
//
// Communication:
//
//   Rust → JS:  webView.evaluateJavaScript("impulseReceiveCommand('...')")
//   JS → Rust:  window.webkit.messageHandlers.impulse.postMessage(json)
//               (WKWebView uses the same webkit.messageHandlers API as WebKitGTK)
//
// The shared protocol types from impulse_editor::protocol are used for
// serialization/deserialization on both platforms.
//
// LSP integration follows the same pattern as Linux:
//   - CompletionRequested events → impulse_core::lsp → ResolveCompletions command
//   - HoverRequested events → impulse_core::lsp → ResolveHover command
//   - DefinitionRequested events → impulse_core::lsp → open file + GoToPosition
//   - Diagnostics from LSP → ApplyDiagnostics command
