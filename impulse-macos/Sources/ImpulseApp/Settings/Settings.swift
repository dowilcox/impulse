import Foundation
import os.log

// MARK: - Sub-Types

struct SettingsLoadWarning: Equatable {
    let settingsPath: URL
    let backupPath: URL?
    let message: String
}

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
    var confirmCloseWarnings: Bool
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
    var terminalAttentionOnBell: Bool
    var terminalFontFamily: String
    var terminalFontSize: Int
    var terminalCopyOnSelect: Bool
    var terminalScrollOnOutput: Bool
    var terminalAllowHyperlink: Bool
    var terminalAllowNotifications: Bool
    var terminalAttentionOnLongCommand: Bool
    var terminalLongCommandSeconds: Int
    var terminalBoldIsBright: Bool
    var terminalAllowOsc52Write: Bool
    var terminalAllowOsc52Read: Bool

    // -- Editor (additional) --
    var editorLineHeight: Int
    var editorAutoClosingBrackets: String
    var editorCursorSurroundingLines: Int
    var editorSelectionHighlight: Bool
    var editorOccurrencesHighlight: Bool
    var editorWordBasedSuggestions: String

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

    // -- Updates --
    var checkForUpdates: Bool

    // MARK: CodingKeys (snake_case to match Linux JSON)

    enum CodingKeys: String, CodingKey {
        case windowWidth = "window_width"
        case windowHeight = "window_height"
        case sidebarVisible = "sidebar_visible"
        case sidebarWidth = "sidebar_width"
        case confirmCloseWarnings = "confirm_close_warnings"
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
        case terminalAttentionOnBell = "terminal_attention_on_bell"
        case terminalFontFamily = "terminal_font_family"
        case terminalFontSize = "terminal_font_size"
        case terminalCopyOnSelect = "terminal_copy_on_select"
        case terminalScrollOnOutput = "terminal_scroll_on_output"
        case terminalAllowHyperlink = "terminal_allow_hyperlink"
        case terminalAllowNotifications = "terminal_allow_notifications"
        case terminalAttentionOnLongCommand = "terminal_attention_on_long_command"
        case terminalLongCommandSeconds = "terminal_long_command_seconds"
        case terminalBoldIsBright = "terminal_bold_is_bright"
        case terminalAllowOsc52Write = "terminal_allow_osc52_write"
        case terminalAllowOsc52Read = "terminal_allow_osc52_read"
        case editorLineHeight = "editor_line_height"
        case editorAutoClosingBrackets = "editor_auto_closing_brackets"
        case editorCursorSurroundingLines = "editor_cursor_surrounding_lines"
        case editorSelectionHighlight = "editor_selection_highlight"
        case editorOccurrencesHighlight = "editor_occurrences_highlight"
        case editorWordBasedSuggestions = "editor_word_based_suggestions"
        case sidebarShowHidden = "sidebar_show_hidden"
        case colorScheme = "color_scheme"
        case commandsOnSave = "commands_on_save"
        case customKeybindings = "custom_keybindings"
        case keybindingOverrides = "keybinding_overrides"
        case fileTypeOverrides = "file_type_overrides"
        case checkForUpdates = "check_for_updates"
    }

    // MARK: Defaults

    static var `default`: Settings {
        Settings(
            windowWidth: 1200,
            windowHeight: 800,
            sidebarVisible: false,
            sidebarWidth: 250,
            confirmCloseWarnings: true,
            lastDirectory: "",
            openFiles: [],
            autoSave: false,
            fontSize: 14,
            fontFamily: "JetBrains Mono",
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
            terminalBell: true,
            terminalAttentionOnBell: true,
            terminalFontFamily: "JetBrains Mono",
            terminalFontSize: 14,
            terminalCopyOnSelect: true,
            terminalScrollOnOutput: true,
            terminalAllowHyperlink: true,
            terminalAllowNotifications: true,
            terminalAttentionOnLongCommand: true,
            terminalLongCommandSeconds: 30,
            terminalBoldIsBright: true,
            terminalAllowOsc52Write: true,
            terminalAllowOsc52Read: false,
            editorLineHeight: 0,
            editorAutoClosingBrackets: "languageDefined",
            editorCursorSurroundingLines: 3,
            editorSelectionHighlight: true,
            editorOccurrencesHighlight: true,
            editorWordBasedSuggestions: "matchingDocuments",
            sidebarShowHidden: false,
            colorScheme: "nord",
            commandsOnSave: [],
            customKeybindings: [],
            keybindingOverrides: [:],
            fileTypeOverrides: [],
            checkForUpdates: true
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
        confirmCloseWarnings = (try? c.decode(Bool.self, forKey: .confirmCloseWarnings)) ?? d.confirmCloseWarnings
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
        terminalAttentionOnBell = (try? c.decode(Bool.self, forKey: .terminalAttentionOnBell)) ?? d.terminalAttentionOnBell
        terminalFontFamily = (try? c.decode(String.self, forKey: .terminalFontFamily)) ?? d.terminalFontFamily
        terminalFontSize = (try? c.decode(Int.self, forKey: .terminalFontSize)) ?? d.terminalFontSize
        terminalCopyOnSelect = (try? c.decode(Bool.self, forKey: .terminalCopyOnSelect)) ?? d.terminalCopyOnSelect
        terminalScrollOnOutput = (try? c.decode(Bool.self, forKey: .terminalScrollOnOutput)) ?? d.terminalScrollOnOutput
        terminalAllowHyperlink = (try? c.decode(Bool.self, forKey: .terminalAllowHyperlink)) ?? d.terminalAllowHyperlink
        terminalAllowNotifications = (try? c.decode(Bool.self, forKey: .terminalAllowNotifications)) ?? d.terminalAllowNotifications
        terminalAttentionOnLongCommand = (try? c.decode(Bool.self, forKey: .terminalAttentionOnLongCommand)) ?? d.terminalAttentionOnLongCommand
        terminalLongCommandSeconds = (try? c.decode(Int.self, forKey: .terminalLongCommandSeconds)) ?? d.terminalLongCommandSeconds
        terminalBoldIsBright = (try? c.decode(Bool.self, forKey: .terminalBoldIsBright)) ?? d.terminalBoldIsBright
        terminalAllowOsc52Write = (try? c.decode(Bool.self, forKey: .terminalAllowOsc52Write)) ?? d.terminalAllowOsc52Write
        terminalAllowOsc52Read = (try? c.decode(Bool.self, forKey: .terminalAllowOsc52Read)) ?? d.terminalAllowOsc52Read
        editorLineHeight = (try? c.decode(Int.self, forKey: .editorLineHeight)) ?? d.editorLineHeight
        editorAutoClosingBrackets = (try? c.decode(String.self, forKey: .editorAutoClosingBrackets)) ?? d.editorAutoClosingBrackets
        editorCursorSurroundingLines = (try? c.decode(Int.self, forKey: .editorCursorSurroundingLines)) ?? d.editorCursorSurroundingLines
        editorSelectionHighlight = (try? c.decode(Bool.self, forKey: .editorSelectionHighlight)) ?? d.editorSelectionHighlight
        editorOccurrencesHighlight = (try? c.decode(Bool.self, forKey: .editorOccurrencesHighlight)) ?? d.editorOccurrencesHighlight
        editorWordBasedSuggestions = (try? c.decode(String.self, forKey: .editorWordBasedSuggestions)) ?? d.editorWordBasedSuggestions
        sidebarShowHidden = (try? c.decode(Bool.self, forKey: .sidebarShowHidden)) ?? d.sidebarShowHidden
        colorScheme = (try? c.decode(String.self, forKey: .colorScheme)) ?? d.colorScheme
        commandsOnSave = (try? c.decode([CommandOnSave].self, forKey: .commandsOnSave)) ?? d.commandsOnSave
        customKeybindings = (try? c.decode([CustomKeybinding].self, forKey: .customKeybindings)) ?? d.customKeybindings
        keybindingOverrides = (try? c.decode([String: String].self, forKey: .keybindingOverrides)) ?? d.keybindingOverrides
        fileTypeOverrides = (try? c.decode([FileTypeOverride].self, forKey: .fileTypeOverrides)) ?? d.fileTypeOverrides
        checkForUpdates = (try? c.decode(Bool.self, forKey: .checkForUpdates)) ?? d.checkForUpdates
    }

    /// Memberwise initializer used by `Settings.default`.
    init(windowWidth: Int, windowHeight: Int, sidebarVisible: Bool, sidebarWidth: Int,
         confirmCloseWarnings: Bool,
         lastDirectory: String, openFiles: [String], autoSave: Bool, fontSize: Int,
         fontFamily: String, tabWidth: Int, useSpaces: Bool, showLineNumbers: Bool,
         showRightMargin: Bool, rightMarginPosition: Int, wordWrap: Bool,
         highlightCurrentLine: Bool, minimapEnabled: Bool, renderWhitespace: String,
         stickyScroll: Bool, bracketPairColorization: Bool, indentGuides: Bool,
         fontLigatures: Bool, folding: Bool, scrollBeyondLastLine: Bool,
         smoothScrolling: Bool, editorCursorStyle: String, editorCursorBlinking: String,
         terminalScrollback: Int, terminalCursorShape: String, terminalCursorBlink: Bool,
         terminalBell: Bool, terminalAttentionOnBell: Bool,
         terminalFontFamily: String, terminalFontSize: Int,
         terminalCopyOnSelect: Bool, terminalScrollOnOutput: Bool,
         terminalAllowHyperlink: Bool, terminalAllowNotifications: Bool,
         terminalAttentionOnLongCommand: Bool, terminalLongCommandSeconds: Int,
         terminalBoldIsBright: Bool,
         terminalAllowOsc52Write: Bool, terminalAllowOsc52Read: Bool,
         editorLineHeight: Int, editorAutoClosingBrackets: String,
         editorCursorSurroundingLines: Int, editorSelectionHighlight: Bool,
         editorOccurrencesHighlight: Bool, editorWordBasedSuggestions: String,
         sidebarShowHidden: Bool, colorScheme: String,
         commandsOnSave: [CommandOnSave], customKeybindings: [CustomKeybinding],
         keybindingOverrides: [String: String], fileTypeOverrides: [FileTypeOverride],
         checkForUpdates: Bool) {
        self.windowWidth = windowWidth
        self.windowHeight = windowHeight
        self.sidebarVisible = sidebarVisible
        self.sidebarWidth = sidebarWidth
        self.confirmCloseWarnings = confirmCloseWarnings
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
        self.terminalAttentionOnBell = terminalAttentionOnBell
        self.terminalFontFamily = terminalFontFamily
        self.terminalFontSize = terminalFontSize
        self.terminalCopyOnSelect = terminalCopyOnSelect
        self.terminalScrollOnOutput = terminalScrollOnOutput
        self.terminalAllowHyperlink = terminalAllowHyperlink
        self.terminalAllowNotifications = terminalAllowNotifications
        self.terminalAttentionOnLongCommand = terminalAttentionOnLongCommand
        self.terminalLongCommandSeconds = terminalLongCommandSeconds
        self.terminalBoldIsBright = terminalBoldIsBright
        self.terminalAllowOsc52Write = terminalAllowOsc52Write
        self.terminalAllowOsc52Read = terminalAllowOsc52Read
        self.editorLineHeight = editorLineHeight
        self.editorAutoClosingBrackets = editorAutoClosingBrackets
        self.editorCursorSurroundingLines = editorCursorSurroundingLines
        self.editorSelectionHighlight = editorSelectionHighlight
        self.editorOccurrencesHighlight = editorOccurrencesHighlight
        self.editorWordBasedSuggestions = editorWordBasedSuggestions
        self.sidebarShowHidden = sidebarShowHidden
        self.colorScheme = colorScheme
        self.commandsOnSave = commandsOnSave
        self.customKeybindings = customKeybindings
        self.keybindingOverrides = keybindingOverrides
        self.fileTypeOverrides = fileTypeOverrides
        self.checkForUpdates = checkForUpdates
    }
}

// MARK: - Settings I/O

extension Settings {
    private static var saveBlockedByLoadError = false
    static private(set) var loadWarning: SettingsLoadWarning?

    /// Returns the path to `~/Library/Application Support/impulse/settings.json`.
    static func settingsPath() -> URL {
        guard let appSupport = FileManager.default.urls(
            for: .applicationSupportDirectory, in: .userDomainMask
        ).first else {
            // Fallback to ~/Library/Application Support if the system API
            // returns an empty array (should never happen on macOS).
            let home = FileManager.default.homeDirectoryForCurrentUser
            return home.appendingPathComponent("Library/Application Support/impulse/settings.json")
        }
        let dir = appSupport.appendingPathComponent("impulse", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        // Set restrictive permissions on settings directory
        try? FileManager.default.setAttributes(
            [.posixPermissions: 0o700],
            ofItemAtPath: dir.path
        )
        return dir.appendingPathComponent("settings.json")
    }

    /// Convenience alias used by existing code.
    static var filePath: URL { settingsPath() }

    /// Clamp numeric settings to safe ranges to prevent crashes or resource
    /// exhaustion from malformed settings files.
    mutating func validate() {
        fontSize = max(6, min(72, fontSize))
        tabWidth = max(1, min(16, tabWidth))
        terminalFontSize = max(6, min(72, terminalFontSize))
        terminalScrollback = max(100, min(1_000_000, terminalScrollback))
        terminalLongCommandSeconds = max(1, min(86_400, terminalLongCommandSeconds))
        sidebarWidth = max(100, min(1000, sidebarWidth))
        rightMarginPosition = max(1, min(500, rightMarginPosition))
        editorLineHeight = max(0, min(100, editorLineHeight))
        windowWidth = max(400, min(10000, windowWidth))
        windowHeight = max(300, min(10000, windowHeight))
    }

    /// Migrates the default font from old platform defaults ("monospace",
    /// "SF Mono") to the bundled "JetBrains Mono". Only changes settings that
    /// still match an old default — user-customized values are left alone.
    mutating func migrateDefaultFont() {
        let oldDefaults = ["monospace", "SF Mono", ""]
        var changed = false
        if oldDefaults.contains(fontFamily) {
            fontFamily = "JetBrains Mono"
            changed = true
        }
        if oldDefaults.contains(terminalFontFamily) {
            terminalFontFamily = "JetBrains Mono"
            changed = true
        }
        if changed {
            save()
        }
    }

    /// Migrates `format_on_save` entries from `FileTypeOverride` into
    /// `CommandOnSave` entries with `reloadFile: true`, then clears the
    /// originals. Matches the Linux `migrate_format_on_save` behavior.
    mutating func migrateFormatOnSave() {
        var migrated = false
        for i in fileTypeOverrides.indices {
            if let fmt = fileTypeOverrides[i].formatOnSave, !fmt.command.isEmpty {
                commandsOnSave.append(CommandOnSave(
                    name: "Format (\(fileTypeOverrides[i].pattern))",
                    command: fmt.command,
                    args: fmt.args,
                    filePattern: fileTypeOverrides[i].pattern,
                    reloadFile: true
                ))
                fileTypeOverrides[i].formatOnSave = nil
                migrated = true
            }
        }
        if migrated {
            save()
        }
    }

    /// Loads settings from disk, falling back to defaults for any missing or
    /// corrupt data.
    static func load() -> Settings {
        let url = settingsPath()
        let data: Data
        do {
            data = try Data(contentsOf: url)
        } catch {
            // A missing file is expected on first launch; existing unreadable files are not.
            if FileManager.default.fileExists(atPath: url.path) {
                loadWarning = SettingsLoadWarning(
                    settingsPath: url,
                    backupPath: nil,
                    message: error.localizedDescription
                )
                saveBlockedByLoadError = true
                os_log(.error, "Failed to read settings from '%{public}@': %{public}@",
                       url.path, error.localizedDescription)
            } else {
                loadWarning = nil
                saveBlockedByLoadError = false
            }
            return .default
        }
        do {
            var settings = try JSONDecoder().decode(Settings.self, from: data)
            loadWarning = nil
            saveBlockedByLoadError = false
            settings.migrateFormatOnSave()
            settings.migrateDefaultFont()
            settings.validate()
            return settings
        } catch {
            let backupURL = backupInvalidSettingsFile(url: url, data: data)
            if let backupURL {
                os_log(.error,
                       "Backed up invalid settings file to '%{public}@'",
                       backupURL.path)
            }
            loadWarning = SettingsLoadWarning(
                settingsPath: url,
                backupPath: backupURL,
                message: error.localizedDescription
            )
            saveBlockedByLoadError = true
            os_log(.error, "Failed to decode settings from '%{public}@': %{public}@",
                   url.path, error.localizedDescription)
            return .default
        }
    }

    /// Persists the current settings to disk as pretty-printed JSON.
    /// Encoding and writing happen on a background queue to avoid blocking the
    /// main thread. File permissions are set to 0600 (owner read/write only).
    func save() {
        if Settings.saveBlockedByLoadError {
            let path = Settings.loadWarning?.settingsPath.path ?? Settings.settingsPath().path
            let message = Settings.loadWarning?.message ?? "settings load failed"
            os_log(.error,
                   "Skipping settings save to preserve invalid settings file '%{public}@': %{public}@",
                   path, message)
            return
        }

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

    private static func backupInvalidSettingsFile(url: URL, data: Data) -> URL? {
        let directory = url.deletingLastPathComponent()
        let stem = url.deletingPathExtension().lastPathComponent
        let ext = url.pathExtension.isEmpty ? "json" : url.pathExtension
        let timestamp = Int(Date().timeIntervalSince1970)

        for attempt in 0..<100 {
            let suffix = attempt == 0 ? "" : "-\(attempt)"
            let backup = directory.appendingPathComponent(
                "\(stem).invalid-\(timestamp)\(suffix).\(ext)"
            )
            if FileManager.default.fileExists(atPath: backup.path) {
                continue
            }
            do {
                try data.write(to: backup, options: .withoutOverwriting)
                try FileManager.default.setAttributes(
                    [.posixPermissions: 0o600],
                    ofItemAtPath: backup.path
                )
                return backup
            } catch CocoaError.fileWriteFileExists {
                continue
            } catch {
                os_log(.error,
                       "Failed to back up invalid settings file to '%{public}@': %{public}@",
                       backup.path, error.localizedDescription)
                return nil
            }
        }

        os_log(.error, "Failed to choose a unique invalid settings backup path")
        return nil
    }
}

// MARK: - Terminal Settings Factory

extension Settings {
    /// Constructs a `TerminalSettings` value from the current settings.
    func terminalSettings(directory: String? = nil) -> TerminalSettings {
        return TerminalSettings(
            terminalFontSize: terminalFontSize,
            terminalFontFamily: terminalFontFamily,
            terminalCursorShape: terminalCursorShape,
            terminalCursorBlink: terminalCursorBlink,
            terminalScrollback: terminalScrollback,
            lastDirectory: directory ?? lastDirectory,
            terminalCopyOnSelect: terminalCopyOnSelect,
            terminalBell: terminalBell,
            terminalAttentionOnBell: terminalAttentionOnBell,
            terminalScrollOnOutput: terminalScrollOnOutput,
            terminalAllowHyperlink: terminalAllowHyperlink,
            terminalAllowNotifications: terminalAllowNotifications,
            terminalAttentionOnLongCommand: terminalAttentionOnLongCommand,
            terminalLongCommandSeconds: terminalLongCommandSeconds,
            terminalBoldIsBright: terminalBoldIsBright,
            terminalAllowOsc52Write: terminalAllowOsc52Write,
            terminalAllowOsc52Read: terminalAllowOsc52Read,
            keybindingOverrides: keybindingOverrides
        )
    }
}

// MARK: - File Pattern Matching

extension Settings {
    /// Check whether a file path matches a glob-like pattern.
    ///
    /// Delegates to the shared Rust implementation in impulse-core for
    /// consistent behaviour across platforms.
    static func matchesFilePattern(_ path: String, pattern: String) -> Bool {
        return ImpulseCore.matchesFilePattern(path: path, pattern: pattern)
    }
}
