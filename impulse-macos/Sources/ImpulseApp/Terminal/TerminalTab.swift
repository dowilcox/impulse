import AppKit
import os.log

// MARK: - TerminalTab

/// Wraps a `TerminalRenderer` (backed by impulse-terminal via FFI) for use as a
/// single terminal tab in the Impulse IDE. Manages shell spawning, theming, and
/// lifecycle notifications.
class TerminalTab: NSView {

    // MARK: Public Properties

    /// Display title for this terminal tab; defaults to the shell name.
    private(set) var tabTitle: String

    /// Current working directory reported by the shell via OSC 7.
    private(set) var currentWorkingDirectory: String

    // MARK: Private Properties

    /// The terminal renderer (NSView that draws the grid).
    let renderer: TerminalRenderer

    /// The terminal backend (Rust-side terminal emulation + PTY).
    private var backend: TerminalBackend?

    /// Local event monitor for copy-on-select behaviour.
    private var mouseUpMonitor: Any?

    /// Whether copy-on-select is currently active.
    private var copyOnSelectEnabled: Bool = false

    /// Temp files/directories created for shell integration (cleaned up in deinit).
    private var shellIntegrationTempPaths: [URL] = []

    /// Cached settings for use during shell spawning.
    private var currentSettings: TerminalSettings?
    private var currentTheme: TerminalTheme?

    /// Timer for periodic CWD polling via proc_pidinfo.
    private var cwdPollTimer: Timer?

    // MARK: Initializer

    override init(frame frameRect: NSRect) {
        let shellName = ImpulseCore.getUserLoginShellName()
        self.tabTitle = shellName
        self.currentWorkingDirectory = NSHomeDirectory()

        // Create renderer with default font; configureTerminal() updates it.
        self.renderer = TerminalRenderer(
            frame: frameRect,
            fontFamily: "JetBrains Mono",
            fontSize: 14
        )
        super.init(frame: frameRect)

        addSubview(renderer)
        setupConstraints()
        setupDragAndDrop()

        // Wire up event handling.
        renderer.onEvent = { [weak self] event in
            self?.handleBackendEvent(event)
        }
        renderer.onPaste = { [weak self] in
            self?.pasteFromClipboard()
        }
        renderer.onCopy = { [weak self] in
            self?.copySelection()
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    deinit {
        cwdPollTimer?.invalidate()
        renderer.stopRefreshLoop()
        if let monitor = mouseUpMonitor {
            NSEvent.removeMonitor(monitor)
        }
        for path in shellIntegrationTempPaths {
            try? FileManager.default.removeItem(at: path)
        }
        backend?.shutdown()
    }

    // MARK: Cleanup

    /// Terminate the shell process and release resources.
    func terminateProcess() {
        cwdPollTimer?.invalidate()
        cwdPollTimer = nil
        renderer.stopRefreshLoop()
        backend?.shutdown()
        backend = nil
    }

    // MARK: Copy on Select

    /// Enables or disables the copy-on-select behaviour at runtime.
    func setCopyOnSelect(enabled: Bool) {
        guard enabled != copyOnSelectEnabled else { return }
        copyOnSelectEnabled = enabled

        if enabled {
            mouseUpMonitor = NSEvent.addLocalMonitorForEvents(matching: .leftMouseUp) { [weak self] event in
                guard let self else { return event }
                let pt = self.renderer.convert(event.locationInWindow, from: nil)
                guard self.renderer.bounds.contains(pt) else { return event }
                DispatchQueue.main.async { [weak self] in
                    guard let self,
                          let text = self.backend?.selectedText(),
                          !text.isEmpty else { return }
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(text, forType: .string)
                }
                return event
            }
        } else {
            if let monitor = mouseUpMonitor {
                NSEvent.removeMonitor(monitor)
                mouseUpMonitor = nil
            }
        }
    }

    // MARK: Drag & Drop

    private func setupDragAndDrop() {
        registerForDraggedTypes([.fileURL])
    }

    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        guard sender.draggingPasteboard.canReadObject(forClasses: [NSURL.self],
                                                       options: [.urlReadingFileURLsOnly: true]) else {
            return []
        }
        return .copy
    }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        guard let backend,
              let urls = sender.draggingPasteboard.readObjects(forClasses: [NSURL.self],
                                                                options: [.urlReadingFileURLsOnly: true]) as? [URL] else {
            return false
        }

        let paths = urls.map { $0.path.shellEscaped }.joined(separator: " ")
        guard !paths.isEmpty else { return false }

        // Wrap in bracketed paste if the terminal supports it.
        if let mode = backend.mode(), mode.bracketedPaste {
            backend.write(bytes: [0x1B, 0x5B, 0x32, 0x30, 0x30, 0x7E]) // \e[200~
        }
        backend.write(paths)
        if let mode = backend.mode(), mode.bracketedPaste {
            backend.write(bytes: [0x1B, 0x5B, 0x32, 0x30, 0x31, 0x7E]) // \e[201~
        }
        return true
    }

    // MARK: Auto Layout

    private func setupConstraints() {
        renderer.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            renderer.topAnchor.constraint(equalTo: topAnchor),
            renderer.leadingAnchor.constraint(equalTo: leadingAnchor),
            renderer.trailingAnchor.constraint(equalTo: trailingAnchor),
            renderer.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    // MARK: Configuration

    /// Apply visual settings and theme to the terminal.
    func configureTerminal(settings: TerminalSettings, theme: TerminalTheme) {
        currentSettings = settings
        currentTheme = theme

        // Update font.
        renderer.updateFont(
            family: settings.terminalFontFamily,
            size: CGFloat(settings.terminalFontSize)
        )

        // Copy on select.
        setCopyOnSelect(enabled: settings.terminalCopyOnSelect)
    }

    /// Update terminal colors from a theme at runtime.
    func applyTheme(theme: TerminalTheme) {
        currentTheme = theme
        // Colors are applied via the backend config at spawn time.
        // For runtime theme changes, we'd need to update the backend colors.
        // For now, trigger a redraw with current snapshot.
        renderer.needsDisplay = true
    }

    // MARK: Shell Spawning

    /// Spawn the user's login shell inside this terminal.
    func spawnShell(initialDirectory: String? = nil, initialCommand: String? = nil) {
        let shellPath = ImpulseCore.getUserLoginShell()
        let shellName = (shellPath as NSString).lastPathComponent

        var envVars: [String: String] = [
            "TERM": "xterm-256color",
            "TERM_PROGRAM": "Impulse",
            "COLORTERM": "truecolor",
        ]

        let dangerousEnvKeys: Set<String> = [
            "DYLD_INSERT_LIBRARIES", "DYLD_LIBRARY_PATH", "DYLD_FRAMEWORK_PATH",
            "DYLD_FALLBACK_LIBRARY_PATH", "DYLD_FALLBACK_FRAMEWORK_PATH",
            "LD_PRELOAD", "LD_LIBRARY_PATH", "LD_AUDIT", "LD_DEBUG",
            "LD_PROFILE", "LD_DYNAMIC_WEAK", "LD_BIND_NOW"
        ]

        for (key, value) in ProcessInfo.processInfo.environment {
            if key == "TERM" || key == "TERM_PROGRAM" || key == "COLORTERM" { continue }
            if dangerousEnvKeys.contains(key) { continue }
            envVars[key] = value
        }

        var args: [String] = []

        // Shell integration injection.
        let shellType = shellName.lowercased()
        if shellType == "fish" {
            if let script = ImpulseCore.getShellIntegrationScript(shell: shellType) {
                args.append(contentsOf: ["--login", "--init-command", script])
            } else {
                args.append("--login")
            }
        } else if shellType == "zsh" {
            if let script = ImpulseCore.getShellIntegrationScript(shell: shellType) {
                let home = NSHomeDirectory()
                let zdotdir = FileManager.default.temporaryDirectory
                    .appendingPathComponent("impulse-zsh-\(ProcessInfo.processInfo.processIdentifier)-\(UUID().uuidString)")
                do {
                    try FileManager.default.createDirectory(at: zdotdir, withIntermediateDirectories: true)
                    shellIntegrationTempPaths.append(zdotdir)

                    let zshenv = "if [ -f '\(home)/.zshenv' ]; then\n    source '\(home)/.zshenv'\nfi\n"
                    try zshenv.write(to: zdotdir.appendingPathComponent(".zshenv"), atomically: true, encoding: .utf8)

                    let zprofile = "if [ -f '\(home)/.zprofile' ]; then\n    source '\(home)/.zprofile'\nfi\n"
                    try zprofile.write(to: zdotdir.appendingPathComponent(".zprofile"), atomically: true, encoding: .utf8)

                    let zlogin = "if [ -f '\(home)/.zlogin' ]; then\n    source '\(home)/.zlogin'\nfi\n"
                    try zlogin.write(to: zdotdir.appendingPathComponent(".zlogin"), atomically: true, encoding: .utf8)

                    let zshrc = """
                        export ZDOTDIR='\(home)'
                        if [ -f '\(home)/.zshrc' ]; then
                            source '\(home)/.zshrc'
                        fi
                        \(script)
                        """
                    try zshrc.write(to: zdotdir.appendingPathComponent(".zshrc"), atomically: true, encoding: .utf8)

                    envVars["ZDOTDIR"] = zdotdir.path
                    args.append("--login")
                } catch {
                    args.append("--login")
                }
            } else {
                args.append("--login")
            }
        } else if shellType == "bash" {
            if let script = ImpulseCore.getShellIntegrationScript(shell: shellType) {
                let home = NSHomeDirectory()
                let rcPath = FileManager.default.temporaryDirectory
                    .appendingPathComponent("impulse-bash-rc-\(ProcessInfo.processInfo.processIdentifier)-\(UUID().uuidString)")
                let rcContent = """
                    if [ -f '\(home)/.bashrc' ]; then
                        source '\(home)/.bashrc'
                    fi
                    \(script)
                    """
                do {
                    try rcContent.write(to: rcPath, atomically: true, encoding: .utf8)
                    shellIntegrationTempPaths.append(rcPath)
                    args.append(contentsOf: ["--rcfile", rcPath.path])
                } catch {
                    args.append("--login")
                }
            } else {
                args.append("--login")
            }
        }

        let workingDir = initialDirectory ?? currentWorkingDirectory
        let settings = currentSettings ?? TerminalSettings()
        let theme = currentTheme ?? TerminalTheme()

        // Calculate grid dimensions from current view size.
        let fm = renderer.fontMetrics
        let (cols, rows) = fm.gridSize(
            viewWidth: renderer.bounds.width > 0 ? renderer.bounds.width : 800,
            viewHeight: renderer.bounds.height > 0 ? renderer.bounds.height : 400,
            padding: renderer.padding
        )

        // Build the backend config.
        let config = TerminalBackendConfig.from(
            settings: settings,
            theme: theme,
            shellPath: shellPath,
            shellArgs: args,
            environment: envVars,
            workingDirectory: workingDir
        )

        do {
            let newBackend = try TerminalBackend(
                config: config,
                cols: UInt16(cols),
                rows: UInt16(rows),
                cellWidth: UInt16(fm.cellWidth),
                cellHeight: UInt16(fm.cellHeight)
            )
            self.backend = newBackend
            renderer.backend = newBackend
            renderer.startRefreshLoop()
            startCwdPolling()

            if let initialCommand {
                sendCommand(initialCommand)
            }
        } catch {
            os_log(.error, "Failed to create terminal backend: %{public}@", "\(error)")
        }
    }

    /// Send a text string (e.g. a shell command + newline) to the terminal's PTY.
    func sendCommand(_ text: String) {
        var bytes = Array(text.utf8)
        bytes.append(0x0A) // newline
        backend?.write(bytes: bytes)
    }

    /// Make this terminal the first responder.
    func focus() {
        window?.makeFirstResponder(renderer)
    }

    // MARK: Paste Support

    /// Paste from the system clipboard, with trailing newline stripping and
    /// image fallback support.
    func pasteFromClipboard() {
        guard let backend else { return }
        let clipboard = NSPasteboard.general

        // Prefer text.
        if var text = clipboard.string(forType: .string) {
            while text.hasSuffix("\n") || text.hasSuffix("\r") {
                text.removeLast()
            }
            text = text.replacingOccurrences(of: "\r\n", with: "\n")
            text = text.replacingOccurrences(of: "\r", with: "\n")
            guard !text.isEmpty else { return }

            if let mode = backend.mode(), mode.bracketedPaste {
                backend.write(bytes: [0x1B, 0x5B, 0x32, 0x30, 0x30, 0x7E]) // \e[200~
            }
            backend.write(text)
            if let mode = backend.mode(), mode.bracketedPaste {
                backend.write(bytes: [0x1B, 0x5B, 0x32, 0x30, 0x31, 0x7E]) // \e[201~
            }
            return
        }

        // Fall back to image: save as temp PNG, paste the path.
        if let image = clipboard.readObjects(forClasses: [NSImage.self], options: nil)?.first as? NSImage,
           let tiffData = image.tiffRepresentation,
           let bitmap = NSBitmapImageRep(data: tiffData),
           let pngData = bitmap.representation(using: .png, properties: [:]) {
            let timestamp = Int(Date().timeIntervalSince1970 * 1000)
            let tmpPath = NSTemporaryDirectory() + "impulse-clipboard-\(timestamp).png"
            do {
                try pngData.write(to: URL(fileURLWithPath: tmpPath))
                try FileManager.default.setAttributes(
                    [.posixPermissions: 0o600], ofItemAtPath: tmpPath
                )
                backend.write(tmpPath.shellEscaped)
            } catch {
                os_log(.error, "Failed to save clipboard image: %{public}@", error.localizedDescription)
            }
        }
    }

    /// Copy the current selection to the system clipboard.
    func copySelection() {
        guard let text = backend?.selectedText(), !text.isEmpty else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    // MARK: CWD Tracking

    /// Start polling the child process's current working directory.
    /// Uses macOS proc_pidinfo to query the shell's CWD every second.
    private func startCwdPolling() {
        cwdPollTimer?.invalidate()
        cwdPollTimer = Timer.scheduledTimer(withTimeInterval: 1.0, repeats: true) { [weak self] _ in
            self?.pollCwd()
        }
    }

    private func pollCwd() {
        guard let backend, !backend.isShutdown else { return }
        guard let cwd = backend.queryCwd(), !cwd.isEmpty, cwd != currentWorkingDirectory else { return }

        currentWorkingDirectory = cwd
        NotificationCenter.default.post(
            name: .terminalCwdChanged,
            object: self,
            userInfo: ["directory": cwd]
        )
    }

    // MARK: Backend Event Handling

    private func handleBackendEvent(_ event: TerminalBackendEvent) {
        switch event {
        case .titleChanged(let title):
            tabTitle = title
            NotificationCenter.default.post(
                name: .terminalTitleChanged,
                object: self,
                userInfo: ["title": title]
            )

        case .resetTitle:
            tabTitle = ImpulseCore.getUserLoginShellName()
            NotificationCenter.default.post(
                name: .terminalTitleChanged,
                object: self,
                userInfo: ["title": tabTitle]
            )

        case .bell:
            NSSound.beep()

        case .childExited(let code):
            os_log(.info, "Terminal child exited with code %d", code)
            NotificationCenter.default.post(
                name: .terminalProcessTerminated,
                object: self,
                userInfo: ["exitCode": code]
            )

        case .clipboardStore(let text):
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(text, forType: .string)

        case .clipboardLoad:
            // Terminal requested clipboard contents (OSC 52 load).
            if let text = NSPasteboard.general.string(forType: .string) {
                backend?.write(text)
            }

        case .exit:
            os_log(.info, "Terminal exit event received")
            NotificationCenter.default.post(
                name: .terminalProcessTerminated,
                object: self,
                userInfo: nil
            )

        case .cursorBlinkingChange:
            break

        case .wakeup:
            // Handled by the renderer's refresh loop.
            break
        }
    }
}

// MARK: - Settings / Theme Data Structures

/// Terminal-specific settings extracted from the application settings JSON.
struct TerminalSettings {
    var terminalFontSize: Int = 14
    var terminalFontFamily: String = ""
    var terminalCursorShape: String = "block" // "block", "beam", "underline"
    var terminalCursorBlink: Bool = true
    var terminalScrollback: Int = 10_000
    var lastDirectory: String = ""
    var terminalCopyOnSelect: Bool = true
}

/// Terminal color theme definition. Hex color strings (e.g. "#1F1F28").
struct TerminalTheme {
    var bg: String = "#1F1F28"
    var fg: String = "#DCD7BA"
    /// 16-color ANSI palette as hex strings. Order: black, red, green, yellow,
    /// blue, magenta, cyan, white, then bright variants.
    var terminalPalette: [String] = [
        "#090618", "#C34043", "#76946A", "#C0A36E",
        "#7E9CD8", "#957FB8", "#6A9589", "#C8C093",
        "#727169", "#E82424", "#98BB6C", "#E6C384",
        "#7FB4CA", "#938AA9", "#7AA89F", "#DCD7BA",
    ]
}
