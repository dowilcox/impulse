import Foundation
import os.log

// MARK: - Sub-Types

/// A formatter command that runs on save before the editor reloads the file.
struct FormatOnSave: Codable {
    var command: String
    var args: [String]

    init(command: String = "", args: [String] = []) {
        self.command = command
        self.args = args
    }
}

/// Per-file-type overrides for editor settings (tab width, spaces, formatter).
struct FileTypeOverride: Codable {
    var pattern: String
    var tabWidth: Int?
    var useSpaces: Bool?
    var formatOnSave: FormatOnSave?

    enum CodingKeys: String, CodingKey {
        case pattern
        case tabWidth = "tab_width"
        case useSpaces = "use_spaces"
        case formatOnSave = "format_on_save"
    }

    init(pattern: String = "", tabWidth: Int? = nil, useSpaces: Bool? = nil,
         formatOnSave: FormatOnSave? = nil) {
        self.pattern = pattern
        self.tabWidth = tabWidth
        self.useSpaces = useSpaces
        self.formatOnSave = formatOnSave
    }
}

/// A command that runs automatically when a file matching the pattern is saved.
struct CommandOnSave: Codable {
    var name: String
    var command: String
    var args: [String]
    var filePattern: String
    var reloadFile: Bool

    enum CodingKeys: String, CodingKey {
        case name, command, args
        case filePattern = "file_pattern"
        case reloadFile = "reload_file"
    }

    init(name: String = "", command: String = "", args: [String] = [],
         filePattern: String = "", reloadFile: Bool = false) {
        self.name = name
        self.command = command
        self.args = args
        self.filePattern = filePattern
        self.reloadFile = reloadFile
    }
}

/// A user-defined keybinding that runs a command.
struct CustomKeybinding: Codable {
    var name: String
    var key: String
    var command: String
    var args: [String]

    init(name: String = "", key: String = "", command: String = "", args: [String] = []) {
        self.name = name
        self.key = key
        self.command = command
        self.args = args
    }
}

// MARK: - Settings

/// Application settings, persisted to `~/Library/Application Support/impulse/settings.json`.
///
/// This mirrors the same schema used by the Linux frontend so that users who
/// share a settings file across platforms get consistent behavior. The JSON keys
/// use snake_case to match the Linux (serde) format.
struct Settings: Codable {
    // -- Window --
    var windowWidth: Int
    var windowHeight: Int
    var sidebarVisible: Bool
    var sidebarWidth: Int
    var lastDirectory: String
    var openFiles: [String]

    // -- Editor --
    var autoSave: Bool
    var fontSize: Int
    var fontFamily: String
    var tabWidth: Int
    var useSpaces: Bool
    var showLineNumbers: Bool
    var showRightMargin: Bool
    var rightMarginPosition: Int
    var wordWrap: Bool
    var highlightCurrentLine: Bool
    var minimapEnabled: Bool
    var renderWhitespace: String
    var stickyScroll: Bool
    var bracketPairColorization: Bool
    var indentGuides: Bool
    var fontLigatures: Bool
    var folding: Bool
    var scrollBeyondLastLine: Bool
    var smoothScrolling: Bool
    var editorCursorStyle: String
    var editorCursorBlinking: String

    // -- Terminal --
    var terminalScrollback: Int
    var terminalCursorShape: String
    var terminalCursorBlink: Bool
    var terminalBell: Bool
    var terminalFontFamily: String
    var terminalFontSize: Int
    var terminalCopyOnSelect: Bool
    var terminalScrollOnOutput: Bool
    var terminalAllowHyperlink: Bool
    var terminalBoldIsBright: Bool

    // -- Editor (additional) --
    var editorLineHeight: Int
    var editorAutoClosingBrackets: String

    // -- Sidebar --
    var sidebarShowHidden: Bool

    // -- Appearance --
    var colorScheme: String

    // -- Custom commands --
    var commandsOnSave: [CommandOnSave]
    var customKeybindings: [CustomKeybinding]

    // -- Keybinding overrides --
    var keybindingOverrides: [String: String]

    // -- Per-file-type overrides --
    var fileTypeOverrides: [FileTypeOverride]

    // MARK: CodingKeys (snake_case to match Linux JSON)

    enum CodingKeys: String, CodingKey {
        case windowWidth = "window_width"
        case windowHeight = "window_height"
        case sidebarVisible = "sidebar_visible"
        case sidebarWidth = "sidebar_width"
        case lastDirectory = "last_directory"
        case openFiles = "open_files"
        case autoSave = "auto_save"
        case fontSize = "font_size"
        case fontFamily = "font_family"
        case tabWidth = "tab_width"
        case useSpaces = "use_spaces"
        case showLineNumbers = "show_line_numbers"
        case showRightMargin = "show_right_margin"
        case rightMarginPosition = "right_margin_position"
        case wordWrap = "word_wrap"
        case highlightCurrentLine = "highlight_current_line"
        case minimapEnabled = "minimap_enabled"
        case renderWhitespace = "render_whitespace"
        case stickyScroll = "sticky_scroll"
        case bracketPairColorization = "bracket_pair_colorization"
        case indentGuides = "indent_guides"
        case fontLigatures = "font_ligatures"
        case folding
        case scrollBeyondLastLine = "scroll_beyond_last_line"
        case smoothScrolling = "smooth_scrolling"
        case editorCursorStyle = "editor_cursor_style"
        case editorCursorBlinking = "editor_cursor_blinking"
        case terminalScrollback = "terminal_scrollback"
        case terminalCursorShape = "terminal_cursor_shape"
        case terminalCursorBlink = "terminal_cursor_blink"
        case terminalBell = "terminal_bell"
        case terminalFontFamily = "terminal_font_family"
        case terminalFontSize = "terminal_font_size"
        case terminalCopyOnSelect = "terminal_copy_on_select"
        case terminalScrollOnOutput = "terminal_scroll_on_output"
        case terminalAllowHyperlink = "terminal_allow_hyperlink"
        case terminalBoldIsBright = "terminal_bold_is_bright"
        case editorLineHeight = "editor_line_height"
        case editorAutoClosingBrackets = "editor_auto_closing_brackets"
        case sidebarShowHidden = "sidebar_show_hidden"
        case colorScheme = "color_scheme"
        case commandsOnSave = "commands_on_save"
        case customKeybindings = "custom_keybindings"
        case keybindingOverrides = "keybinding_overrides"
        case fileTypeOverrides = "file_type_overrides"
    }

    // MARK: Defaults

    static var `default`: Settings {
        Settings(
            windowWidth: 1200,
            windowHeight: 800,
            sidebarVisible: false,
            sidebarWidth: 250,
            lastDirectory: "",
            openFiles: [],
            autoSave: false,
            fontSize: 14,
            fontFamily: "SF Mono",
            tabWidth: 4,
            useSpaces: true,
            showLineNumbers: true,
            showRightMargin: true,
            rightMarginPosition: 120,
            wordWrap: false,
            highlightCurrentLine: true,
            minimapEnabled: false,
            renderWhitespace: "selection",
            stickyScroll: false,
            bracketPairColorization: true,
            indentGuides: true,
            fontLigatures: true,
            folding: true,
            scrollBeyondLastLine: false,
            smoothScrolling: false,
            editorCursorStyle: "line",
            editorCursorBlinking: "smooth",
            terminalScrollback: 10000,
            terminalCursorShape: "block",
            terminalCursorBlink: true,
            terminalBell: false,
            terminalFontFamily: "SF Mono",
            terminalFontSize: 14,
            terminalCopyOnSelect: true,
            terminalScrollOnOutput: false,
            terminalAllowHyperlink: true,
            terminalBoldIsBright: false,
            editorLineHeight: 0,
            editorAutoClosingBrackets: "languageDefined",
            sidebarShowHidden: false,
            colorScheme: "nord",
            commandsOnSave: [],
            customKeybindings: [],
            keybindingOverrides: [:],
            fileTypeOverrides: []
        )
    }

    // MARK: Fault-tolerant decoding

    /// Decodes settings with defaults for missing keys so old settings files
    /// don't break when new fields are added.
    init(from decoder: Decoder) throws {
        let d = Settings.default
        let c = try decoder.container(keyedBy: CodingKeys.self)
        windowWidth = (try? c.decode(Int.self, forKey: .windowWidth)) ?? d.windowWidth
        windowHeight = (try? c.decode(Int.self, forKey: .windowHeight)) ?? d.windowHeight
        sidebarVisible = (try? c.decode(Bool.self, forKey: .sidebarVisible)) ?? d.sidebarVisible
        sidebarWidth = (try? c.decode(Int.self, forKey: .sidebarWidth)) ?? d.sidebarWidth
        lastDirectory = (try? c.decode(String.self, forKey: .lastDirectory)) ?? d.lastDirectory
        openFiles = (try? c.decode([String].self, forKey: .openFiles)) ?? d.openFiles
        autoSave = (try? c.decode(Bool.self, forKey: .autoSave)) ?? d.autoSave
        fontSize = (try? c.decode(Int.self, forKey: .fontSize)) ?? d.fontSize
        fontFamily = (try? c.decode(String.self, forKey: .fontFamily)) ?? d.fontFamily
        tabWidth = (try? c.decode(Int.self, forKey: .tabWidth)) ?? d.tabWidth
        useSpaces = (try? c.decode(Bool.self, forKey: .useSpaces)) ?? d.useSpaces
        showLineNumbers = (try? c.decode(Bool.self, forKey: .showLineNumbers)) ?? d.showLineNumbers
        showRightMargin = (try? c.decode(Bool.self, forKey: .showRightMargin)) ?? d.showRightMargin
        rightMarginPosition = (try? c.decode(Int.self, forKey: .rightMarginPosition)) ?? d.rightMarginPosition
        wordWrap = (try? c.decode(Bool.self, forKey: .wordWrap)) ?? d.wordWrap
        highlightCurrentLine = (try? c.decode(Bool.self, forKey: .highlightCurrentLine)) ?? d.highlightCurrentLine
        minimapEnabled = (try? c.decode(Bool.self, forKey: .minimapEnabled)) ?? d.minimapEnabled
        renderWhitespace = (try? c.decode(String.self, forKey: .renderWhitespace)) ?? d.renderWhitespace
        stickyScroll = (try? c.decode(Bool.self, forKey: .stickyScroll)) ?? d.stickyScroll
        bracketPairColorization = (try? c.decode(Bool.self, forKey: .bracketPairColorization)) ?? d.bracketPairColorization
        indentGuides = (try? c.decode(Bool.self, forKey: .indentGuides)) ?? d.indentGuides
        fontLigatures = (try? c.decode(Bool.self, forKey: .fontLigatures)) ?? d.fontLigatures
        folding = (try? c.decode(Bool.self, forKey: .folding)) ?? d.folding
        scrollBeyondLastLine = (try? c.decode(Bool.self, forKey: .scrollBeyondLastLine)) ?? d.scrollBeyondLastLine
        smoothScrolling = (try? c.decode(Bool.self, forKey: .smoothScrolling)) ?? d.smoothScrolling
        editorCursorStyle = (try? c.decode(String.self, forKey: .editorCursorStyle)) ?? d.editorCursorStyle
        editorCursorBlinking = (try? c.decode(String.self, forKey: .editorCursorBlinking)) ?? d.editorCursorBlinking
        terminalScrollback = (try? c.decode(Int.self, forKey: .terminalScrollback)) ?? d.terminalScrollback
        terminalCursorShape = (try? c.decode(String.self, forKey: .terminalCursorShape)) ?? d.terminalCursorShape
        terminalCursorBlink = (try? c.decode(Bool.self, forKey: .terminalCursorBlink)) ?? d.terminalCursorBlink
        terminalBell = (try? c.decode(Bool.self, forKey: .terminalBell)) ?? d.terminalBell
        terminalFontFamily = (try? c.decode(String.self, forKey: .terminalFontFamily)) ?? d.terminalFontFamily
        terminalFontSize = (try? c.decode(Int.self, forKey: .terminalFontSize)) ?? d.terminalFontSize
        terminalCopyOnSelect = (try? c.decode(Bool.self, forKey: .terminalCopyOnSelect)) ?? d.terminalCopyOnSelect
        terminalScrollOnOutput = (try? c.decode(Bool.self, forKey: .terminalScrollOnOutput)) ?? d.terminalScrollOnOutput
        terminalAllowHyperlink = (try? c.decode(Bool.self, forKey: .terminalAllowHyperlink)) ?? d.terminalAllowHyperlink
        terminalBoldIsBright = (try? c.decode(Bool.self, forKey: .terminalBoldIsBright)) ?? d.terminalBoldIsBright
        editorLineHeight = (try? c.decode(Int.self, forKey: .editorLineHeight)) ?? d.editorLineHeight
        editorAutoClosingBrackets = (try? c.decode(String.self, forKey: .editorAutoClosingBrackets)) ?? d.editorAutoClosingBrackets
        sidebarShowHidden = (try? c.decode(Bool.self, forKey: .sidebarShowHidden)) ?? d.sidebarShowHidden
        colorScheme = (try? c.decode(String.self, forKey: .colorScheme)) ?? d.colorScheme
        commandsOnSave = (try? c.decode([CommandOnSave].self, forKey: .commandsOnSave)) ?? d.commandsOnSave
        customKeybindings = (try? c.decode([CustomKeybinding].self, forKey: .customKeybindings)) ?? d.customKeybindings
        keybindingOverrides = (try? c.decode([String: String].self, forKey: .keybindingOverrides)) ?? d.keybindingOverrides
        fileTypeOverrides = (try? c.decode([FileTypeOverride].self, forKey: .fileTypeOverrides)) ?? d.fileTypeOverrides
    }

    /// Memberwise initializer used by `Settings.default`.
    init(windowWidth: Int, windowHeight: Int, sidebarVisible: Bool, sidebarWidth: Int,
         lastDirectory: String, openFiles: [String], autoSave: Bool, fontSize: Int,
         fontFamily: String, tabWidth: Int, useSpaces: Bool, showLineNumbers: Bool,
         showRightMargin: Bool, rightMarginPosition: Int, wordWrap: Bool,
         highlightCurrentLine: Bool, minimapEnabled: Bool, renderWhitespace: String,
         stickyScroll: Bool, bracketPairColorization: Bool, indentGuides: Bool,
         fontLigatures: Bool, folding: Bool, scrollBeyondLastLine: Bool,
         smoothScrolling: Bool, editorCursorStyle: String, editorCursorBlinking: String,
         terminalScrollback: Int, terminalCursorShape: String, terminalCursorBlink: Bool,
         terminalBell: Bool, terminalFontFamily: String, terminalFontSize: Int,
         terminalCopyOnSelect: Bool, terminalScrollOnOutput: Bool,
         terminalAllowHyperlink: Bool, terminalBoldIsBright: Bool,
         editorLineHeight: Int, editorAutoClosingBrackets: String,
         sidebarShowHidden: Bool, colorScheme: String,
         commandsOnSave: [CommandOnSave], customKeybindings: [CustomKeybinding],
         keybindingOverrides: [String: String], fileTypeOverrides: [FileTypeOverride]) {
        self.windowWidth = windowWidth
        self.windowHeight = windowHeight
        self.sidebarVisible = sidebarVisible
        self.sidebarWidth = sidebarWidth
        self.lastDirectory = lastDirectory
        self.openFiles = openFiles
        self.autoSave = autoSave
        self.fontSize = fontSize
        self.fontFamily = fontFamily
        self.tabWidth = tabWidth
        self.useSpaces = useSpaces
        self.showLineNumbers = showLineNumbers
        self.showRightMargin = showRightMargin
        self.rightMarginPosition = rightMarginPosition
        self.wordWrap = wordWrap
        self.highlightCurrentLine = highlightCurrentLine
        self.minimapEnabled = minimapEnabled
        self.renderWhitespace = renderWhitespace
        self.stickyScroll = stickyScroll
        self.bracketPairColorization = bracketPairColorization
        self.indentGuides = indentGuides
        self.fontLigatures = fontLigatures
        self.folding = folding
        self.scrollBeyondLastLine = scrollBeyondLastLine
        self.smoothScrolling = smoothScrolling
        self.editorCursorStyle = editorCursorStyle
        self.editorCursorBlinking = editorCursorBlinking
        self.terminalScrollback = terminalScrollback
        self.terminalCursorShape = terminalCursorShape
        self.terminalCursorBlink = terminalCursorBlink
        self.terminalBell = terminalBell
        self.terminalFontFamily = terminalFontFamily
        self.terminalFontSize = terminalFontSize
        self.terminalCopyOnSelect = terminalCopyOnSelect
        self.terminalScrollOnOutput = terminalScrollOnOutput
        self.terminalAllowHyperlink = terminalAllowHyperlink
        self.terminalBoldIsBright = terminalBoldIsBright
        self.editorLineHeight = editorLineHeight
        self.editorAutoClosingBrackets = editorAutoClosingBrackets
        self.sidebarShowHidden = sidebarShowHidden
        self.colorScheme = colorScheme
        self.commandsOnSave = commandsOnSave
        self.customKeybindings = customKeybindings
        self.keybindingOverrides = keybindingOverrides
        self.fileTypeOverrides = fileTypeOverrides
    }
}

// MARK: - Settings I/O

extension Settings {
    /// Returns the path to `~/Library/Application Support/impulse/settings.json`.
    static func settingsPath() -> URL {
        let appSupport = FileManager.default.urls(
            for: .applicationSupportDirectory, in: .userDomainMask
        ).first!
        let dir = appSupport.appendingPathComponent("impulse", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.appendingPathComponent("settings.json")
    }

    /// Convenience alias used by existing code.
    static var filePath: URL { settingsPath() }

    /// Loads settings from disk, falling back to defaults for any missing or
    /// corrupt data.
    static func load() -> Settings {
        let url = settingsPath()
        let data: Data
        do {
            data = try Data(contentsOf: url)
        } catch {
            // File may not exist on first launch — this is expected.
            return .default
        }
        do {
            return try JSONDecoder().decode(Settings.self, from: data)
        } catch {
            os_log(.error, "Failed to decode settings from '%{public}@': %{public}@",
                   url.path, error.localizedDescription)
            return .default
        }
    }

    /// Persists the current settings to disk as pretty-printed JSON.
    /// Encoding and writing happen on a background queue to avoid blocking the
    /// main thread. File permissions are set to 0600 (owner read/write only).
    func save() {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        let data: Data
        do {
            data = try encoder.encode(self)
        } catch {
            os_log(.error, "Failed to encode settings: %{public}@", error.localizedDescription)
            return
        }
        let url = Settings.settingsPath()
        DispatchQueue.global(qos: .utility).async {
            do {
                try data.write(to: url, options: .atomic)
                // Restrict permissions to owner-only (0600).
                try FileManager.default.setAttributes(
                    [.posixPermissions: 0o600], ofItemAtPath: url.path)
            } catch {
                os_log(.error, "Failed to write settings to '%{public}@': %{public}@",
                       url.path, error.localizedDescription)
            }
        }
    }

    /// Static variant for callers that don't hold an instance.
    static func save(_ settings: Settings) {
        settings.save()
    }
}

// MARK: - File Pattern Matching

extension Settings {
    /// Check whether a file path matches a glob-like pattern.
    ///
    /// Supports `"*"` (match all), `"*.ext"` (extension match), and exact
    /// filename suffix matching.
    static func matchesFilePattern(_ path: String, pattern: String) -> Bool {
        if pattern == "*" { return true }
        if pattern.hasPrefix("*.") {
            let extPattern = String(pattern.dropFirst(2))
            let fileExt = (path as NSString).pathExtension
            return fileExt.caseInsensitiveCompare(extPattern) == .orderedSame
        }
        let filename = (path as NSString).lastPathComponent
        return filename == pattern || path.hasSuffix(pattern)
    }
}
