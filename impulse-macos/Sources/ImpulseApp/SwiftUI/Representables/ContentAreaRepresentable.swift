import SwiftUI
import AppKit

/// Container that forces child views to resize when SwiftUI updates the frame.
/// Sends a window resize notification after the first non-zero layout so
/// SwiftTerm recalculates its row/column count.
private class ContentContainer: NSView {
    private var hasNotifiedInitialSize = false

    override func layout() {
        super.layout()
        if let child = subviews.first, child.frame != bounds {
            child.frame = bounds
        }
        // After the first real layout (non-zero), trigger a window resize
        // notification so SwiftTerm picks up the correct terminal dimensions.
        if !hasNotifiedInitialSize && bounds.width > 0 && bounds.height > 0 {
            hasNotifiedInitialSize = true
            DispatchQueue.main.async { [weak self] in
                guard let self, let window = self.window else { return }
                // Force all terminal views to recalculate their size by
                // posting windowDidResize. SwiftTerm listens for frame
                // changes via Auto Layout, but needs a nudge after the
                // initial embedding into the SwiftUI view hierarchy.
                NotificationCenter.default.post(
                    name: NSWindow.didResizeNotification,
                    object: window
                )
            }
        }
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        // Reset so we re-notify if moved to a new window.
        if window == nil {
            hasNotifiedInitialSize = false
        }
    }
}

/// Wraps TabManager's contentView in an NSViewRepresentable for SwiftUI.
struct ContentAreaRepresentable: NSViewRepresentable {
    let contentView: NSView

    func makeNSView(context: Context) -> NSView {
        let container = ContentContainer()
        contentView.translatesAutoresizingMaskIntoConstraints = true
        contentView.autoresizingMask = [.width, .height]
        contentView.frame = container.bounds
        container.addSubview(contentView)
        return container
    }

    func updateNSView(_ nsView: NSView, context: Context) {
    }
}
