import AppKit
import SwiftUI

// MARK: - App Font Helper

extension NSFont {
    /// Returns Inter at the given size/weight, falling back to the
    /// system proportional font if the bundled font isn't available yet.
    static func appFont(ofSize size: CGFloat, weight: NSFont.Weight = .regular) -> NSFont {
        // Map NSFont.Weight to the closest Inter style name.
        let name: String
        switch weight {
        case .bold: name = "Inter-Bold"
        case .semibold: name = "Inter-SemiBold"
        case .medium: name = "Inter-Medium"
        default: name = "Inter-Regular"
        }
        return NSFont(name: name, size: size)
            ?? NSFont.systemFont(ofSize: size, weight: weight)
    }
}

// MARK: - NSColor Hex Helpers

extension NSColor {
    /// Initializes an NSColor from a hex string such as "#1F1F28".
    /// Accepts 6-digit (RGB) and 8-digit (RGBA) hex strings.
    convenience init(hex: String) {
        let trimmed = hex.trimmingCharacters(in: CharacterSet(charactersIn: "#"))
        guard trimmed.count == 6 || trimmed.count == 8,
              trimmed.allSatisfy({ $0.isHexDigit }) else {
            self.init(srgbRed: 0, green: 0, blue: 0, alpha: 1.0)
            return
        }
        var rgb: UInt64 = 0
        Scanner(string: trimmed).scanHexInt64(&rgb)
        if trimmed.count == 8 {
            let r = CGFloat((rgb >> 24) & 0xFF) / 255.0
            let g = CGFloat((rgb >> 16) & 0xFF) / 255.0
            let b = CGFloat((rgb >> 8) & 0xFF) / 255.0
            let a = CGFloat(rgb & 0xFF) / 255.0
            self.init(srgbRed: r, green: g, blue: b, alpha: a)
        } else {
            let r = CGFloat((rgb >> 16) & 0xFF) / 255.0
            let g = CGFloat((rgb >> 8) & 0xFF) / 255.0
            let b = CGFloat(rgb & 0xFF) / 255.0
            self.init(srgbRed: r, green: g, blue: b, alpha: 1.0)
        }
    }

    /// Static factory method to create an NSColor from a hex string.
    static func fromHex(_ hex: String) -> NSColor {
        NSColor(hex: hex)
    }

    /// Returns the color as a `#RRGGBB` hex string.
    var hexString: String {
        guard let rgb = usingColorSpace(.sRGB) else { return "#000000" }
        let r = Int(round(rgb.redComponent * 255))
        let g = Int(round(rgb.greenComponent * 255))
        let b = Int(round(rgb.blueComponent * 255))
        return String(format: "#%02X%02X%02X", r, g, b)
    }
}

// MARK: - Theme Definition

/// A color theme definition decoded from the Rust backend via FFI.
///
/// All color fields are stored as hex strings (e.g. "#1F1F28").
/// Computed NSColor properties are provided for AppKit usage.
struct Theme: Codable {
    let id: String
    let name: String
    let isLight: Bool
    let bg: String
    let bgDark: String
    let bgHighlight: String
    let bgSurface: String
    let border: String
    let fg: String
    let fgMuted: String
    let fgComment: String
    let accent: String
    let selection: String
    let cursor: String
    let red: String
    let orange: String
    let yellow: String
    let green: String
    let cyan: String
    let blue: String
    let magenta: String
    let gitAdded: String
    let gitModified: String
    let gitDeleted: String
    let gitRenamed: String
    let gitConflict: String
    let gitIgnored: String
    let syntaxKeyword: String
    let syntaxFunction: String
    let syntaxType: String
    let syntaxString: String
    let syntaxNumber: String
    let syntaxConstant: String
    let syntaxComment: String
    let syntaxOperator: String
    let syntaxTag: String
    let syntaxAttribute: String
    let syntaxVariable: String
    let syntaxDelimiter: String
    let syntaxEscape: String
    let syntaxRegexp: String
    let syntaxLink: String
    let terminalFg: String
    let terminalBg: String
    let terminalPalette: [String]

    // Pre-resolved SwiftUI.Color values. Computed once at decode time so hot
    // SwiftUI bodies (e.g. the status bar) don't re-parse hex strings on every
    // re-render. Not part of Codable — absent from CodingKeys below.
    let colorBg: Color
    let colorBgDark: Color
    let colorBgHighlight: Color
    let colorBgSurface: Color
    let colorBorder: Color
    let colorFg: Color
    let colorFgMuted: Color
    let colorFgComment: Color
    let colorAccent: Color
    let colorSelection: Color
    let colorCursor: Color
    let colorRed: Color
    let colorOrange: Color
    let colorYellow: Color
    let colorGreen: Color
    let colorCyan: Color
    let colorBlue: Color
    let colorMagenta: Color
    let colorGitAdded: Color
    let colorGitModified: Color
    let colorGitDeleted: Color
    let colorGitRenamed: Color
    let colorGitConflict: Color
    let colorGitIgnored: Color
    let colorSyntaxKeyword: Color
    let colorSyntaxFunction: Color
    let colorSyntaxType: Color
    let colorSyntaxString: Color
    let colorSyntaxNumber: Color
    let colorSyntaxConstant: Color
    let colorSyntaxComment: Color
    let colorSyntaxOperator: Color
    let colorSyntaxTag: Color
    let colorSyntaxAttribute: Color
    let colorSyntaxVariable: Color
    let colorSyntaxDelimiter: Color
    let colorSyntaxEscape: Color
    let colorSyntaxRegexp: Color
    let colorSyntaxLink: Color
    let colorTerminalFg: Color
    let colorTerminalBg: Color

    enum CodingKeys: String, CodingKey {
        case id
        case name
        case isLight = "is_light"
        case bg
        case bgDark = "bg_dark"
        case bgHighlight = "bg_highlight"
        case bgSurface = "bg_surface"
        case border
        case fg
        case fgMuted = "fg_muted"
        case fgComment = "fg_comment"
        case accent
        case selection
        case cursor
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
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.id = try c.decode(String.self, forKey: .id)
        self.name = try c.decode(String.self, forKey: .name)
        self.isLight = try c.decode(Bool.self, forKey: .isLight)
        self.bg = try c.decode(String.self, forKey: .bg)
        self.bgDark = try c.decode(String.self, forKey: .bgDark)
        self.bgHighlight = try c.decode(String.self, forKey: .bgHighlight)
        self.bgSurface = try c.decode(String.self, forKey: .bgSurface)
        self.border = try c.decode(String.self, forKey: .border)
        self.fg = try c.decode(String.self, forKey: .fg)
        self.fgMuted = try c.decode(String.self, forKey: .fgMuted)
        self.fgComment = try c.decode(String.self, forKey: .fgComment)
        self.accent = try c.decode(String.self, forKey: .accent)
        self.selection = try c.decode(String.self, forKey: .selection)
        self.cursor = try c.decode(String.self, forKey: .cursor)
        self.red = try c.decode(String.self, forKey: .red)
        self.orange = try c.decode(String.self, forKey: .orange)
        self.yellow = try c.decode(String.self, forKey: .yellow)
        self.green = try c.decode(String.self, forKey: .green)
        self.cyan = try c.decode(String.self, forKey: .cyan)
        self.blue = try c.decode(String.self, forKey: .blue)
        self.magenta = try c.decode(String.self, forKey: .magenta)
        self.gitAdded = try c.decode(String.self, forKey: .gitAdded)
        self.gitModified = try c.decode(String.self, forKey: .gitModified)
        self.gitDeleted = try c.decode(String.self, forKey: .gitDeleted)
        self.gitRenamed = try c.decode(String.self, forKey: .gitRenamed)
        self.gitConflict = try c.decode(String.self, forKey: .gitConflict)
        self.gitIgnored = try c.decode(String.self, forKey: .gitIgnored)
        self.syntaxKeyword = try c.decode(String.self, forKey: .syntaxKeyword)
        self.syntaxFunction = try c.decode(String.self, forKey: .syntaxFunction)
        self.syntaxType = try c.decode(String.self, forKey: .syntaxType)
        self.syntaxString = try c.decode(String.self, forKey: .syntaxString)
        self.syntaxNumber = try c.decode(String.self, forKey: .syntaxNumber)
        self.syntaxConstant = try c.decode(String.self, forKey: .syntaxConstant)
        self.syntaxComment = try c.decode(String.self, forKey: .syntaxComment)
        self.syntaxOperator = try c.decode(String.self, forKey: .syntaxOperator)
        self.syntaxTag = try c.decode(String.self, forKey: .syntaxTag)
        self.syntaxAttribute = try c.decode(String.self, forKey: .syntaxAttribute)
        self.syntaxVariable = try c.decode(String.self, forKey: .syntaxVariable)
        self.syntaxDelimiter = try c.decode(String.self, forKey: .syntaxDelimiter)
        self.syntaxEscape = try c.decode(String.self, forKey: .syntaxEscape)
        self.syntaxRegexp = try c.decode(String.self, forKey: .syntaxRegexp)
        self.syntaxLink = try c.decode(String.self, forKey: .syntaxLink)
        self.terminalFg = try c.decode(String.self, forKey: .terminalFg)
        self.terminalBg = try c.decode(String.self, forKey: .terminalBg)
        self.terminalPalette = try c.decode([String].self, forKey: .terminalPalette)

        self.colorBg = Color(NSColor(hex: self.bg))
        self.colorBgDark = Color(NSColor(hex: self.bgDark))
        self.colorBgHighlight = Color(NSColor(hex: self.bgHighlight))
        self.colorBgSurface = Color(NSColor(hex: self.bgSurface))
        self.colorBorder = Color(NSColor(hex: self.border))
        self.colorFg = Color(NSColor(hex: self.fg))
        self.colorFgMuted = Color(NSColor(hex: self.fgMuted))
        self.colorFgComment = Color(NSColor(hex: self.fgComment))
        self.colorAccent = Color(NSColor(hex: self.accent))
        self.colorSelection = Color(NSColor(hex: self.selection))
        self.colorCursor = Color(NSColor(hex: self.cursor))
        self.colorRed = Color(NSColor(hex: self.red))
        self.colorOrange = Color(NSColor(hex: self.orange))
        self.colorYellow = Color(NSColor(hex: self.yellow))
        self.colorGreen = Color(NSColor(hex: self.green))
        self.colorCyan = Color(NSColor(hex: self.cyan))
        self.colorBlue = Color(NSColor(hex: self.blue))
        self.colorMagenta = Color(NSColor(hex: self.magenta))
        self.colorGitAdded = Color(NSColor(hex: self.gitAdded))
        self.colorGitModified = Color(NSColor(hex: self.gitModified))
        self.colorGitDeleted = Color(NSColor(hex: self.gitDeleted))
        self.colorGitRenamed = Color(NSColor(hex: self.gitRenamed))
        self.colorGitConflict = Color(NSColor(hex: self.gitConflict))
        self.colorGitIgnored = Color(NSColor(hex: self.gitIgnored))
        self.colorSyntaxKeyword = Color(NSColor(hex: self.syntaxKeyword))
        self.colorSyntaxFunction = Color(NSColor(hex: self.syntaxFunction))
        self.colorSyntaxType = Color(NSColor(hex: self.syntaxType))
        self.colorSyntaxString = Color(NSColor(hex: self.syntaxString))
        self.colorSyntaxNumber = Color(NSColor(hex: self.syntaxNumber))
        self.colorSyntaxConstant = Color(NSColor(hex: self.syntaxConstant))
        self.colorSyntaxComment = Color(NSColor(hex: self.syntaxComment))
        self.colorSyntaxOperator = Color(NSColor(hex: self.syntaxOperator))
        self.colorSyntaxTag = Color(NSColor(hex: self.syntaxTag))
        self.colorSyntaxAttribute = Color(NSColor(hex: self.syntaxAttribute))
        self.colorSyntaxVariable = Color(NSColor(hex: self.syntaxVariable))
        self.colorSyntaxDelimiter = Color(NSColor(hex: self.syntaxDelimiter))
        self.colorSyntaxEscape = Color(NSColor(hex: self.syntaxEscape))
        self.colorSyntaxRegexp = Color(NSColor(hex: self.syntaxRegexp))
        self.colorSyntaxLink = Color(NSColor(hex: self.syntaxLink))
        self.colorTerminalFg = Color(NSColor(hex: self.terminalFg))
        self.colorTerminalBg = Color(NSColor(hex: self.terminalBg))
    }

    // MARK: - Computed NSColor Properties

    var bgColor: NSColor { NSColor(hex: bg) }
    var bgDarkColor: NSColor { NSColor(hex: bgDark) }
    var bgHighlightColor: NSColor { NSColor(hex: bgHighlight) }
    var bgSurfaceColor: NSColor { NSColor(hex: bgSurface) }
    var borderColor: NSColor { NSColor(hex: border) }
    var accentColor: NSColor { NSColor(hex: accent) }
    var fgColor: NSColor { NSColor(hex: fg) }
    var fgMutedColor: NSColor { NSColor(hex: fgMuted) }
    var fgCommentColor: NSColor { NSColor(hex: fgComment) }
    var cyanColor: NSColor { NSColor(hex: cyan) }
    var blueColor: NSColor { NSColor(hex: blue) }
    var greenColor: NSColor { NSColor(hex: green) }
    var magentaColor: NSColor { NSColor(hex: magenta) }
    var redColor: NSColor { NSColor(hex: red) }
    var yellowColor: NSColor { NSColor(hex: yellow) }
    var orangeColor: NSColor { NSColor(hex: orange) }
    var cursorColor: NSColor { NSColor(hex: cursor) }

    /// Monaco base theme: `"vs-dark"` for dark themes, `"vs"` for light themes.
    var base: String { isLight ? "vs" : "vs-dark" }
}

// MARK: - ThemeManager (FFI-backed)

enum ThemeManager {

    /// JSON decoder configured for snake_case keys from the Rust backend.
    private static let decoder: JSONDecoder = {
        let d = JSONDecoder()
        return d
    }()

    /// Returns the list of all available theme names (built-in + user).
    static func availableThemes() -> [String] {
        return ImpulseCore.availableThemes()
    }

    /// Returns the display name for a theme ID.
    static func displayName(for id: String) -> String {
        return ImpulseCore.themeDisplayName(id: id)
    }

    /// Returns the theme matching `name`. Falls back to Nord on decode failure.
    static func theme(forName name: String) -> Theme {
        let json = ImpulseCore.getTheme(name: name)
        guard let data = json.data(using: .utf8),
              let theme = try? decoder.decode(Theme.self, from: data) else {
            // If decoding fails and we're not already requesting nord, try nord
            if name != "nord" {
                return theme(forName: "nord")
            }
            // Ultimate fallback — should never happen
            fatalError("Failed to decode fallback theme 'nord' from FFI")
        }
        return theme
    }

    /// Convenience: returns the theme for the given settings.
    static func currentTheme(from settings: Settings) -> Theme {
        theme(forName: settings.colorScheme)
    }

    /// Returns a `MonacoThemeDefinition` for the named theme, decoded from FFI JSON.
    static func monacoTheme(forName name: String) -> MonacoThemeDefinition {
        let json = ImpulseCore.getMonacoTheme(name: name)
        guard let data = json.data(using: .utf8),
              let theme = try? decoder.decode(MonacoThemeDefinition.self, from: data) else {
            // Fallback: return a minimal definition
            return MonacoThemeDefinition(
                base: "vs-dark", inherit: true, rules: [],
                colors: MonacoThemeColors(
                    editorBackground: "#1F1F28", editorForeground: "#DCD7BA",
                    editorLineHighlightBackground: "#2A2A37",
                    editorSelectionBackground: "#7E9CD850",
                    editorCursorForeground: "#7AA89F",
                    editorLineNumberForeground: "#727169",
                    editorLineNumberActiveForeground: "#DCD7BA",
                    editorWidgetBackground: "#141419",
                    editorSuggestWidgetBackground: "#141419",
                    editorSuggestWidgetSelectedBackground: "#262633",
                    editorHoverWidgetBackground: "#141419",
                    editorGutterBackground: "#1F1F28",
                    minimapBackground: "#141419",
                    scrollbarSliderBackground: "#72716940",
                    scrollbarSliderHoverBackground: "#72716980",
                    scrollbarSliderActiveBackground: "#727169A0",
                    diffAddedColor: "#98BB6C",
                    diffModifiedColor: "#E6C384",
                    diffDeletedColor: "#E46876"
                )
            )
        }
        return theme
    }

    /// Returns the markdown theme JSON string for the named theme.
    static func markdownThemeJSON(forName name: String) -> String {
        return ImpulseCore.getMarkdownTheme(name: name)
    }
}
