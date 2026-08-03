import Foundation

// Ported from `MonacoThemeDefinition`, `MonacoTokenRule`, `MonacoThemeColors`,
// and `theme_to_monaco` in impulse-editor/src/protocol.rs. The serialized JSON
// is structurally identical to Rust's serde output (rule order preserved,
// `foreground`/`font_style` omitted when nil).

public struct MonacoThemeDefinition: Codable, Equatable {
  public var base: String
  public var inherit: Bool
  public var rules: [MonacoTokenRule]
  public var colors: MonacoThemeColors
}

public struct MonacoTokenRule: Codable, Equatable {
  public var token: String
  public var foreground: String?
  public var fontStyle: String?

  enum CodingKeys: String, CodingKey {
    case token, foreground
    case fontStyle = "font_style"
  }

  public init(token: String, foreground: String? = nil, fontStyle: String? = nil) {
    self.token = token
    self.foreground = foreground
    self.fontStyle = fontStyle
  }
}

public struct MonacoThemeColors: Codable, Equatable {
  public var editorBackground: String
  public var editorForeground: String
  public var editorLineHighlightBackground: String
  public var editorSelectionBackground: String
  public var editorCursorForeground: String
  public var editorLineNumberForeground: String
  public var editorLineNumberActiveForeground: String
  public var editorWidgetBackground: String
  public var editorSuggestWidgetBackground: String
  public var editorSuggestWidgetSelectedBackground: String
  public var editorHoverWidgetBackground: String
  public var editorGutterBackground: String
  public var minimapBackground: String
  public var scrollbarSliderBackground: String
  public var scrollbarSliderHoverBackground: String
  public var scrollbarSliderActiveBackground: String
  public var diffAddedColor: String
  public var diffModifiedColor: String
  public var diffDeletedColor: String

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
    case scrollbarSliderActiveBackground = "scrollbarSlider.activeBackground"
    case diffAddedColor = "impulse.diffAddedColor"
    case diffModifiedColor = "impulse.diffModifiedColor"
    case diffDeletedColor = "impulse.diffDeletedColor"
  }
}

/// Convert a `ResolvedTheme` into a `MonacoThemeDefinition` ready for the
/// Monaco editor WebView. Ported from `theme_to_monaco`.
public func themeToMonaco(_ theme: ResolvedTheme) -> MonacoThemeDefinition {
  func strip(_ c: String) -> String {
    String(c.drop(while: { $0 == "#" }))
  }

  return MonacoThemeDefinition(
    base: theme.isLight ? "vs" : "vs-dark",
    inherit: true,
    rules: [
      // Comments (italic)
      MonacoTokenRule(
        token: "comment", foreground: strip(theme.syntaxComment), fontStyle: "italic"),
      MonacoTokenRule(
        token: "comment.doc", foreground: strip(theme.syntaxComment), fontStyle: "italic"),
      // Keywords
      MonacoTokenRule(token: "keyword", foreground: strip(theme.syntaxKeyword)),
      MonacoTokenRule(token: "keyword.control", foreground: strip(theme.syntaxKeyword)),
      MonacoTokenRule(token: "keyword.declaration", foreground: strip(theme.syntaxKeyword)),
      MonacoTokenRule(token: "keyword.type", foreground: strip(theme.syntaxKeyword)),
      MonacoTokenRule(token: "keyword.other", foreground: strip(theme.syntaxKeyword)),
      MonacoTokenRule(token: "keyword.flow", foreground: strip(theme.syntaxKeyword)),
      MonacoTokenRule(token: "keyword.block", foreground: strip(theme.syntaxKeyword)),
      MonacoTokenRule(token: "keyword.try", foreground: strip(theme.syntaxKeyword)),
      MonacoTokenRule(token: "keyword.catch", foreground: strip(theme.syntaxKeyword)),
      MonacoTokenRule(token: "keyword.choice", foreground: strip(theme.syntaxKeyword)),
      MonacoTokenRule(token: "keyword.modifier", foreground: strip(theme.syntaxKeyword)),
      // Constants & numbers
      MonacoTokenRule(token: "keyword.constant", foreground: strip(theme.syntaxConstant)),
      MonacoTokenRule(token: "number", foreground: strip(theme.syntaxNumber)),
      MonacoTokenRule(token: "number.hex", foreground: strip(theme.syntaxNumber)),
      MonacoTokenRule(token: "number.float", foreground: strip(theme.syntaxNumber)),
      MonacoTokenRule(token: "number.binary", foreground: strip(theme.syntaxNumber)),
      MonacoTokenRule(token: "number.octal", foreground: strip(theme.syntaxNumber)),
      MonacoTokenRule(token: "constant", foreground: strip(theme.syntaxConstant)),
      MonacoTokenRule(token: "string.escape", foreground: strip(theme.syntaxEscape)),
      // Strings
      MonacoTokenRule(token: "string", foreground: strip(theme.syntaxString)),
      MonacoTokenRule(token: "string.heredoc", foreground: strip(theme.syntaxString)),
      MonacoTokenRule(token: "string.raw", foreground: strip(theme.syntaxString)),
      MonacoTokenRule(token: "attribute.value", foreground: strip(theme.syntaxString)),
      // Operators, special strings, predefined
      MonacoTokenRule(token: "string.key", foreground: strip(theme.syntaxOperator)),
      MonacoTokenRule(token: "string.link", foreground: strip(theme.syntaxLink)),
      MonacoTokenRule(token: "operator", foreground: strip(theme.syntaxOperator)),
      MonacoTokenRule(token: "keyword.operator", foreground: strip(theme.syntaxOperator)),
      MonacoTokenRule(token: "variable.predefined", foreground: strip(theme.syntaxOperator)),
      MonacoTokenRule(token: "predefined", foreground: strip(theme.syntaxOperator)),
      // Types, classes, annotations
      MonacoTokenRule(token: "type", foreground: strip(theme.syntaxType)),
      MonacoTokenRule(token: "type.identifier", foreground: strip(theme.syntaxType)),
      MonacoTokenRule(token: "class", foreground: strip(theme.syntaxType)),
      MonacoTokenRule(token: "annotation", foreground: strip(theme.syntaxAttribute)),
      MonacoTokenRule(token: "namespace", foreground: strip(theme.syntaxType)),
      MonacoTokenRule(token: "constructor", foreground: strip(theme.syntaxType)),
      MonacoTokenRule(token: "attribute.name", foreground: strip(theme.syntaxAttribute)),
      // Functions
      MonacoTokenRule(token: "function", foreground: strip(theme.syntaxFunction)),
      MonacoTokenRule(token: "function.declaration", foreground: strip(theme.syntaxFunction)),
      MonacoTokenRule(token: "function.call", foreground: strip(theme.syntaxFunction)),
      MonacoTokenRule(token: "predefined.function", foreground: strip(theme.syntaxFunction)),
      // Tags, invalid, regexp
      MonacoTokenRule(token: "string.escape.invalid", foreground: strip(theme.syntaxTag)),
      MonacoTokenRule(token: "string.invalid", foreground: strip(theme.syntaxTag)),
      MonacoTokenRule(token: "regexp", foreground: strip(theme.syntaxRegexp)),
      MonacoTokenRule(token: "tag", foreground: strip(theme.syntaxTag)),
      MonacoTokenRule(token: "metatag", foreground: strip(theme.syntaxTag)),
      MonacoTokenRule(token: "invalid", foreground: strip(theme.syntaxTag)),
      // Variables, emphasis
      MonacoTokenRule(token: "variable", foreground: strip(theme.syntaxVariable)),
      MonacoTokenRule(
        token: "emphasis", foreground: strip(theme.syntaxVariable), fontStyle: "italic"),
      // Delimiters
      MonacoTokenRule(token: "delimiter", foreground: strip(theme.syntaxDelimiter)),
      // Strong (bold)
      MonacoTokenRule(token: "strong", foreground: strip(theme.orange), fontStyle: "bold"),
    ],
    colors: MonacoThemeColors(
      editorBackground: "#\(strip(theme.bg))",
      editorForeground: "#\(strip(theme.fg))",
      editorLineHighlightBackground: "#\(strip(theme.bgHighlight))",
      editorSelectionBackground: "#\(strip(theme.selection))",
      editorCursorForeground: "#\(strip(theme.cursor))",
      editorLineNumberForeground: "#\(strip(theme.fgComment))",
      editorLineNumberActiveForeground: "#\(strip(theme.fg))",
      editorWidgetBackground: "#\(strip(theme.bgDark))",
      editorSuggestWidgetBackground: "#\(strip(theme.bgDark))",
      editorSuggestWidgetSelectedBackground: "#\(strip(theme.bgHighlight))",
      editorHoverWidgetBackground: "#\(strip(theme.bgDark))",
      editorGutterBackground: "#\(strip(theme.bg))",
      minimapBackground: "#\(strip(theme.bgDark))",
      scrollbarSliderBackground: "#\(strip(theme.fgComment))40",
      scrollbarSliderHoverBackground: "#\(strip(theme.fgComment))80",
      scrollbarSliderActiveBackground: "#\(strip(theme.fgComment))A0",
      diffAddedColor: "#\(strip(theme.gitAdded))",
      diffModifiedColor: "#\(strip(theme.gitModified))",
      diffDeletedColor: "#\(strip(theme.gitDeleted))"
    )
  )
}
