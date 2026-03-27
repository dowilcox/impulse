import SwiftUI
import AppKit

/// Wraps TabManager's contentView (the NSView that shows/hides tab content)
/// in an NSViewRepresentable for embedding in SwiftUI.
///
/// TabManager continues to manage the terminal/editor NSView lifecycle
/// internally (add/remove from superview, show/hide). This wrapper simply
/// provides the container view to SwiftUI's layout system.
struct ContentAreaRepresentable: NSViewRepresentable {
    let contentView: NSView

    func makeNSView(context: Context) -> NSView {
        contentView
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        // TabManager manages content internally; nothing to update here.
    }
}
