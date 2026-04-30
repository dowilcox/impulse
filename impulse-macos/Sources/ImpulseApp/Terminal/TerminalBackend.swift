import Darwin
import Foundation

// MARK: - Configuration

/// Configuration passed to the Rust terminal backend via JSON.
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

/// Terminal color configuration (foreground, background, and 16-color palette).
struct TerminalBackendColors: Codable {
    var foreground: TerminalRgb = TerminalRgb(r: 220, g: 215, b: 186)
    var background: TerminalRgb = TerminalRgb(r: 31, g: 31, b: 40)
    var palette: [TerminalRgb] = Array(repeating: TerminalRgb(r: 0, g: 0, b: 0), count: 16)
}

/// An RGB color triplet.
struct TerminalRgb: Codable {
    var r: UInt8
    var g: UInt8
    var b: UInt8
}

// MARK: - Mode Flags

/// Decoded terminal mode bitflags from the Rust side.
///
/// The Rust `TerminalMode` is serialized via serde as `{"bits": N}`.
struct TerminalModeFlags: Codable {
    let bits: UInt16

    var showCursor: Bool { bits & (1 << 0) != 0 }
    var appCursor: Bool { bits & (1 << 1) != 0 }
    var appKeypad: Bool { bits & (1 << 2) != 0 }
    var mouseReportClick: Bool { bits & (1 << 3) != 0 }
    var mouseMotion: Bool { bits & (1 << 4) != 0 }
    var mouseDrag: Bool { bits & (1 << 5) != 0 }
    var mouseSgr: Bool { bits & (1 << 6) != 0 }
    var bracketedPaste: Bool { bits & (1 << 7) != 0 }
    var focusInOut: Bool { bits & (1 << 8) != 0 }
    var altScreen: Bool { bits & (1 << 9) != 0 }
    var lineWrap: Bool { bits & (1 << 10) != 0 }
}

// MARK: - Events

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
    case cwdChanged(String)
    case promptStart
    case commandStart
    case commandEnd(Int32)
    case attentionRequest(String)
    case notification(title: String, body: String)
}

// MARK: - Grid Buffer Reader

/// Provides typed read access to the binary grid snapshot buffer.
///
/// The buffer layout matches `impulse-terminal`'s `buffer.rs` format:
///
///     [Header: 16 bytes]
///       cols(u16) | lines(u16) | cursor_row(u16) | cursor_col(u16)
///       cursor_shape(u8) | cursor_visible(u8) | mode_flags(u16)
///       selection_range_count(u16) | search_match_range_count(u16)
///     [Selection ranges: N * 6 bytes each]
///       row(u16) | start_col(u16) | end_col(u16)
///     [Search match ranges: M * 6 bytes each]
///       row(u16) | start_col(u16) | end_col(u16)
///     [Cell data: cols * lines * 12 bytes each]
///       codepoint(u32) | fg_r(u8) | fg_g(u8) | fg_b(u8)
///       bg_r(u8) | bg_g(u8) | bg_b(u8) | flags(u16)
struct GridBufferReader {
    let pointer: UnsafePointer<UInt8>
    let size: Int

    // Header constants
    private static let fixedHeaderSize = 16
    private static let cellStride = 12
    private static let rangeEntrySize = 6

    var cols: Int { Int(readU16(at: 0)) }
    var lines: Int { Int(readU16(at: 2)) }

    var cursorRow: Int { Int(readU16(at: 4)) }
    var cursorCol: Int { Int(readU16(at: 6)) }
    /// Cursor shape: 0=Block, 1=Beam, 2=Underline, 3=HollowBlock, 4=Hidden
    var cursorShape: UInt8 { pointer[8] }
    var cursorVisible: Bool { pointer[9] != 0 }

    var modeFlags: UInt16 { readU16(at: 10) }
    var showCursor: Bool { modeFlags & (1 << 0) != 0 }
    var appCursor: Bool { modeFlags & (1 << 1) != 0 }
    var mouseReportClick: Bool { modeFlags & (1 << 3) != 0 }
    var mouseMotion: Bool { modeFlags & (1 << 4) != 0 }
    var mouseDrag: Bool { modeFlags & (1 << 5) != 0 }
    var mouseSgr: Bool { modeFlags & (1 << 6) != 0 }
    var bracketedPaste: Bool { modeFlags & (1 << 7) != 0 }

    var selectionRangeCount: Int { Int(readU16(at: 12)) }
    var searchMatchRangeCount: Int { Int(readU16(at: 14)) }

    /// Offset where cell data begins (after header + all range entries).
    var cellDataOffset: Int {
        Self.fixedHeaderSize + (selectionRangeCount + searchMatchRangeCount) * Self.rangeEntrySize
    }

    /// Read a selection range at the given index.
    func selectionRange(at index: Int) -> (row: Int, startCol: Int, endCol: Int) {
        let base = Self.fixedHeaderSize + index * Self.rangeEntrySize
        return (Int(readU16(at: base)), Int(readU16(at: base + 2)), Int(readU16(at: base + 4)))
    }

    /// Read a search match range at the given index.
    func searchMatchRange(at index: Int) -> (row: Int, startCol: Int, endCol: Int) {
        let base = Self.fixedHeaderSize + selectionRangeCount * Self.rangeEntrySize + index * Self.rangeEntrySize
        return (Int(readU16(at: base)), Int(readU16(at: base + 2)), Int(readU16(at: base + 4)))
    }

    /// Read cell data at a grid position.
    ///
    /// Returns the cell's codepoint as a `UnicodeScalar`, foreground/background
    /// RGB components, and attribute flags.
    @inline(__always)
    func cell(row: Int, col: Int) -> (character: UnicodeScalar, fgR: UInt8, fgG: UInt8, fgB: UInt8, bgR: UInt8, bgG: UInt8, bgB: UInt8, flags: UInt16) {
        let offset = cellDataOffset + (row * cols + col) * Self.cellStride
        let codepoint = readU32(at: offset)
        let scalar: UnicodeScalar
        if let s = UnicodeScalar(codepoint) {
            scalar = s
        } else {
            scalar = UnicodeScalar(UInt8(0x20))  // space fallback
        }
        return (
            scalar,
            pointer[offset + 4], pointer[offset + 5], pointer[offset + 6],
            pointer[offset + 7], pointer[offset + 8], pointer[offset + 9],
            readU16(at: offset + 10)
        )
    }

    // Cell flag constants (matching CellFlags in grid.rs)
    static let flagBold: UInt16 = 1 << 0
    static let flagItalic: UInt16 = 1 << 1
    static let flagUnderline: UInt16 = 1 << 2
    static let flagStrikethrough: UInt16 = 1 << 3
    static let flagDim: UInt16 = 1 << 4
    static let flagInverse: UInt16 = 1 << 5
    static let flagHidden: UInt16 = 1 << 6
    static let flagWideChar: UInt16 = 1 << 7
    static let flagWideCharSpacer: UInt16 = 1 << 8
    static let flagHyperlink: UInt16 = 1 << 13

    // MARK: - Private helpers

    @inline(__always)
    private func readU16(at offset: Int) -> UInt16 {
        UInt16(pointer[offset]) | (UInt16(pointer[offset + 1]) << 8)
    }

    @inline(__always)
    private func readU32(at offset: Int) -> UInt32 {
        UInt32(pointer[offset])
            | (UInt32(pointer[offset + 1]) << 8)
            | (UInt32(pointer[offset + 2]) << 16)
            | (UInt32(pointer[offset + 3]) << 24)
    }
}

// MARK: - Terminal Backend

/// Swift wrapper around the Rust terminal backend accessed through FFI.
///
/// Manages the lifecycle of a terminal instance: creation, input/output,
/// resize, selection, search, and shutdown. The grid state is accessed via
/// `gridSnapshot()` which returns a `GridBufferReader` for zero-copy
/// reading of the binary buffer.
final class TerminalBackend {
    private var handle: OpaquePointer?
    private var snapshotBuffer: UnsafeMutablePointer<UInt8>?
    private var snapshotBufferSize: Int = 0
    private(set) var isShutdown = false

    private let decoder = JSONDecoder()
    private let encoder = JSONEncoder()

    init(config: TerminalBackendConfig, cols: UInt16, rows: UInt16, cellWidth: UInt16, cellHeight: UInt16) throws {
        let configData = try encoder.encode(config)
        guard let configJson = String(data: configData, encoding: .utf8) else {
            throw TerminalBackendError.configEncodingFailed
        }
        guard let h = ImpulseCore.terminalCreate(configJson: configJson, cols: cols, rows: rows, cellWidth: cellWidth, cellHeight: cellHeight) else {
            throw TerminalBackendError.createFailed
        }
        self.handle = h

        // Allocate snapshot buffer.
        let bufSize = ImpulseCore.terminalGridSnapshotSize(handle: h)
        self.snapshotBufferSize = bufSize
        self.snapshotBuffer = .allocate(capacity: bufSize)
    }

    deinit {
        shutdown()
        snapshotBuffer?.deallocate()
    }

    // MARK: - Input

    func write(_ data: Data) {
        guard let handle, !isShutdown else { return }
        ImpulseCore.terminalWrite(handle: handle, data: data)
    }

    func write(_ string: String) {
        guard let data = string.data(using: .utf8) else { return }
        write(data)
    }

    func write(bytes: [UInt8]) {
        write(Data(bytes))
    }

    // MARK: - Resize

    func resize(cols: UInt16, rows: UInt16, cellWidth: UInt16, cellHeight: UInt16) {
        guard let handle, !isShutdown else { return }
        ImpulseCore.terminalResize(handle: handle, cols: cols, rows: rows, cellWidth: cellWidth, cellHeight: cellHeight)

        // Reallocate snapshot buffer if needed.
        let newSize = ImpulseCore.terminalGridSnapshotSize(handle: handle)
        if newSize > snapshotBufferSize {
            snapshotBuffer?.deallocate()
            snapshotBuffer = .allocate(capacity: newSize)
            snapshotBufferSize = newSize
        }
    }

    // MARK: - Grid Snapshot

    func gridSnapshot() -> GridBufferReader? {
        guard let handle, !isShutdown, let buf = snapshotBuffer else { return nil }
        let written = ImpulseCore.terminalGridSnapshot(handle: handle, buffer: buf, bufferSize: snapshotBufferSize)
        guard written > 0 else { return nil }
        return GridBufferReader(pointer: UnsafePointer(buf), size: written)
    }

    // MARK: - Events

    func pollEvents() -> [TerminalBackendEvent] {
        guard let handle, !isShutdown else { return [] }
        guard let json = ImpulseCore.terminalPollEvents(handle: handle),
              let data = json.data(using: .utf8),
              let rawArray = try? JSONSerialization.jsonObject(with: data) as? [Any]
        else {
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
                case "PromptStart": events.append(.promptStart)
                case "CommandStart": events.append(.commandStart)
                default: break
                }
            } else if let dict = item as? [String: Any] {
                if let title = dict["TitleChanged"] as? String {
                    events.append(.titleChanged(title))
                } else if let code = dict["ChildExited"] as? Int {
                    events.append(.childExited(Int32(code)))
                } else if let text = dict["ClipboardStore"] as? String {
                    events.append(.clipboardStore(text))
                } else if let path = dict["CwdChanged"] as? String {
                    events.append(.cwdChanged(path))
                } else if let code = dict["CommandEnd"] as? Int {
                    events.append(.commandEnd(Int32(code)))
                } else if let value = dict["AttentionRequest"] as? String {
                    events.append(.attentionRequest(value))
                } else if let payload = dict["Notification"] as? [String: Any] {
                    let title = payload["title"] as? String ?? "Terminal"
                    let body = payload["body"] as? String ?? ""
                    events.append(.notification(title: title, body: body))
                }
            }
        }
        return events
    }

    // MARK: - Selection

    func startSelection(col: UInt16, row: UInt16, kind: UInt8 = 0) {
        guard let handle, !isShutdown else { return }
        ImpulseCore.terminalStartSelection(handle: handle, col: col, row: row, kind: kind)
    }

    func updateSelection(col: UInt16, row: UInt16) {
        guard let handle, !isShutdown else { return }
        ImpulseCore.terminalUpdateSelection(handle: handle, col: col, row: row)
    }

    func clearSelection() {
        guard let handle, !isShutdown else { return }
        ImpulseCore.terminalClearSelection(handle: handle)
    }

    func selectedText() -> String? {
        guard let handle, !isShutdown else { return nil }
        return ImpulseCore.terminalSelectedText(handle: handle)
    }

    // MARK: - Scroll

    func scroll(delta: Int32) {
        guard let handle, !isShutdown else { return }
        ImpulseCore.terminalScroll(handle: handle, delta: delta)
    }

    func scrollToBottom() {
        guard let handle, !isShutdown else { return }
        ImpulseCore.terminalScrollToBottom(handle: handle)
    }

    // MARK: - Mode / Focus

    func mode() -> TerminalModeFlags? {
        guard let handle, !isShutdown else { return nil }
        guard let json = ImpulseCore.terminalMode(handle: handle),
              let data = json.data(using: .utf8)
        else { return nil }
        return try? decoder.decode(TerminalModeFlags.self, from: data)
    }

    func setFocus(_ focused: Bool) {
        guard let handle, !isShutdown else { return }
        ImpulseCore.terminalSetFocus(handle: handle, focused: focused)
    }

    // MARK: - Process Info

    func childPid() -> pid_t {
        guard let handle, !isShutdown else { return 0 }
        return pid_t(ImpulseCore.terminalChildPid(handle: handle))
    }

    /// Queries the child process's current working directory via `proc_pidinfo`.
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

    // MARK: - Search

    func search(_ pattern: String) {
        guard let handle, !isShutdown else { return }
        _ = ImpulseCore.terminalSearch(handle: handle, pattern: pattern)
    }

    func searchNext() {
        guard let handle, !isShutdown else { return }
        _ = ImpulseCore.terminalSearchNext(handle: handle)
    }

    func searchPrev() {
        guard let handle, !isShutdown else { return }
        _ = ImpulseCore.terminalSearchPrev(handle: handle)
    }

    func searchClear() {
        guard let handle, !isShutdown else { return }
        ImpulseCore.terminalSearchClear(handle: handle)
    }

    // MARK: - Colors

    func setColors(config: TerminalBackendConfig) {
        guard let handle, !isShutdown else { return }
        guard let data = try? encoder.encode(config),
              let json = String(data: data, encoding: .utf8) else { return }
        ImpulseCore.terminalSetColors(handle: handle, configJson: json)
    }

    /// Returns the OSC 8 hyperlink URI at the given grid cell, or nil.
    func hyperlinkAt(col: Int, row: Int) -> String? {
        guard let handle, !isShutdown, col >= 0, row >= 0 else { return nil }
        return ImpulseCore.terminalHyperlinkAt(
            handle: handle, col: UInt32(col), row: UInt32(row)
        )
    }

    // MARK: - Lifecycle

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

// MARK: - Config Helpers

extension TerminalBackendConfig {
    /// Builds a backend configuration from the existing `TerminalSettings` and
    /// `TerminalTheme` types (defined in `TerminalTab.swift`).
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
        config.colors.foreground = hexToRgb(theme.fg)
        config.colors.background = hexToRgb(theme.bg)
        config.colors.palette = theme.terminalPalette.map { hexToRgb($0) }
        return config
    }
}

/// Converts a hex color string (e.g. "#1F1F28") to a `TerminalRgb` value.
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
