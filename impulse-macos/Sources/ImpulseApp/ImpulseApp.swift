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
        let args = CommandLine.arguments

        // Handle CLI-only LSP management flags before launching the GUI.
        if args.contains("--install-lsp-servers") {
            let result = ImpulseCore.lspInstall()
            switch result {
            case .success(let message):
                print(message)
                exit(0)
            case .failure(let error):
                fputs("Error: \(error)\n", stderr)
                exit(1)
            }
        }

        if args.contains("--check-lsp-servers") {
            let servers = ImpulseCore.lspCheckStatus()
            if servers.isEmpty {
                print("No managed LSP servers found.")
            } else {
                for server in servers {
                    let name = server["name"] as? String ?? "unknown"
                    let installed = server["installed"] as? Bool ?? false
                    let version = server["version"] as? String
                    let status = installed ? "installed" : "not installed"
                    if let ver = version {
                        print("\(name): \(status) (v\(ver))")
                    } else {
                        print("\(name): \(status)")
                    }
                }
            }
            exit(0)
        }

        // Collect non-flag arguments as file paths to open.
        let filePaths = args.dropFirst().filter { !$0.hasPrefix("-") }

        let app = NSApplication.shared
        app.setActivationPolicy(.regular)

        let delegate = AppDelegate()
        if !filePaths.isEmpty {
            delegate.pendingFiles = filePaths.map { path in
                // Resolve relative paths against the current working directory.
                if path.hasPrefix("/") {
                    return path
                }
                let cwd = FileManager.default.currentDirectoryPath
                return (cwd as NSString).appendingPathComponent(path)
            }
        }
        app.delegate = delegate

        // Build the shared menu bar before the run loop starts so that it is
        // available as soon as the first window appears.
        app.mainMenu = MenuBuilder.buildMainMenu()

        app.run()
    }
}
