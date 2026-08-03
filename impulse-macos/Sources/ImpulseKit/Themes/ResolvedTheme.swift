import Foundation

// Ported from `ResolvedTheme` in impulse-core/src/theme.rs.
//
// The Codable implementation produces JSON that is field-for-field identical
// to Rust's `theme_to_json` (serde snake_case field names).

/// Fully resolved theme with every field populated. This is the type that
/// frontends, the Monaco converter, and the app consume.
public struct ResolvedTheme {
  public var id: String
  public var name: String
  public var isLight: Bool
  // UI backgrounds
  public var bg: String
  public var bgDark: String
  public var bgHighlight: String
  public var bgSurface: String
  public var border: String
  // UI foregrounds
  public var fg: String
  public var fgMuted: String
  public var fgComment: String
  public var accent: String
  public var selection: String
  public var cursor: String
  // Raw palette hues (for icons, git badges, status bar accents)
  public var red: String
  public var orange: String
  public var yellow: String
  public var green: String
  public var cyan: String
  public var blue: String
  public var magenta: String
  // Git indicators
  public var gitAdded: String
  public var gitModified: String
  public var gitDeleted: String
  public var gitRenamed: String
  public var gitConflict: String
  public var gitIgnored: String
  // Syntax (semantic names)
  public var syntaxKeyword: String
  public var syntaxFunction: String
  public var syntaxType: String
  public var syntaxString: String
  public var syntaxNumber: String
  public var syntaxConstant: String
  public var syntaxComment: String
  public var syntaxOperator: String
  public var syntaxTag: String
  public var syntaxAttribute: String
  public var syntaxVariable: String
  public var syntaxDelimiter: String
  public var syntaxEscape: String
  public var syntaxRegexp: String
  public var syntaxLink: String
  // Terminal
  public var terminalFg: String
  public var terminalBg: String
  public var terminalPalette: [String]
  /// `"flat"` or `"card"` — see `SemanticUI.surfaceStyle`.
  public var surfaceStyle: String
}

extension ResolvedTheme: Codable {
  enum CodingKeys: String, CodingKey {
    case id, name
    case isLight = "is_light"
    case bg
    case bgDark = "bg_dark"
    case bgHighlight = "bg_highlight"
    case bgSurface = "bg_surface"
    case border, fg
    case fgMuted = "fg_muted"
    case fgComment = "fg_comment"
    case accent, selection, cursor
    case red, orange, yellow, green, cyan, blue, magenta
    case gitAdded = "git_added"
    case gitModified = "git_modified"
    case gitDeleted = "git_deleted"
    case gitRenamed = "git_renamed"
    case gitConflict = "git_conflict"
    case gitIgnored = "git_ignored"
    case syntaxKeyword = "syntax_keyword"
    case syntaxFunction = "syntax_function"
    case syntaxType = "syntax_type"
    case syntaxString = "syntax_string"
    case syntaxNumber = "syntax_number"
    case syntaxConstant = "syntax_constant"
    case syntaxComment = "syntax_comment"
    case syntaxOperator = "syntax_operator"
    case syntaxTag = "syntax_tag"
    case syntaxAttribute = "syntax_attribute"
    case syntaxVariable = "syntax_variable"
    case syntaxDelimiter = "syntax_delimiter"
    case syntaxEscape = "syntax_escape"
    case syntaxRegexp = "syntax_regexp"
    case syntaxLink = "syntax_link"
    case terminalFg = "terminal_fg"
    case terminalBg = "terminal_bg"
    case terminalPalette = "terminal_palette"
    case surfaceStyle = "surface_style"
  }

  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: CodingKeys.self)
    id = try c.decode(String.self, forKey: .id)
    name = try c.decode(String.self, forKey: .name)
    isLight = try c.decode(Bool.self, forKey: .isLight)
    bg = try c.decode(String.self, forKey: .bg)
    bgDark = try c.decode(String.self, forKey: .bgDark)
    bgHighlight = try c.decode(String.self, forKey: .bgHighlight)
    bgSurface = try c.decode(String.self, forKey: .bgSurface)
    border = try c.decode(String.self, forKey: .border)
    fg = try c.decode(String.self, forKey: .fg)
    fgMuted = try c.decode(String.self, forKey: .fgMuted)
    fgComment = try c.decode(String.self, forKey: .fgComment)
    accent = try c.decode(String.self, forKey: .accent)
    selection = try c.decode(String.self, forKey: .selection)
    cursor = try c.decode(String.self, forKey: .cursor)
    red = try c.decode(String.self, forKey: .red)
    orange = try c.decode(String.self, forKey: .orange)
    yellow = try c.decode(String.self, forKey: .yellow)
    green = try c.decode(String.self, forKey: .green)
    cyan = try c.decode(String.self, forKey: .cyan)
    blue = try c.decode(String.self, forKey: .blue)
    magenta = try c.decode(String.self, forKey: .magenta)
    gitAdded = try c.decode(String.self, forKey: .gitAdded)
    gitModified = try c.decode(String.self, forKey: .gitModified)
    gitDeleted = try c.decode(String.self, forKey: .gitDeleted)
    gitRenamed = try c.decode(String.self, forKey: .gitRenamed)
    gitConflict = try c.decode(String.self, forKey: .gitConflict)
    gitIgnored = try c.decode(String.self, forKey: .gitIgnored)
    syntaxKeyword = try c.decode(String.self, forKey: .syntaxKeyword)
    syntaxFunction = try c.decode(String.self, forKey: .syntaxFunction)
    syntaxType = try c.decode(String.self, forKey: .syntaxType)
    syntaxString = try c.decode(String.self, forKey: .syntaxString)
    syntaxNumber = try c.decode(String.self, forKey: .syntaxNumber)
    syntaxConstant = try c.decode(String.self, forKey: .syntaxConstant)
    syntaxComment = try c.decode(String.self, forKey: .syntaxComment)
    syntaxOperator = try c.decode(String.self, forKey: .syntaxOperator)
    syntaxTag = try c.decode(String.self, forKey: .syntaxTag)
    syntaxAttribute = try c.decode(String.self, forKey: .syntaxAttribute)
    syntaxVariable = try c.decode(String.self, forKey: .syntaxVariable)
    syntaxDelimiter = try c.decode(String.self, forKey: .syntaxDelimiter)
    syntaxEscape = try c.decode(String.self, forKey: .syntaxEscape)
    syntaxRegexp = try c.decode(String.self, forKey: .syntaxRegexp)
    syntaxLink = try c.decode(String.self, forKey: .syntaxLink)
    terminalFg = try c.decode(String.self, forKey: .terminalFg)
    terminalBg = try c.decode(String.self, forKey: .terminalBg)
    terminalPalette = try c.decode([String].self, forKey: .terminalPalette)
    // Older serialized themes have no surface_style key — default to "flat"
    // (mirrors serde `#[serde(default = "default_surface_style")]`).
    surfaceStyle = try c.decodeIfPresent(String.self, forKey: .surfaceStyle) ?? "flat"
  }

  public func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: CodingKeys.self)
    try c.encode(id, forKey: .id)
    try c.encode(name, forKey: .name)
    try c.encode(isLight, forKey: .isLight)
    try c.encode(bg, forKey: .bg)
    try c.encode(bgDark, forKey: .bgDark)
    try c.encode(bgHighlight, forKey: .bgHighlight)
    try c.encode(bgSurface, forKey: .bgSurface)
    try c.encode(border, forKey: .border)
    try c.encode(fg, forKey: .fg)
    try c.encode(fgMuted, forKey: .fgMuted)
    try c.encode(fgComment, forKey: .fgComment)
    try c.encode(accent, forKey: .accent)
    try c.encode(selection, forKey: .selection)
    try c.encode(cursor, forKey: .cursor)
    try c.encode(red, forKey: .red)
    try c.encode(orange, forKey: .orange)
    try c.encode(yellow, forKey: .yellow)
    try c.encode(green, forKey: .green)
    try c.encode(cyan, forKey: .cyan)
    try c.encode(blue, forKey: .blue)
    try c.encode(magenta, forKey: .magenta)
    try c.encode(gitAdded, forKey: .gitAdded)
    try c.encode(gitModified, forKey: .gitModified)
    try c.encode(gitDeleted, forKey: .gitDeleted)
    try c.encode(gitRenamed, forKey: .gitRenamed)
    try c.encode(gitConflict, forKey: .gitConflict)
    try c.encode(gitIgnored, forKey: .gitIgnored)
    try c.encode(syntaxKeyword, forKey: .syntaxKeyword)
    try c.encode(syntaxFunction, forKey: .syntaxFunction)
    try c.encode(syntaxType, forKey: .syntaxType)
    try c.encode(syntaxString, forKey: .syntaxString)
    try c.encode(syntaxNumber, forKey: .syntaxNumber)
    try c.encode(syntaxConstant, forKey: .syntaxConstant)
    try c.encode(syntaxComment, forKey: .syntaxComment)
    try c.encode(syntaxOperator, forKey: .syntaxOperator)
    try c.encode(syntaxTag, forKey: .syntaxTag)
    try c.encode(syntaxAttribute, forKey: .syntaxAttribute)
    try c.encode(syntaxVariable, forKey: .syntaxVariable)
    try c.encode(syntaxDelimiter, forKey: .syntaxDelimiter)
    try c.encode(syntaxEscape, forKey: .syntaxEscape)
    try c.encode(syntaxRegexp, forKey: .syntaxRegexp)
    try c.encode(syntaxLink, forKey: .syntaxLink)
    try c.encode(terminalFg, forKey: .terminalFg)
    try c.encode(terminalBg, forKey: .terminalBg)
    try c.encode(terminalPalette, forKey: .terminalPalette)
    try c.encode(surfaceStyle, forKey: .surfaceStyle)
  }
}
