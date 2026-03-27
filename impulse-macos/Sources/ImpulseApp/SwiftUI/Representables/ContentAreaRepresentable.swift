import SwiftUI
import AppKit

/// Wraps TabManager's contentView (the NSView that shows/hides tab content)
/// in an NSViewRepresentable for embedding in SwiftUI.
///
/// Uses a stable container NSView so that SwiftUI view recreation doesn't
/// disrupt the AppKit view hierarchy. TabManager continues to manage the
/// terminal/editor NSView lifecycle internally.
struct ContentAreaRepresentable: NSViewRepresentable {
    let contentView: NSView

    func makeNSView(context: Context) -> NSView {
        let container = NSView()
        container.translatesAutoresizingMaskIntoConstraints = false
        contentView.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(contentView)
        NSLayoutConstraint.activate([
            contentView.topAnchor.constraint(equalTo: container.topAnchor),
            contentView.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            contentView.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            contentView.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])
        return container
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        // TabManager manages content internally; nothing to update here.
    }
}
