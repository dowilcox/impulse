// ---------------------------------------------------------------------------
// macOS Main Window Layout
// ---------------------------------------------------------------------------
//
// The main window uses NSSplitView to divide into three areas:
//
// ┌────────────┬─────────────────────────────┐
// │            │                             │
// │  Sidebar   │       Editor Area           │
// │  (File     │    (WKWebView + Monaco)     │
// │   Tree)    │                             │
// │            │                             │
// │            ├─────────────────────────────┤
// │            │      Terminal Area          │
// │            │   (SwiftTerm or custom PTY) │
// └────────────┴─────────────────────────────┘
//
// Components:
//
// 1. Sidebar (left, resizable):
//    - NSOutlineView with file tree
//    - Uses impulse_core::filesystem for directory listing
//    - Uses impulse_core::git for status indicators
//
// 2. Editor (center, main area):
//    - WKWebView loading impulse_editor::assets::EDITOR_HTML
//    - Communication via WKScriptMessageHandler ("impulse" handler)
//    - Same protocol as Linux: EditorCommand/EditorEvent
//    - Tab bar above editor (NSTabView or custom)
//
// 3. Terminal (bottom, resizable):
//    - Options: SwiftTerm framework, or custom PTY via impulse_core::pty
//    - VTE is Linux-only, so a different terminal emulator is needed
//
// Window chrome:
//    - NSToolbar with tab bar integration
//    - Native title bar (blends with macOS style)
//    - NSSearchField for quick-open (Cmd+P)
//
// Dependencies:
//    objc2-app-kit (NSWindow, NSView, NSSplitView, NSOutlineView, NSToolbar)
//    objc2-web-kit (WKWebView, WKUserContentController, WKScriptMessageHandler)
