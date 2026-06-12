import SwiftUI
import AppKit

/// Container that forces child views to resize when SwiftUI updates the frame.
/// Sends a window resize notification after the first non-zero layout so
/// the terminal renderer recalculates its row/column count.
private class ContentContainer: NSView {
    private var hasNotifiedInitialSize = false

    override func layout() {
        super.layout()
        if let child = subviews.first, child.frame != bounds {
            child.frame = bounds
        }
        // After the first real layout (non-zero), trigger a window resize
        // notification so the terminal picks up the correct dimensions.
        if !hasNotifiedInitialSize && bounds.width > 0 && bounds.height > 0 {
            hasNotifiedInitialSize = true
            DispatchQueue.main.async { [weak self] in
                guard let self, let window = self.window else { return }
                // Force all terminal views to recalculate their size by
                // posting windowDidResize. The terminal renderer listens
                // for frame changes via Auto Layout, but needs a nudge
                // after the initial embedding into the SwiftUI view hierarchy.
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
    /// Corner radius for "card" surface themes (e.g. Harbor). Clipping must
    /// happen at the AppKit layer — SwiftUI's clipShape doesn't clip the
    /// drawing of embedded NSViews.
    var cornerRadius: CGFloat = 0

    func makeNSView(context: Context) -> NSView {
        let container = ContentContainer()
        container.wantsLayer = true
        applyCornerRadius(to: container)
        contentView.translatesAutoresizingMaskIntoConstraints = true
        contentView.autoresizingMask = [.width, .height]
        contentView.frame = container.bounds
        container.addSubview(contentView)
        return container
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        applyCornerRadius(to: nsView)
    }

    private func applyCornerRadius(to view: NSView) {
        view.layer?.cornerRadius = cornerRadius
        view.layer?.cornerCurve = .continuous
        view.layer?.masksToBounds = cornerRadius > 0
    }
}
