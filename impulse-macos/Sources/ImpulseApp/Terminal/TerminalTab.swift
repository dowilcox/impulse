import AppKit
import SwiftTerm

// MARK: - TerminalTab

/// Wraps a SwiftTerm `LocalProcessTerminalView` for use as a single terminal tab
/// in the Impulse IDE. Manages shell spawning, theming, and lifecycle notifications.
class TerminalTab: NSView, LocalProcessTerminalViewDelegate {

    // MARK: Public Properties

    /// Display title for this terminal tab; defaults to the shell name.
    private(set) var tabTitle: String

    /// Current working directory reported by the shell via OSC 7.
    private(set) var currentWorkingDirectory: String

    /// PID of the running shell process, or 0 if not yet spawned.
    private(set) var shellPid: pid_t = 0

    // MARK: Private Properties

    let terminalView: LocalProcessTerminalView

    /// Cached reference to the vertical scroller to avoid subview iteration on every layout.
    private weak var cachedScroller: NSScroller?

    /// Local event monitor for copy-on-select behaviour.
    private var mouseUpMonitor: Any?

    /// Whether copy-on-select is currently active.
    private var copyOnSelectEnabled: Bool = false

    /// Temp files/directories created for shell integration (cleaned up in deinit).
    private var shellIntegrationTempPaths: [URL] = []


    // MARK: Initializer

    override init(frame frameRect: NSRect) {
        let shellName = ImpulseCore.getUserLoginShellName()
        self.tabTitle = shellName
        self.currentWorkingDirectory = NSHomeDirectory()

        self.terminalView = LocalProcessTerminalView(frame: frameRect)
        super.init(frame: frameRect)

        terminalView.processDelegate = self
        addSubview(terminalView)
        setupConstraints()
        setupDragAndDrop()
        // Copy-on-select is not installed here; call setCopyOnSelect(enabled:)
        // after configureTerminal() to respect the user's setting.

    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    deinit {
        if let monitor = mouseUpMonitor {
            NSEvent.removeMonitor(monitor)
        }
        for path in shellIntegrationTempPaths {
            try? FileManager.default.removeItem(at: path)
        }
    }

    // MARK: Cleanup

    /// Terminate the shell process and release resources. Must be called before
    /// the tab is removed from the view hierarchy to ensure child processes
    /// (and any programs running inside the shell) are cleaned up.
    func terminateProcess() {
        // Kill the entire process group so child processes (lazygit, vim, etc.)
        // are terminated, not just the shell. SwiftTerm's terminate() only sends
        // SIGTERM to the shell PID.
        // Use getpgid() to get the actual process group ID rather than assuming
        // PID == PGID, which may not hold if setpgrp() was not called.
        let pid = terminalView.process?.shellPid ?? 0
        if pid > 0 {
            let pgid = getpgid(pid)
            if pgid > 0 {
                killpg(pgid, SIGHUP)
            } else {
                kill(pid, SIGHUP)
            }
        }
        terminalView.terminate()
    }

    // MARK: Copy on Select

    /// Enables or disables the copy-on-select behaviour at runtime.
    func setCopyOnSelect(enabled: Bool) {
        guard enabled != copyOnSelectEnabled else { return }
        copyOnSelectEnabled = enabled

        if enabled {
            mouseUpMonitor = NSEvent.addLocalMonitorForEvents(matching: .leftMouseUp) { [weak self] event in
                guard let self else { return event }
                let pt = self.terminalView.convert(event.locationInWindow, from: nil)
                guard self.terminalView.bounds.contains(pt) else { return event }
                DispatchQueue.main.async { [weak self] in
                    guard let self,
                          self.terminalView.selectionActive,
                          let text = self.terminalView.getSelection(),
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
        guard let urls = sender.draggingPasteboard.readObjects(forClasses: [NSURL.self],
                                                                options: [.urlReadingFileURLsOnly: true]) as? [URL] else {
            return false
        }

        // Build a space-separated, shell-escaped list of file paths.
        let paths = urls.map { shellEscape($0.path) }.joined(separator: " ")
        guard !paths.isEmpty else { return false }

        // Send the escaped paths directly to the terminal.
        let bytes = Array(paths.utf8)
        terminalView.send(data: bytes[...])
        return true
    }

    /// Characters considered safe for shell paths (no escaping needed).
    private static let shellSafeChars = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "/_.-"))

    /// Shell-escapes a file path for safe pasting into a terminal.
    private func shellEscape(_ path: String) -> String {
        // If the path contains no special characters, return as-is.
        if path.unicodeScalars.allSatisfy({ Self.shellSafeChars.contains($0) }) {
            return path
        }
        // Otherwise, wrap in single quotes with internal single quotes escaped.
        let escaped = path.replacingOccurrences(of: "'", with: "'\\''")
        return "'\(escaped)'"
    }

    // MARK: Auto Layout

    private func setupConstraints() {
        terminalView.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            terminalView.topAnchor.constraint(equalTo: topAnchor, constant: 8),
            terminalView.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            terminalView.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            terminalView.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -8),
        ])
    }

    // MARK: Scrollbar

    override func layout() {
        super.layout()
        // Defer so this runs after SwiftTerm's own layout resets the scroller frame.
        DispatchQueue.main.async { [weak self] in
            self?.resizeScroller()
        }
    }

    private func resizeScroller() {
        let scroller = cachedScroller ?? terminalView.subviews.compactMap { $0 as? NSScroller }.first
        cachedScroller = scroller
        guard let scroller else { return }
        scroller.controlSize = .mini
        let width = NSScroller.scrollerWidth(for: .mini, scrollerStyle: scroller.scrollerStyle)
        let tvBounds = terminalView.bounds
        scroller.frame = NSRect(
            x: tvBounds.maxX - width,
            y: tvBounds.minY,
            width: width,
            height: tvBounds.height
        )
    }

    // MARK: Configuration

    /// Apply visual settings and theme to the terminal view.
    func configureTerminal(settings: TerminalSettings, theme: TerminalTheme) {
        // Font
        let fontSize = CGFloat(settings.terminalFontSize)
        if !settings.terminalFontFamily.isEmpty,
           let customFont = NSFont(name: settings.terminalFontFamily, size: fontSize) {
            terminalView.font = customFont
        } else {
            terminalView.font = NSFont.monospacedSystemFont(ofSize: fontSize, weight: .regular)
        }

        // Colors
        applyTheme(theme: theme)

        // Cursor style
        let terminal = terminalView.getTerminal()
        switch settings.terminalCursorShape {
        case "beam":
            terminal.options.cursorStyle = settings.terminalCursorBlink ? .blinkBar : .steadyBar
        case "underline":
            terminal.options.cursorStyle = settings.terminalCursorBlink ? .blinkUnderline : .steadyUnderline
        default:
            terminal.options.cursorStyle = settings.terminalCursorBlink ? .blinkBlock : .steadyBlock
        }

        // Scrollback
        terminal.options.scrollback = settings.terminalScrollback

        // Copy on select
        setCopyOnSelect(enabled: settings.terminalCopyOnSelect)
    }

    /// Update terminal colors from a theme at runtime.
    func applyTheme(theme: TerminalTheme) {
        terminalView.nativeForegroundColor = NSColor(hex: theme.fg)
        terminalView.nativeBackgroundColor = NSColor(hex: theme.bg)

        let palette = theme.terminalPalette.map { hex in
            colorFromHex(hex)
        }
        if palette.count == 16 {
            terminalView.installColors(palette)
        }
    }

    // MARK: Shell Spawning

    /// Spawn the user's login shell inside this terminal.
    func spawnShell(initialDirectory: String? = nil) {
        let shellPath = ImpulseCore.getUserLoginShell()
        let shellName = (shellPath as NSString).lastPathComponent

        var environment: [String] = [
            "TERM=xterm-256color",
            "TERM_PROGRAM=Impulse",
            "COLORTERM=truecolor",
        ]

        // Dangerous linker/loader environment variables that could be used for
        // library injection attacks. Filter these out of the inherited environment.
        let dangerousEnvKeys: Set<String> = [
            "DYLD_INSERT_LIBRARIES", "DYLD_LIBRARY_PATH", "DYLD_FRAMEWORK_PATH",
            "DYLD_FALLBACK_LIBRARY_PATH", "DYLD_FALLBACK_FRAMEWORK_PATH",
            "LD_PRELOAD", "LD_LIBRARY_PATH", "LD_AUDIT", "LD_DEBUG",
            "LD_PROFILE", "LD_DYNAMIC_WEAK", "LD_BIND_NOW"
        ]

        // Inherit the current process environment
        for (key, value) in ProcessInfo.processInfo.environment {
            // Skip keys we set explicitly
            if key == "TERM" || key == "TERM_PROGRAM" || key == "COLORTERM" {
                continue
            }
            // Skip dangerous linker/loader variables
            if dangerousEnvKeys.contains(key) { continue }
            environment.append("\(key)=\(value)")
        }

        var args: [String] = []

        // Add shell integration (OSC 7 CWD tracking, OSC 133 command boundaries).
        // Each shell type requires a different injection method.
        let shellType = shellName.lowercased()
        if shellType == "fish" {
            if let script = ImpulseCore.getShellIntegrationScript(shell: shellType) {
                args.append(contentsOf: ["--login", "--init-command", script])
            } else {
                args.append("--login")
            }
        } else if shellType == "zsh" {
            // Zsh requires a ZDOTDIR trick: create a temp directory with wrapper
            // rc files that source the user's originals then inject integration.
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

                    environment.append("ZDOTDIR=\(zdotdir.path)")
                    args.append("--login")
                } catch {
                    // Fall back to plain login shell if temp files fail
                    args.append("--login")
                }
            } else {
                args.append("--login")
            }
        } else if shellType == "bash" {
            // Bash supports --rcfile to inject a custom rc that sources the
            // user's .bashrc then appends integration.
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
                    // Fall back to plain login shell
                    args.append("--login")
                }
            } else {
                args.append("--login")
            }
        }

        let workingDir = initialDirectory ?? currentWorkingDirectory

        terminalView.startProcess(
            executable: shellPath,
            args: args,
            environment: environment,
            execName: nil,
            currentDirectory: workingDir
        )
    }

    /// Send a text string (e.g. a shell command + newline) to the terminal's PTY.
    func sendCommand(_ text: String) {
        let bytes = Array(text.utf8) + [0x0A]  // Append newline (Enter)
        terminalView.send(data: bytes[...])
    }

    /// Make this terminal the first responder.
    func focus() {
        window?.makeFirstResponder(terminalView)
    }

    // MARK: LocalProcessTerminalViewDelegate

    func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {
        guard let directory = directory, !directory.isEmpty else { return }

        // The directory may come as a file:// URL from OSC 7
        let path: String
        if directory.hasPrefix("file://") {
            path = URL(string: directory)?.path ?? directory
        } else {
            path = directory
        }

        currentWorkingDirectory = path
        NotificationCenter.default.post(
            name: .terminalCwdChanged,
            object: self,
            userInfo: ["directory": path]
        )
    }

    func sizeChanged(source: LocalProcessTerminalView, newCols: Int, newRows: Int) {
        // Size changes are handled internally by SwiftTerm's PTY bridging.
        // No additional action needed.
    }

    func setTerminalTitle(source: LocalProcessTerminalView, title: String) {
        tabTitle = title
        NotificationCenter.default.post(
            name: .terminalTitleChanged,
            object: self,
            userInfo: ["title": title]
        )
    }

    func processTerminated(source: TerminalView, exitCode: Int32?) {
        NotificationCenter.default.post(
            name: .terminalProcessTerminated,
            object: self,
            userInfo: exitCode.map { ["exitCode": $0] }
        )
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

// MARK: - Color Helpers

/// Convert a hex color string to a SwiftTerm `Color` (UInt16 components, 0-65535 range).
private func colorFromHex(_ hex: String) -> Color {
    let cleaned = hex.trimmingCharacters(in: .whitespacesAndNewlines)
        .replacingOccurrences(of: "#", with: "")
    guard cleaned.count == 6, let value = UInt32(cleaned, radix: 16) else {
        return Color(red: 0, green: 0, blue: 0)
    }
    let r = UInt16((value >> 16) & 0xFF)
    let g = UInt16((value >> 8) & 0xFF)
    let b = UInt16(value & 0xFF)
    // Scale 8-bit (0-255) to 16-bit (0-65535)
    return Color(red: r * 257, green: g * 257, blue: b * 257)
}

// NSColor(hex:) is defined in Theme/Theme.swift
