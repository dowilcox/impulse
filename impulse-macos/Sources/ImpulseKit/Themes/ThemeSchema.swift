import Foundation

// Ported from the TOML schema types in impulse-core/src/theme.rs.
// These structs mirror `ThemeFile` and friends field-for-field, decoded from
// theme `.toml` files with TOMLKit's `TOMLDecoder`.

/// Top-level structure of a `.toml` theme file.
public struct ThemeFile {
  public var name: String
  /// `"dark"` or `"light"`.
  public var variant: String
  public var palette: ThemePalette
  public var ui: SemanticUI
  public var syntax: SemanticSyntax
  public var terminal: TerminalPalette?
}

extension ThemeFile: Codable {
  enum CodingKeys: String, CodingKey {
    case name, variant, palette, ui, syntax, terminal
  }

  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: CodingKeys.self)
    name = try c.decode(String.self, forKey: .name)
    variant = try c.decode(String.self, forKey: .variant)
    palette = try c.decode(ThemePalette.self, forKey: .palette)
    // `[ui]` and `[syntax]` are optional sections (serde `#[serde(default)]`).
    ui = try c.decodeIfPresent(SemanticUI.self, forKey: .ui) ?? SemanticUI()
    syntax = try c.decodeIfPresent(SemanticSyntax.self, forKey: .syntax) ?? SemanticSyntax()
    terminal = try c.decodeIfPresent(TerminalPalette.self, forKey: .terminal)
  }

  public func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: CodingKeys.self)
    try c.encode(name, forKey: .name)
    try c.encode(variant, forKey: .variant)
    try c.encode(palette, forKey: .palette)
    try c.encode(ui, forKey: .ui)
    try c.encode(syntax, forKey: .syntax)
    try c.encodeIfPresent(terminal, forKey: .terminal)
  }
}

/// The core color palette — 10 required hues plus 4 optional derived shades.
public struct ThemePalette: Codable {
  public var bg: String
  public var fg: String
  public var accent: String
  public var red: String
  public var orange: String
  public var yellow: String
  public var green: String
  public var cyan: String
  public var blue: String
  public var magenta: String
  /// Darker surface shade — derived from `bg` if omitted.
  public var surface: String?
  /// Lighter overlay shade — derived from `bg` if omitted.
  public var overlay: String?
  /// Muted foreground — derived from `fg` if omitted.
  public var muted: String?
  /// Subtle foreground (comments) — derived from `fg` if omitted.
  public var subtle: String?
}

/// Semantic UI color overrides. All fields optional — derived from palette.
public struct SemanticUI: Codable {
  public var bgDark: String?
  public var bgHighlight: String?
  public var bgSurface: String?
  public var border: String?
  public var fgMuted: String?
  public var fgComment: String?
  public var selection: String?
  public var cursor: String?
  public var gitAdded: String?
  public var gitModified: String?
  public var gitDeleted: String?
  public var gitRenamed: String?
  public var gitConflict: String?
  public var gitIgnored: String?
  /// Content-surface presentation: `"flat"` (default) or `"card"`.
  public var surfaceStyle: String?

  public init() {}

  enum CodingKeys: String, CodingKey {
    case bgDark = "bg_dark"
    case bgHighlight = "bg_highlight"
    case bgSurface = "bg_surface"
    case border
    case fgMuted = "fg_muted"
    case fgComment = "fg_comment"
    case selection
    case cursor
    case gitAdded = "git_added"
    case gitModified = "git_modified"
    case gitDeleted = "git_deleted"
    case gitRenamed = "git_renamed"
    case gitConflict = "git_conflict"
    case gitIgnored = "git_ignored"
    case surfaceStyle = "surface_style"
  }
}

/// Semantic syntax color overrides. All fields optional — derived from palette.
public struct SemanticSyntax: Codable {
  public var keyword: String?
  public var function: String?
  public var type: String?
  public var string: String?
  public var number: String?
  public var constant: String?
  public var comment: String?
  public var `operator`: String?
  public var tag: String?
  public var attribute: String?
  public var variable: String?
  public var delimiter: String?
  public var escape: String?
  public var regexp: String?
  public var link: String?

  public init() {}
}

/// 16-color terminal palette, compatible with Alacritty/Ghostty/Kitty.
public struct TerminalPalette: Codable {
  public var black: String
  public var red: String
  public var green: String
  public var yellow: String
  public var blue: String
  public var magenta: String
  public var cyan: String
  public var white: String
  public var brightBlack: String
  public var brightRed: String
  public var brightGreen: String
  public var brightYellow: String
  public var brightBlue: String
  public var brightMagenta: String
  public var brightCyan: String
  public var brightWhite: String

  enum CodingKeys: String, CodingKey {
    case black, red, green, yellow, blue, magenta, cyan, white
    case brightBlack = "bright_black"
    case brightRed = "bright_red"
    case brightGreen = "bright_green"
    case brightYellow = "bright_yellow"
    case brightBlue = "bright_blue"
    case brightMagenta = "bright_magenta"
    case brightCyan = "bright_cyan"
    case brightWhite = "bright_white"
  }
}
