import SwiftUI
import AppKit

/// Container that syncs its child view's frame on every layout pass,
/// ensuring SwiftTerm picks up the correct size immediately.
private class ContentContainer: NSView {
    override func layout() {
        super.layout()
        // After SwiftUI sets our frame, push it to the child.
        if let child = subviews.first, child.frame != bounds {
            child.frame = bounds
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
