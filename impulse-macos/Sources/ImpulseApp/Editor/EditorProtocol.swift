import Foundation

// MARK: - Editor Commands (Swift -> Monaco)

/// Commands sent from Swift to the Monaco WebView via evaluateJavaScript.
///
/// JSON encoding uses a tagged union format with a "type" field and snake_case
/// keys, matching the Rust `EditorCommand` serde output from impulse-editor.
enum EditorCommand: Encodable {
    case openFile(filePath: String, content: String, language: String)
    case setTheme(theme: MonacoThemeDefinition)
    case updateSettings(options: EditorOptions)
    case applyDiagnostics(uri: String, markers: [MonacoDiagnostic])
    case resolveCompletions(requestId: UInt64, items: [MonacoCompletionItem])
    case resolveHover(requestId: UInt64, contents: [MonacoHoverContent])
    case resolveDefinition(requestId: UInt64, uri: String?, line: UInt32?, column: UInt32?)
    case goToPosition(line: UInt32, column: UInt32)
    case setReadOnly(readOnly: Bool)
    case applyDiffDecorations(decorations: [DiffDecoration])

    // MARK: Tagged Enum Encoding

    private enum TypeTag: String, Encodable {
        case openFile = "OpenFile"
        case setTheme = "SetTheme"
        case updateSettings = "UpdateSettings"
        case applyDiagnostics = "ApplyDiagnostics"
        case resolveCompletions = "ResolveCompletions"
        case resolveHover = "ResolveHover"
        case resolveDefinition = "ResolveDefinition"
        case goToPosition = "GoToPosition"
        case setReadOnly = "SetReadOnly"
        case applyDiffDecorations = "ApplyDiffDecorations"
    }

    private enum CodingKeys: String, CodingKey {
        case type
        case filePath = "file_path"
        case content
        case language
        case theme
        case options
        case uri
        case markers
        case requestId = "request_id"
        case items
        case contents
        case line
        case column
        case readOnly = "read_only"
        case decorations
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)

        switch self {
        case let .openFile(filePath, content, language):
            try container.encode(TypeTag.openFile, forKey: .type)
            try container.encode(filePath, forKey: .filePath)
            try container.encode(content, forKey: .content)
            try container.encode(language, forKey: .language)

        case let .setTheme(theme):
            try container.encode(TypeTag.setTheme, forKey: .type)
            try container.encode(theme, forKey: .theme)

        case let .updateSettings(options):
            try container.encode(TypeTag.updateSettings, forKey: .type)
            try container.encode(options, forKey: .options)

        case let .applyDiagnostics(uri, markers):
            try container.encode(TypeTag.applyDiagnostics, forKey: .type)
            try container.encode(uri, forKey: .uri)
            try container.encode(markers, forKey: .markers)

        case let .resolveCompletions(requestId, items):
            try container.encode(TypeTag.resolveCompletions, forKey: .type)
            try container.encode(requestId, forKey: .requestId)
            try container.encode(items, forKey: .items)

        case let .resolveHover(requestId, contents):
            try container.encode(TypeTag.resolveHover, forKey: .type)
            try container.encode(requestId, forKey: .requestId)
            try container.encode(contents, forKey: .contents)

        case let .resolveDefinition(requestId, uri, line, column):
            try container.encode(TypeTag.resolveDefinition, forKey: .type)
            try container.encode(requestId, forKey: .requestId)
            try container.encodeIfPresent(uri, forKey: .uri)
            try container.encodeIfPresent(line, forKey: .line)
            try container.encodeIfPresent(column, forKey: .column)

        case let .goToPosition(line, column):
            try container.encode(TypeTag.goToPosition, forKey: .type)
            try container.encode(line, forKey: .line)
            try container.encode(column, forKey: .column)

        case let .setReadOnly(readOnly):
            try container.encode(TypeTag.setReadOnly, forKey: .type)
            try container.encode(readOnly, forKey: .readOnly)

        case let .applyDiffDecorations(decorations):
            try container.encode(TypeTag.applyDiffDecorations, forKey: .type)
            try container.encode(decorations, forKey: .decorations)
        }
    }
}

// MARK: - Editor Events (Monaco -> Swift)

/// Events received from the Monaco WebView via WKScriptMessageHandler.
///
/// JSON decoding expects the same tagged union format with a "type" field
/// and snake_case keys produced by the Monaco JavaScript layer.
enum EditorEvent: Decodable {
    case ready
    case fileOpened
    case contentChanged(content: String, version: UInt32)
    case cursorMoved(line: UInt32, column: UInt32)
    case saveRequested
    case completionRequested(requestId: UInt64, line: UInt32, character: UInt32)
    case hoverRequested(requestId: UInt64, line: UInt32, character: UInt32)
    case definitionRequested(requestId: UInt64, line: UInt32, character: UInt32)
    case openFileRequested(uri: String, line: UInt32, character: UInt32)
    case focusChanged(focused: Bool)

    private enum TypeTag: String, Decodable {
        case ready = "Ready"
        case fileOpened = "FileOpened"
        case contentChanged = "ContentChanged"
        case cursorMoved = "CursorMoved"
        case saveRequested = "SaveRequested"
        case completionRequested = "CompletionRequested"
        case hoverRequested = "HoverRequested"
        case definitionRequested = "DefinitionRequested"
        case openFileRequested = "OpenFileRequested"
        case focusChanged = "FocusChanged"
    }

    private enum CodingKeys: String, CodingKey {
        case type
        case content
        case version
        case line
        case column
        case character
        case requestId = "request_id"
        case uri
        case focused
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let tag = try container.decode(TypeTag.self, forKey: .type)

        switch tag {
        case .ready:
            self = .ready

        case .fileOpened:
            self = .fileOpened

        case .contentChanged:
            let content = try container.decode(String.self, forKey: .content)
            let version = try container.decode(UInt32.self, forKey: .version)
            self = .contentChanged(content: content, version: version)

        case .cursorMoved:
            let line = try container.decode(UInt32.self, forKey: .line)
            let column = try container.decode(UInt32.self, forKey: .column)
            self = .cursorMoved(line: line, column: column)

        case .saveRequested:
            self = .saveRequested

        case .completionRequested:
            let requestId = try container.decode(UInt64.self, forKey: .requestId)
            let line = try container.decode(UInt32.self, forKey: .line)
            let character = try container.decode(UInt32.self, forKey: .character)
            self = .completionRequested(requestId: requestId, line: line, character: character)

        case .hoverRequested:
            let requestId = try container.decode(UInt64.self, forKey: .requestId)
            let line = try container.decode(UInt32.self, forKey: .line)
            let character = try container.decode(UInt32.self, forKey: .character)
            self = .hoverRequested(requestId: requestId, line: line, character: character)

        case .definitionRequested:
            let requestId = try container.decode(UInt64.self, forKey: .requestId)
            let line = try container.decode(UInt32.self, forKey: .line)
            let character = try container.decode(UInt32.self, forKey: .character)
            self = .definitionRequested(requestId: requestId, line: line, character: character)

        case .openFileRequested:
            let uri = try container.decode(String.self, forKey: .uri)
            let line = try container.decode(UInt32.self, forKey: .line)
            let character = try container.decode(UInt32.self, forKey: .character)
            self = .openFileRequested(uri: uri, line: line, character: character)

        case .focusChanged:
            let focused = try container.decode(Bool.self, forKey: .focused)
            self = .focusChanged(focused: focused)
        }
    }
}

// MARK: - Supporting Types

/// Editor configuration options. All fields are optional; only non-nil values
/// are serialized, matching the Rust `EditorOptions` struct with
/// `skip_serializing_if = "Option::is_none"`.
struct EditorOptions: Codable {
    var fontSize: UInt32?
    var fontFamily: String?
    var tabSize: UInt32?
    var insertSpaces: Bool?
    var wordWrap: String?
    var minimapEnabled: Bool?
    var lineNumbers: String?
    var renderWhitespace: String?
    var renderLineHighlight: String?
    var rulers: [UInt32]?
    var stickyScroll: Bool?
    var bracketPairColorization: Bool?
    var indentGuides: Bool?
    var fontLigatures: Bool?
    var folding: Bool?
    var scrollBeyondLastLine: Bool?
    var smoothScrolling: Bool?
    var cursorStyle: String?
    var cursorBlinking: String?
    var lineHeight: UInt32?
    var autoClosingBrackets: String?

    enum CodingKeys: String, CodingKey {
        case fontSize = "font_size"
        case fontFamily = "font_family"
        case tabSize = "tab_size"
        case insertSpaces = "insert_spaces"
        case wordWrap = "word_wrap"
        case minimapEnabled = "minimap_enabled"
        case lineNumbers = "line_numbers"
        case renderWhitespace = "render_whitespace"
        case renderLineHighlight = "render_line_highlight"
        case rulers
        case stickyScroll = "sticky_scroll"
        case bracketPairColorization = "bracket_pair_colorization"
        case indentGuides = "indent_guides"
        case fontLigatures = "font_ligatures"
        case folding
        case scrollBeyondLastLine = "scroll_beyond_last_line"
        case smoothScrolling = "smooth_scrolling"
        case cursorStyle = "cursor_style"
        case cursorBlinking = "cursor_blinking"
        case lineHeight = "line_height"
        case autoClosingBrackets = "auto_closing_brackets"
    }
}

struct MonacoDiagnostic: Codable {
    var severity: UInt8
    var startLine: UInt32
    var startColumn: UInt32
    var endLine: UInt32
    var endColumn: UInt32
    var message: String
    var source: String?

    enum CodingKeys: String, CodingKey {
        case severity
        case startLine = "start_line"
        case startColumn = "start_column"
        case endLine = "end_line"
        case endColumn = "end_column"
        case message
        case source
    }
}

struct MonacoCompletionItem: Codable {
    var label: String
    var kind: UInt32
    var detail: String?
    var insertText: String
    var insertTextRules: UInt32?
    var range: MonacoRange?
    var additionalTextEdits: [MonacoTextEdit]

    enum CodingKeys: String, CodingKey {
        case label
        case kind
        case detail
        case insertText = "insert_text"
        case insertTextRules = "insert_text_rules"
        case range
        case additionalTextEdits = "additional_text_edits"
    }

    init(
        label: String,
        kind: UInt32,
        detail: String? = nil,
        insertText: String,
        insertTextRules: UInt32? = nil,
        range: MonacoRange? = nil,
        additionalTextEdits: [MonacoTextEdit] = []
    ) {
        self.label = label
        self.kind = kind
        self.detail = detail
        self.insertText = insertText
        self.insertTextRules = insertTextRules
        self.range = range
        self.additionalTextEdits = additionalTextEdits
    }
}

struct MonacoHoverContent: Codable {
    var value: String
    var isTrusted: Bool

    enum CodingKeys: String, CodingKey {
        case value
        case isTrusted = "is_trusted"
    }

    init(value: String, isTrusted: Bool = false) {
        self.value = value
        self.isTrusted = isTrusted
    }
}

struct MonacoRange: Codable {
    var startLine: UInt32
    var startColumn: UInt32
    var endLine: UInt32
    var endColumn: UInt32

    enum CodingKeys: String, CodingKey {
        case startLine = "start_line"
        case startColumn = "start_column"
        case endLine = "end_line"
        case endColumn = "end_column"
    }
}

struct MonacoTextEdit: Codable {
    var range: MonacoRange
    var text: String
}

struct DiffDecoration: Codable {
    /// 1-based line number.
    var line: UInt32
    /// One of "added", "modified", or "deleted".
    var status: String
}

// MARK: - Theme Types

struct MonacoThemeDefinition: Codable {
    var base: String
    var inherit: Bool
    var rules: [MonacoTokenRule]
    var colors: MonacoThemeColors
}

struct MonacoTokenRule: Codable {
    var token: String
    var foreground: String?
    var fontStyle: String?

    enum CodingKeys: String, CodingKey {
        case token
        case foreground
        case fontStyle = "font_style"
    }
}

struct MonacoThemeColors: Codable {
    var editorBackground: String
    var editorForeground: String
    var editorLineHighlightBackground: String
    var editorSelectionBackground: String
    var editorCursorForeground: String
    var editorLineNumberForeground: String
    var editorLineNumberActiveForeground: String
    var editorWidgetBackground: String
    var editorSuggestWidgetBackground: String
    var editorSuggestWidgetSelectedBackground: String
    var editorHoverWidgetBackground: String
    var editorGutterBackground: String
    var minimapBackground: String
    var scrollbarSliderBackground: String
    var scrollbarSliderHoverBackground: String
    var diffAddedColor: String
    var diffModifiedColor: String
    var diffDeletedColor: String

    enum CodingKeys: String, CodingKey {
        case editorBackground = "editor.background"
        case editorForeground = "editor.foreground"
        case editorLineHighlightBackground = "editor.lineHighlightBackground"
        case editorSelectionBackground = "editor.selectionBackground"
        case editorCursorForeground = "editorCursor.foreground"
        case editorLineNumberForeground = "editorLineNumber.foreground"
        case editorLineNumberActiveForeground = "editorLineNumber.activeForeground"
        case editorWidgetBackground = "editorWidget.background"
        case editorSuggestWidgetBackground = "editorSuggestWidget.background"
        case editorSuggestWidgetSelectedBackground = "editorSuggestWidget.selectedBackground"
        case editorHoverWidgetBackground = "editorHoverWidget.background"
        case editorGutterBackground = "editorGutter.background"
        case minimapBackground = "minimap.background"
        case scrollbarSliderBackground = "scrollbarSlider.background"
        case scrollbarSliderHoverBackground = "scrollbarSlider.hoverBackground"
        case diffAddedColor = "impulse.diffAddedColor"
        case diffModifiedColor = "impulse.diffModifiedColor"
        case diffDeletedColor = "impulse.diffDeletedColor"
    }
}

// MARK: - Completion Kind Mapping

/// Maps LSP completion kind strings to Monaco CompletionItemKind values.
func lspCompletionKindToMonaco(_ kind: String) -> UInt32 {
    switch kind.lowercased() {
    case "method":       return 0
    case "function":     return 1
    case "constructor":  return 2
    case "field":        return 3
    case "variable":     return 4
    case "class":        return 5
    case "struct":       return 6
    case "interface":    return 7
    case "module":       return 8
    case "property":     return 9
    case "event":        return 10
    case "operator":     return 11
    case "unit":         return 12
    case "value":        return 13
    case "constant":     return 14
    case "enum":         return 15
    case "enum-member",
         "enummember":   return 16
    case "keyword":      return 17
    case "snippet":      return 27
    case "text":         return 18
    case "color":        return 19
    case "file":         return 20
    case "reference":    return 21
    case "folder":       return 23
    case "type-parameter",
         "typeparameter": return 24
    default:             return 18 // Text
    }
}

/// Maps LSP diagnostic severity to Monaco MarkerSeverity values.
func diagnosticSeverityToMonaco(_ severity: UInt8) -> UInt8 {
    switch severity {
    case 1: return 8 // Error
    case 2: return 4 // Warning
    case 3: return 2 // Information
    case 4: return 1 // Hint
    default: return 2 // Information
    }
}
