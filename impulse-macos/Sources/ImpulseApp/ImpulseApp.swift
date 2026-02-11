import AppKit

// MARK: - Application Entry Point

/// Impulse macOS application entry point.
///
/// We use a traditional NSApplication-based launch rather than SwiftUI's @main App
/// protocol because the app requires deep AppKit integration: NSSplitView,
/// NSToolbar with custom tab segments, WKWebView for Monaco, and SwiftTerm for
/// terminal emulation. Running the NSApplication run loop directly gives full
/// control over the responder chain, menu bar, and window lifecycle.

@main
struct ImpulseApp {
    static func main() {
        let app = NSApplication.shared
        app.setActivationPolicy(.regular)

        let delegate = AppDelegate()
        app.delegate = delegate

        // Build the shared menu bar before the run loop starts so that it is
        // available as soon as the first window appears.
        app.mainMenu = MenuBuilder.buildMainMenu()

        app.run()
    }
}
