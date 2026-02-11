import AppKit
import SwiftTerm

// MARK: - Notifications

extension Notification.Name {
    static let terminalTitleChanged = Notification.Name("impulse.terminalTitleChanged")
    static let terminalCwdChanged = Notification.Name("impulse.terminalCwdChanged")
    static let terminalProcessTerminated = Notification.Name("impulse.terminalProcessTerminated")
}

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

    private let terminalView: LocalProcessTerminalView

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
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
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
        terminalView.send(bytes[...])
        return true
    }

    /// Shell-escapes a file path for safe pasting into a terminal.
    private func shellEscape(_ path: String) -> String {
        // If the path contains no special characters, return as-is.
        let safeChars = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "/_.-"))
        if path.unicodeScalars.allSatisfy({ safeChars.contains($0) }) {
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
            terminalView.topAnchor.constraint(equalTo: topAnchor),
            terminalView.leadingAnchor.constraint(equalTo: leadingAnchor),
            terminalView.trailingAnchor.constraint(equalTo: trailingAnchor),
            terminalView.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
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

        // Inherit the current process environment
        for (key, value) in ProcessInfo.processInfo.environment {
            // Skip keys we set explicitly
            if key == "TERM" || key == "TERM_PROGRAM" || key == "COLORTERM" {
                continue
            }
            environment.append("\(key)=\(value)")
        }

        var args: [String] = []

        // Add shell integration where possible
        let shellType = shellName.lowercased()
        if shellType == "fish" {
            if let script = ImpulseCore.getShellIntegrationScript(shell: shellType) {
                args.append(contentsOf: ["--login", "--init-command", script])
            } else {
                args.append("--login")
            }
        } else if shellType == "zsh" {
            args.append("--login")
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
        terminalView.send(bytes[...])
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

    func sizeChanged(source: TerminalView, newCols: Int, newRows: Int) {
        // Size changes are handled internally by SwiftTerm's PTY bridging.
        // No additional action needed.
    }

    func setTerminalTitle(source: TerminalView, title: String) {
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
