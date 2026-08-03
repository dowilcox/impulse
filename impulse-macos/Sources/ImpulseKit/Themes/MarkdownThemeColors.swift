import Foundation

// Ported from `MarkdownThemeColors` and `theme_to_markdown_colors` in
// impulse-editor/src/markdown.rs. The Codable keys match the Rust snake_case
// JSON contract exactly.

/// Theme colors for the rendered markdown preview.
public struct MarkdownThemeColors: Codable, Equatable {
  public var bg: String
  public var fg: String
  public var heading: String
  public var link: String
  public var codeBg: String
  public var border: String
  public var blockquoteFg: String
  /// highlight.js token colors
  public var hljsKeyword: String
  public var hljsString: String
  public var hljsNumber: String
  public var hljsComment: String
  public var hljsFunction: String
  public var hljsType: String
  public var fontFamily: String
  public var codeFontFamily: String

  enum CodingKeys: String, CodingKey {
    case bg, fg, heading, link, border
    case codeBg = "code_bg"
    case blockquoteFg = "blockquote_fg"
    case hljsKeyword = "hljs_keyword"
    case hljsString = "hljs_string"
    case hljsNumber = "hljs_number"
    case hljsComment = "hljs_comment"
    case hljsFunction = "hljs_function"
    case hljsType = "hljs_type"
    case fontFamily = "font_family"
    case codeFontFamily = "code_font_family"
  }

  public init(
    bg: String, fg: String, heading: String, link: String, codeBg: String,
    border: String, blockquoteFg: String, hljsKeyword: String, hljsString: String,
    hljsNumber: String, hljsComment: String, hljsFunction: String, hljsType: String,
    fontFamily: String, codeFontFamily: String
  ) {
    self.bg = bg
    self.fg = fg
    self.heading = heading
    self.link = link
    self.codeBg = codeBg
    self.border = border
    self.blockquoteFg = blockquoteFg
    self.hljsKeyword = hljsKeyword
    self.hljsString = hljsString
    self.hljsNumber = hljsNumber
    self.hljsComment = hljsComment
    self.hljsFunction = hljsFunction
    self.hljsType = hljsType
    self.fontFamily = fontFamily
    self.codeFontFamily = codeFontFamily
  }

  /// The sanitize-fallback palette from the Rust markdown renderer.
  public static var fallback: MarkdownThemeColors {
    MarkdownThemeColors(
      bg: "#1a1b26",
      fg: "#c0caf5",
      heading: "#7dcfff",
      link: "#7aa2f7",
      codeBg: "#16161e",
      border: "#292e42",
      blockquoteFg: "#565f89",
      hljsKeyword: "#bb9af7",
      hljsString: "#9ece6a",
      hljsNumber: "#ff9e64",
      hljsComment: "#565f89",
      hljsFunction: "#7aa2f7",
      hljsType: "#e0af68",
      fontFamily: "Inter, system-ui, sans-serif",
      codeFontFamily: "'JetBrains Mono', monospace"
    )
  }
}

/// Convert a `ResolvedTheme` into `MarkdownThemeColors` for the markdown
/// preview. Ported from `theme_to_markdown_colors`.
public func themeToMarkdownColors(_ theme: ResolvedTheme) -> MarkdownThemeColors {
  MarkdownThemeColors(
    bg: theme.bg,
    fg: theme.fg,
    heading: theme.cyan,
    link: theme.syntaxLink,
    codeBg: theme.bgDark,
    border: theme.bgHighlight,
    blockquoteFg: theme.fgComment,
    hljsKeyword: theme.syntaxKeyword,
    hljsString: theme.syntaxString,
    hljsNumber: theme.syntaxNumber,
    hljsComment: theme.syntaxComment,
    hljsFunction: theme.syntaxFunction,
    hljsType: theme.syntaxType,
    fontFamily: "Inter, system-ui, sans-serif",
    codeFontFamily: "'JetBrains Mono', monospace"
  )
}
