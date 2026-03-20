import Foundation

// MARK: - Terminal Backend Configuration

/// Configuration for creating a new terminal backend.
/// Encoded as JSON and passed to the Rust FFI.
struct TerminalBackendConfig: Codable {
    var scrollbackLines: Int = 10_000
    var cursorShape: String = "Block"
    var cursorBlink: Bool = true
    var shellPath: String = ""
    var shellArgs: [String] = []
    var workingDirectory: String?
    var envVars: [String: String] = [:]
    var colors: TerminalBackendColors = TerminalBackendColors()

    enum CodingKeys: String, CodingKey {
        case scrollbackLines = "scrollback_lines"
        case cursorShape = "cursor_shape"
        case cursorBlink = "cursor_blink"
        case shellPath = "shell_path"
        case shellArgs = "shell_args"
        case workingDirectory = "working_directory"
        case envVars = "env_vars"
        case colors
    }
}

/// Color palette for the terminal backend.
struct TerminalBackendColors: Codable {
    var foreground: TerminalRgb = TerminalRgb(r: 220, g: 215, b: 186)
    var background: TerminalRgb = TerminalRgb(r: 31, g: 31, b: 40)
    var palette: [TerminalRgb] = Array(repeating: TerminalRgb(r: 0, g: 0, b: 0), count: 16)
}

/// RGB color value.
struct TerminalRgb: Codable {
    var r: UInt8
    var g: UInt8
    var b: UInt8
}

// MARK: - Grid Snapshot Types

/// A snapshot of the terminal grid, decoded from the Rust FFI JSON.
struct TerminalGridSnapshot: Codable {
    let cells: [[TerminalStyledCell]]
    let cursor: TerminalCursorState
    let hasSelection: Bool
    let selectionRanges: [[Int]]
    let cols: Int
    let lines: Int
    let mode: TerminalModeFlags

    enum CodingKeys: String, CodingKey {
        case cells, cursor, cols, lines, mode
        case hasSelection = "has_selection"
        case selectionRanges = "selection_ranges"
    }
}

/// A single styled cell in the grid.
struct TerminalStyledCell: Codable {
    let character: String
    let fg: TerminalRgb
    let bg: TerminalRgb
    let flags: UInt16
}

/// Cursor state.
struct TerminalCursorState: Codable {
    let row: Int
    let col: Int
    let shape: String
    let visible: Bool
}

/// Terminal mode flags.
struct TerminalModeFlags: Codable {
    let showCursor: Bool
    let appCursor: Bool
    let appKeypad: Bool
    let mouseReportClick: Bool
    let mouseMotion: Bool
    let mouseDrag: Bool
    let mouseSgr: Bool
    let bracketedPaste: Bool
    let focusInOut: Bool
    let altScreen: Bool
    let lineWrap: Bool

    enum CodingKeys: String, CodingKey {
        case showCursor = "show_cursor"
        case appCursor = "app_cursor"
        case appKeypad = "app_keypad"
        case mouseReportClick = "mouse_report_click"
        case mouseMotion = "mouse_motion"
        case mouseDrag = "mouse_drag"
        case mouseSgr = "mouse_sgr"
        case bracketedPaste = "bracketed_paste"
        case focusInOut = "focus_in_out"
        case altScreen = "alt_screen"
        case lineWrap = "line_wrap"
    }
}

// MARK: - Terminal Event Types

/// Events emitted by the terminal backend.
enum TerminalBackendEvent {
    case wakeup
    case titleChanged(String)
    case resetTitle
    case bell
    case childExited(Int32)
    case clipboardStore(String)
    case clipboardLoad
    case cursorBlinkingChange
    case exit
}

/// Raw event structure for JSON decoding.
private struct RawTerminalEvent: Codable {
    // Each variant is an enum with associated data.
    // serde serializes Rust enums as {"variant": data} or just "variant".
    let Wakeup: Bool?
    let TitleChanged: String?
    let ResetTitle: Bool?
    let Bell: Bool?
    let ChildExited: Int32?
    let ClipboardStore: String?
    let ClipboardLoad: Bool?
    let CursorBlinkingChange: Bool?
    let Exit: Bool?
}

// MARK: - Terminal Backend

/// Swift wrapper around the Rust terminal backend (impulse-terminal via FFI).
///
/// One instance per terminal tab/split. Manages the lifecycle of the Rust backend,
/// provides typed Swift APIs, and handles JSON encoding/decoding.
final class TerminalBackend {

    /// The opaque handle to the Rust terminal backend.
    private var handle: OpaquePointer?

    /// Cached JSON decoder for grid snapshots and events.
    private let decoder = JSONDecoder()

    /// Cached JSON encoder for configuration.
    private let encoder = JSONEncoder()

    /// Whether the backend has been shut down.
    private(set) var isShutdown = false

    /// Create a new terminal backend with the given configuration.
    init(config: TerminalBackendConfig, cols: UInt16, rows: UInt16, cellWidth: UInt16, cellHeight: UInt16) throws {
        encoder.outputFormatting = []
        let configData = try encoder.encode(config)
        guard let configJson = String(data: configData, encoding: .utf8) else {
            throw TerminalBackendError.configEncodingFailed
        }

        guard let h = ImpulseCore.terminalCreate(
            configJson: configJson,
            cols: cols,
            rows: rows,
            cellWidth: cellWidth,
            cellHeight: cellHeight
        ) else {
            throw TerminalBackendError.createFailed
        }
        self.handle = h
    }

    deinit {
        shutdown()
    }

    /// Send input bytes to the terminal's PTY.
    func write(_ data: Data) {
        guard let handle, !isShutdown else { return }
        ImpulseCore.terminalWrite(handle: handle, data: data)
    }

    /// Send a string as input to the terminal's PTY.
    func write(_ string: String) {
        guard let data = string.data(using: .utf8) else { return }
        write(data)
    }

    /// Send raw bytes to the terminal's PTY.
    func write(bytes: [UInt8]) {
        write(Data(bytes))
    }

    /// Resize the terminal grid and PTY.
    func resize(cols: UInt16, rows: UInt16, cellWidth: UInt16, cellHeight: UInt16) {
        guard let handle, !isShutdown else { return }
        ImpulseCore.terminalResize(handle: handle, cols: cols, rows: rows, cellWidth: cellWidth, cellHeight: cellHeight)
    }

    /// Get a snapshot of the visible terminal grid.
    func gridSnapshot() -> TerminalGridSnapshot? {
        guard let handle, !isShutdown else { return nil }
        guard let json = ImpulseCore.terminalGridSnapshot(handle: handle),
              let data = json.data(using: .utf8) else { return nil }
        return try? decoder.decode(TerminalGridSnapshot.self, from: data)
    }

    /// Poll for terminal events (non-blocking).
    func pollEvents() -> [TerminalBackendEvent] {
        guard let handle, !isShutdown else { return [] }
        guard let json = ImpulseCore.terminalPollEvents(handle: handle),
              let data = json.data(using: .utf8) else { return [] }

        // The Rust side serializes events as a JSON array of serde-tagged enums.
        // Each element is either a string ("Wakeup") or {"TitleChanged":"..."}.
        guard let rawArray = try? JSONSerialization.jsonObject(with: data) as? [Any] else {
            return []
        }

        var events: [TerminalBackendEvent] = []
        for item in rawArray {
            if let str = item as? String {
                switch str {
                case "Wakeup": events.append(.wakeup)
                case "ResetTitle": events.append(.resetTitle)
                case "Bell": events.append(.bell)
                case "ClipboardLoad": events.append(.clipboardLoad)
                case "CursorBlinkingChange": events.append(.cursorBlinkingChange)
                case "Exit": events.append(.exit)
                default: break
                }
            } else if let dict = item as? [String: Any] {
                if let title = dict["TitleChanged"] as? String {
                    events.append(.titleChanged(title))
                } else if let code = dict["ChildExited"] as? Int {
                    events.append(.childExited(Int32(code)))
                } else if let text = dict["ClipboardStore"] as? String {
                    events.append(.clipboardStore(text))
                }
            }
        }
        return events
    }

    /// Start a text selection at the given grid position.
    func startSelection(col: UInt16, row: UInt16, kind: String = "simple") {
        guard let handle, !isShutdown else { return }
        ImpulseCore.terminalStartSelection(handle: handle, col: col, row: row, kind: kind)
    }

    /// Update the current selection to the given grid position.
    func updateSelection(col: UInt16, row: UInt16) {
        guard let handle, !isShutdown else { return }
        ImpulseCore.terminalUpdateSelection(handle: handle, col: col, row: row)
    }

    /// Clear the current selection.
    func clearSelection() {
        guard let handle, !isShutdown else { return }
        ImpulseCore.terminalClearSelection(handle: handle)
    }

    /// Get the selected text, or nil if nothing is selected.
    func selectedText() -> String? {
        guard let handle, !isShutdown else { return nil }
        return ImpulseCore.terminalSelectedText(handle: handle)
    }

    /// Scroll the terminal viewport.
    func scroll(delta: Int32) {
        guard let handle, !isShutdown else { return }
        ImpulseCore.terminalScroll(handle: handle, delta: delta)
    }

    /// Get the current terminal mode flags.
    func mode() -> TerminalModeFlags? {
        guard let handle, !isShutdown else { return nil }
        guard let json = ImpulseCore.terminalMode(handle: handle),
              let data = json.data(using: .utf8) else { return nil }
        return try? decoder.decode(TerminalModeFlags.self, from: data)
    }

    /// Notify the terminal about focus change.
    func setFocus(_ focused: Bool) {
        guard let handle, !isShutdown else { return }
        ImpulseCore.terminalSetFocus(handle: handle, focused: focused)
    }

    /// Get the PID of the child shell process.
    func childPid() -> pid_t {
        guard let handle, !isShutdown else { return 0 }
        return pid_t(ImpulseCore.terminalChildPid(handle: handle))
    }

    /// Query the current working directory of the child process using macOS APIs.
    ///
    /// Uses `proc_pidinfo` with `PROC_PIDVNODEPATHINFO` to get the CWD of
    /// the shell process, which is more reliable than parsing OSC 7 sequences.
    func queryCwd() -> String? {
        let pid = childPid()
        guard pid > 0 else { return nil }

        var vnodeInfo = proc_vnodepathinfo()
        let size = MemoryLayout<proc_vnodepathinfo>.size
        let result = proc_pidinfo(pid, PROC_PIDVNODEPATHINFO, 0, &vnodeInfo, Int32(size))
        guard result == size else { return nil }

        let path = withUnsafePointer(to: &vnodeInfo.pvi_cdir.vip_path) { ptr in
            ptr.withMemoryRebound(to: CChar.self, capacity: Int(MAXPATHLEN)) { charPtr in
                String(cString: charPtr)
            }
        }
        return path.isEmpty ? nil : path
    }

    /// Shut down the terminal and kill the child process.
    func shutdown() {
        guard let handle, !isShutdown else { return }
        isShutdown = true
        ImpulseCore.terminalDestroy(handle: handle)
        self.handle = nil
    }
}

// MARK: - Errors

enum TerminalBackendError: Error {
    case configEncodingFailed
    case createFailed
}

// MARK: - Helpers

extension TerminalBackendConfig {
    /// Create a config from Impulse's terminal settings and theme.
    static func from(
        settings: TerminalSettings,
        theme: TerminalTheme,
        shellPath: String,
        shellArgs: [String],
        environment: [String: String],
        workingDirectory: String?
    ) -> TerminalBackendConfig {
        var config = TerminalBackendConfig()
        config.scrollbackLines = settings.terminalScrollback
        config.cursorShape = settings.terminalCursorShape.capitalized
        config.cursorBlink = settings.terminalCursorBlink
        config.shellPath = shellPath
        config.shellArgs = shellArgs
        config.workingDirectory = workingDirectory
        config.envVars = environment

        // Convert theme colors
        config.colors.foreground = hexToRgb(theme.fg)
        config.colors.background = hexToRgb(theme.bg)
        config.colors.palette = theme.terminalPalette.map { hexToRgb($0) }

        return config
    }
}

private func hexToRgb(_ hex: String) -> TerminalRgb {
    let cleaned = hex.trimmingCharacters(in: .whitespacesAndNewlines)
        .replacingOccurrences(of: "#", with: "")
    guard cleaned.count == 6, let value = UInt32(cleaned, radix: 16) else {
        return TerminalRgb(r: 0, g: 0, b: 0)
    }
    return TerminalRgb(
        r: UInt8((value >> 16) & 0xFF),
        g: UInt8((value >> 8) & 0xFF),
        b: UInt8(value & 0xFF)
    )
}
