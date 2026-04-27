import AppKit
import os.log

// MARK: - TerminalTab

/// Wraps a `TerminalRenderer` backed by `TerminalBackend` (alacritty_terminal)
/// for use as a single terminal tab in the Impulse IDE. Manages shell spawning,
/// theming, and lifecycle notifications.
class TerminalTab: NSView {

    // MARK: Public Properties

    /// Display title for this terminal tab; defaults to the shell name.
    private(set) var tabTitle: String

    /// Current working directory reported by the shell via CWD polling.
    private(set) var currentWorkingDirectory: String

    /// The renderer view that draws the terminal grid.
    let renderer: TerminalRenderer

    // MARK: Private Properties

    private var backend: TerminalBackend?

    /// Whether copy-on-select is currently active.
    private var copyOnSelectEnabled: Bool = false

    /// Temp files/directories created for shell integration (cleaned up in deinit).
    private var shellIntegrationTempPaths: [URL] = []

    /// Stored settings/theme for later use when creating the backend.
    private var currentSettings: TerminalSettings?
    private var currentTheme: TerminalTheme?

    /// Timer for polling the child process CWD.
    private var cwdPollTimer: Timer?


    // MARK: Initializer

    override init(frame frameRect: NSRect) {
        let shellName = ImpulseCore.getUserLoginShellName()
        self.tabTitle = shellName
        self.currentWorkingDirectory = NSHomeDirectory()

        self.renderer = TerminalRenderer(
            frame: frameRect,
            fontFamily: "JetBrains Mono",
            fontSize: 14
        )
        super.init(frame: frameRect)

        addSubview(renderer)
        setupConstraints()
        setupDragAndDrop()
        wireRendererCallbacks()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    deinit {
        cwdPollTimer?.invalidate()
        for path in shellIntegrationTempPaths {
            try? FileManager.default.removeItem(at: path)
        }
    }

    // MARK: Renderer Callbacks

    private func wireRendererCallbacks() {
        renderer.onEvent = { [weak self] event in
            guard let self else { return }
            self.handleBackendEvent(event)
        }
        renderer.onPaste = { [weak self] in
            self?.pasteFromClipboard()
        }
        renderer.onCopy = { [weak self] in
            self?.copySelection()
        }
        renderer.onSelectionFinished = { [weak self] in
            guard let self, self.copyOnSelectEnabled else { return }
            self.copySelection()
        }
    }

    private func handleBackendEvent(_ event: TerminalBackendEvent) {
        switch event {
        case .wakeup:
            break // Handled by renderer refresh loop.
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
            if currentSettings?.terminalBell ?? true {
                NSSound.beep()
            }
        case .childExited(let code):
            NotificationCenter.default.post(
                name: .terminalProcessTerminated,
                object: self,
                userInfo: ["exitCode": code]
            )
        case .clipboardStore(let text):
            if currentSettings?.terminalAllowOsc52Write ?? true {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(text, forType: .string)
            }
        case .clipboardLoad:
            if currentSettings?.terminalAllowOsc52Read ?? false,
               let text = NSPasteboard.general.string(forType: .string) {
                backend?.write(text)
            }
        case .cursorBlinkingChange:
            break
        case .exit:
            NotificationCenter.default.post(
                name: .terminalProcessTerminated,
                object: self,
                userInfo: nil
            )
        case .cwdChanged(let path):
            currentWorkingDirectory = path
            NotificationCenter.default.post(
                name: .terminalCwdChanged,
                object: self,
                userInfo: ["directory": path]
            )
        case .promptStart:
            // TODO: scroll-to-prompt navigation. Needs alacritty absolute-line
            // tracking exposed through FFI so prompt positions survive scrollback.
            break
        case .commandStart:
            // TODO: command timing — record start time per command.
            break
        case .commandEnd(_):
            // TODO: surface exit code in the status bar or tab title.
            break
        }
    }

    // MARK: Cleanup

    /// Terminate the shell process and release resources. Must be called before
    /// the tab is removed from the view hierarchy to ensure child processes
    /// (and any programs running inside the shell) are cleaned up.
    func terminateProcess() {
        cwdPollTimer?.invalidate()
        cwdPollTimer = nil

        // Detach the backend from the renderer BEFORE stopping the display
        // link.  CVDisplayLinkStop blocks until the current tick() callback
        // completes; if tick() is mid-FFI call we'd deadlock.  Nil-ing the
        // reference makes tick() return immediately via its guard.
        let backendToShutdown = backend
        backend = nil
        renderer.backend = nil
        renderer.stopRefreshLoop()

        guard let backendToShutdown, !backendToShutdown.isShutdown else { return }

        // Send signals and destroy the backend on a background queue so the
        // main thread is never blocked by PTY teardown.
        let pid = backendToShutdown.childPid()
        DispatchQueue.global(qos: .utility).async { [weak self] in
            if pid > 0 {
                let descendants = self?.collectDescendants(of: pid) ?? []

                let pgid = getpgid(pid)
                if pgid > 0 {
                    killpg(pgid, SIGHUP)
                } else {
                    kill(pid, SIGHUP)
                }

                for desc in descendants {
                    kill(desc, SIGTERM)
                }

                if !descendants.isEmpty {
                    self?.escalateKill(shellPid: pid, descendants: descendants)
                }
            }
            backendToShutdown.shutdown()
        }
    }

    /// Recursively collect all descendant PIDs of a given process using
    /// `proc_listchildpids()`. Returns PIDs in leaf-first order.
    private func collectDescendants(of pid: pid_t) -> [pid_t] {
        let count = proc_listchildpids(pid, nil, 0)
        guard count > 0 else { return [] }

        let bufferSize = Int(count) * MemoryLayout<pid_t>.size
        var pids = [pid_t](repeating: 0, count: Int(count))
        let actual = pids.withUnsafeMutableBufferPointer { buf in
            proc_listchildpids(pid, buf.baseAddress, Int32(bufferSize))
        }
        let childCount = Int(actual) / MemoryLayout<pid_t>.size
        guard childCount > 0 else { return [] }

        var result: [pid_t] = []
        for i in 0..<childCount {
            let child = pids[i]
            guard child > 0 else { continue }
            result.append(contentsOf: collectDescendants(of: child))
            result.append(child)
        }
        return result
    }

    /// After a grace period, SIGKILL any descendants still alive and reap zombies.
    private func escalateKill(shellPid: pid_t, descendants: [pid_t]) {
        let allPids = descendants + [shellPid]
        DispatchQueue.global().asyncAfter(deadline: .now() + 2.0) {
            for pid in allPids {
                if kill(pid, 0) == 0 {
                    kill(pid, SIGKILL)
                }
            }
            for pid in allPids {
                var status: Int32 = 0
                waitpid(pid, &status, WNOHANG)
            }
        }
    }

    // MARK: Copy on Select

    /// Enables or disables the copy-on-select behaviour at runtime.
    func setCopyOnSelect(enabled: Bool) {
        copyOnSelectEnabled = enabled
    }

    // MARK: Clipboard

    /// Paste text from the system clipboard to the terminal PTY.
    func pasteFromClipboard() {
        let clipboard = NSPasteboard.general

        // Prefer text when available.
        if var text = clipboard.string(forType: .string) {
            // Strip trailing newlines/carriage returns.
            while text.hasSuffix("\n") || text.hasSuffix("\r") {
                text.removeLast()
            }

            // Normalize CRLF and standalone CR to LF.
            text = text.replacingOccurrences(of: "\r\n", with: "\n")
            text = text.replacingOccurrences(of: "\r", with: "\n")

            guard !text.isEmpty else { return }
            guard let backend else { return }

            writePastePayload(text, to: backend)
            return
        }

        // Fall back to image: save as a temp PNG and paste the path.
        // TUI apps like Claude Code detect image attachments from a pasted
        // path only when it arrives inside a bracketed paste event, so we
        // write the raw path (no shell escaping) wrapped in the same
        // bracketed-paste markers as the text branch.
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
                guard let backend else { return }
                writePastePayload(tmpPath, to: backend)
            } catch {
                os_log(.error, "Failed to save clipboard image: %{public}@", error.localizedDescription)
            }
        }
    }

    private func writePastePayload(_ payload: String, to backend: TerminalBackend) {
        guard backend.mode()?.bracketedPaste ?? false else {
            backend.write(payload)
            return
        }

        // Drop embedded paste-mode markers so a poisoned clipboard cannot break
        // out of bracketed paste and inject commands into the shell.
        let sanitized = payload
            .replacingOccurrences(of: "\u{1b}[201~", with: "")
            .replacingOccurrences(of: "\u{1b}[200~", with: "")
        backend.write("\u{1b}[200~\(sanitized)\u{1b}[201~")
    }

    /// Copy the current selection to the system clipboard.
    func copySelection() {
        guard let text = backend?.selectedText(), !text.isEmpty else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
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
        guard let urls = sender.draggingPasteboard.readObjects(forClasses: [NSURL.self],
                                                                options: [.urlReadingFileURLsOnly: true]) as? [URL] else {
            return false
        }

        let paths = urls.map { $0.path.shellEscaped }.joined(separator: " ")
        guard !paths.isEmpty, let backend else { return false }

        writePastePayload(paths, to: backend)
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
        self.currentSettings = settings
        self.currentTheme = theme

        // Font
        let fontSize = CGFloat(settings.terminalFontSize)
        let fontFamily = settings.terminalFontFamily.isEmpty ? "JetBrains Mono" : settings.terminalFontFamily
        renderer.updateFont(family: fontFamily, size: fontSize)

        // Cursor shape (0=Block, 1=Beam, 2=Underline)
        renderer.cursorShapeOverride = switch settings.terminalCursorShape.lowercased() {
        case "beam": 1
        case "underline": 2
        default: 0
        }

        // Cursor blink
        renderer.cursorBlinkEnabled = settings.terminalCursorBlink

        renderer.defaultBackgroundColor = cgColorFromHex(theme.bg)
        renderer.defaultBackgroundRgb = hexToRgbBytes(theme.bg)
        renderer.selectionColor = cgColorFromHex(theme.selection)
        renderer.cursorColor = cgColorFromHex(theme.cursor)

        // Palette (used for bold-is-bright substitution)
        renderer.paletteRgb = theme.terminalPalette.map { hex in
            let rgb = hexToRgbBytes(hex)
            return (rgb.0, rgb.1, rgb.2)
        }

        // Bold-is-bright palette substitution
        renderer.boldIsBright = settings.terminalBoldIsBright

        // Auto-scroll on output
        renderer.scrollOnOutput = settings.terminalScrollOnOutput

        // Copy on select
        setCopyOnSelect(enabled: settings.terminalCopyOnSelect)
    }

    /// Update terminal colors from a theme at runtime.
    func applyTheme(theme: TerminalTheme) {
        currentTheme = theme
        // Build a config with the new colors and push it to the backend.
        let settings = currentSettings ?? TerminalSettings()
        let config = TerminalBackendConfig.from(
            settings: settings,
            theme: theme,
            shellPath: "",
            shellArgs: [],
            environment: [:],
            workingDirectory: nil
        )
        backend?.setColors(config: config)
        renderer.defaultBackgroundColor = cgColorFromHex(theme.bg)
        renderer.defaultBackgroundRgb = hexToRgbBytes(theme.bg)
        renderer.selectionColor = cgColorFromHex(theme.selection)
        renderer.cursorColor = cgColorFromHex(theme.cursor)
        renderer.needsDisplay = true
    }

    // MARK: Shell Spawning

    /// Spawn the user's login shell inside this terminal.
    /// If `initialCommand` is provided, it is sent to the PTY immediately after
    /// the process starts.
    func spawnShell(initialDirectory: String? = nil, initialCommand: String? = nil) {
        let shellPath = ImpulseCore.getUserLoginShell()
        let shellName = (shellPath as NSString).lastPathComponent

        var envDict: [String: String] = [
            "TERM": "xterm-256color",
            "TERM_PROGRAM": "Impulse",
            "COLORTERM": "truecolor",
        ]

        // Dangerous linker/loader environment variables.
        let dangerousEnvKeys: Set<String> = [
            "DYLD_INSERT_LIBRARIES", "DYLD_LIBRARY_PATH", "DYLD_FRAMEWORK_PATH",
            "DYLD_FALLBACK_LIBRARY_PATH", "DYLD_FALLBACK_FRAMEWORK_PATH",
            "LD_PRELOAD", "LD_LIBRARY_PATH", "LD_AUDIT", "LD_DEBUG",
            "LD_PROFILE", "LD_DYNAMIC_WEAK", "LD_BIND_NOW"
        ]

        // Inherit the current process environment.
        for (key, value) in ProcessInfo.processInfo.environment {
            if key == "TERM" || key == "TERM_PROGRAM" || key == "COLORTERM" { continue }
            if dangerousEnvKeys.contains(key) { continue }
            envDict[key] = value
        }

        var args: [String] = []

        // Add shell integration (OSC 7 CWD tracking, OSC 133 command boundaries).
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

                    envDict["ZDOTDIR"] = zdotdir.path
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

        let config = TerminalBackendConfig.from(
            settings: settings,
            theme: theme,
            shellPath: shellPath,
            shellArgs: args,
            environment: envDict,
            workingDirectory: workingDir
        )

        // Calculate grid dimensions from renderer bounds.
        let metrics = renderer.fontMetrics
        let viewWidth = renderer.bounds.width > 0 ? renderer.bounds.width : 800
        let viewHeight = renderer.bounds.height > 0 ? renderer.bounds.height : 600
        let (cols, rows) = metrics.gridSize(
            viewWidth: viewWidth, viewHeight: viewHeight, padding: renderer.padding
        )

        do {
            let newBackend = try TerminalBackend(
                config: config,
                cols: UInt16(cols),
                rows: UInt16(rows),
                cellWidth: UInt16(metrics.cellWidth),
                cellHeight: UInt16(metrics.cellHeight)
            )
            self.backend = newBackend
            renderer.backend = newBackend
            renderer.startRefreshLoop()

            // Start CWD polling timer (5 second interval).
            startCwdPolling()
        } catch {
            os_log(.error, "Failed to create terminal backend: %{public}@", String(describing: error))
        }

        if let initialCommand {
            sendCommand(initialCommand)
        }
    }

    /// Send a text string (e.g. a shell command + newline) to the terminal's PTY.
    func sendCommand(_ text: String) {
        backend?.write(text + "\n")
    }

    /// Make this terminal the first responder.
    func focus() {
        window?.makeFirstResponder(renderer)
    }

    // MARK: Search

    func search(_ pattern: String) { backend?.search(pattern) }
    func searchNext() { backend?.searchNext() }
    func searchPrev() { backend?.searchPrev() }
    func searchClear() { backend?.searchClear() }

    // MARK: CWD Polling

    private func startCwdPolling() {
        cwdPollTimer?.invalidate()
        cwdPollTimer = Timer.scheduledTimer(withTimeInterval: 5.0, repeats: true) { [weak self] _ in
            guard let self, let backend = self.backend, !backend.isShutdown else { return }
            if let cwd = backend.queryCwd(), cwd != self.currentWorkingDirectory {
                self.currentWorkingDirectory = cwd
                NotificationCenter.default.post(
                    name: .terminalCwdChanged,
                    object: self,
                    userInfo: ["directory": cwd]
                )
            }
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
    var terminalBell: Bool = false
    var terminalScrollOnOutput: Bool = true
    var terminalAllowHyperlink: Bool = true
    var terminalBoldIsBright: Bool = true
    var terminalAllowOsc52Write: Bool = true
    var terminalAllowOsc52Read: Bool = false
}

/// Convert a hex color string (e.g. "#DCD7BA" or "#7AA2F740") to a CGColor.
/// Falls back to black on parse failure.
private func cgColorFromHex(_ hex: String) -> CGColor {
    NSColor(hex: hex).cgColor
}

/// Parse a hex color string into an (r, g, b) byte tuple.
/// Falls back to white (255,255,255) on parse failure.
func hexToRgbBytes(_ hex: String) -> (UInt8, UInt8, UInt8) {
    let cleaned = hex.trimmingCharacters(in: .whitespacesAndNewlines)
        .replacingOccurrences(of: "#", with: "")
    guard (cleaned.count == 6 || cleaned.count == 8),
          let value = UInt32(cleaned.prefix(6), radix: 16) else {
        return (255, 255, 255)
    }
    return (
        UInt8((value >> 16) & 0xFF),
        UInt8((value >> 8) & 0xFF),
        UInt8(value & 0xFF)
    )
}

/// Terminal color theme definition. Hex color strings (e.g. "#1F1F28").
struct TerminalTheme {
    var bg: String = "#1F1F28"
    var fg: String = "#DCD7BA"
    var selection: String = "#7AA2F740"
    var cursor: String = "#DCD7BA"
    /// 16-color ANSI palette as hex strings. Order: black, red, green, yellow,
    /// blue, magenta, cyan, white, then bright variants.
    var terminalPalette: [String] = [
        "#090618", "#C34043", "#76946A", "#C0A36E",
        "#7E9CD8", "#957FB8", "#6A9589", "#C8C093",
        "#727169", "#E82424", "#98BB6C", "#E6C384",
        "#7FB4CA", "#938AA9", "#7AA89F", "#DCD7BA",
    ]
}
