import AppKit

// MARK: - NSColor Hex Helpers

extension NSColor {
    /// Initializes an NSColor from a hex string such as "#1F1F28".
    convenience init(hex: String) {
        let trimmed = hex.trimmingCharacters(in: CharacterSet(charactersIn: "#"))
        var rgb: UInt64 = 0
        Scanner(string: trimmed).scanHexInt64(&rgb)
        let r = CGFloat((rgb >> 16) & 0xFF) / 255.0
        let g = CGFloat((rgb >> 8) & 0xFF) / 255.0
        let b = CGFloat(rgb & 0xFF) / 255.0
        self.init(srgbRed: r, green: g, blue: b, alpha: 1.0)
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

/// A color theme definition for the entire application, mirroring the Rust
/// `ThemeColors` struct from the Linux frontend.
struct Theme {
    let name: String
    let bg: NSColor
    let bgDark: NSColor
    let bgHighlight: NSColor
    let fg: NSColor
    let fgDark: NSColor
    let cyan: NSColor
    let blue: NSColor
    let green: NSColor
    let magenta: NSColor
    let red: NSColor
    let yellow: NSColor
    let orange: NSColor
    let comment: NSColor
    let terminalPalette: [NSColor]

    /// Returns the hex string for the background color (used for WKWebView).
    var bgHex: String { bg.hexString }
    var bgDarkHex: String { bgDark.hexString }
    var bgHighlightHex: String { bgHighlight.hexString }
    var fgHex: String { fg.hexString }
    var fgDarkHex: String { fgDark.hexString }
    var cyanHex: String { cyan.hexString }
    var blueHex: String { blue.hexString }
    var greenHex: String { green.hexString }
    var magentaHex: String { magenta.hexString }
    var redHex: String { red.hexString }
    var yellowHex: String { yellow.hexString }
    var orangeHex: String { orange.hexString }
    var commentHex: String { comment.hexString }
}

// MARK: - Built-in Themes

enum ThemeManager {

    /// Returns the list of all built-in theme names in display order.
    static func availableThemes() -> [String] {
        ["kanagawa", "rose-pine", "nord", "gruvbox", "tokyo-night",
         "tokyo-night-storm", "catppuccin-mocha"]
    }

    /// Returns the theme matching `name` (case-insensitive). Falls back to Nord.
    static func theme(forName name: String) -> Theme {
        switch name.lowercased() {
        case "kanagawa":
            return kanagawa
        case "rose-pine", "rose_pine", "rosepine":
            return rosePine
        case "nord":
            return nord
        case "gruvbox", "gruvbox-dark", "gruvbox_dark":
            return gruvbox
        case "tokyo-night", "tokyo_night", "tokyonight":
            return tokyoNight
        case "tokyo-night-storm", "tokyo_night_storm", "tokyonightstorm":
            return tokyoNightStorm
        case "catppuccin-mocha", "catppuccin_mocha", "catppuccinmocha":
            return catppuccinMocha
        default:
            return nord
        }
    }

    /// Convenience: returns the theme for the given settings.
    static func currentTheme(from settings: Settings) -> Theme {
        theme(forName: settings.colorScheme)
    }

    // MARK: Kanagawa

    static let kanagawa = Theme(
        name: "kanagawa",
        bg: NSColor(hex: "#1F1F28"),
        bgDark: NSColor(hex: "#16161D"),
        bgHighlight: NSColor(hex: "#2A2A37"),
        fg: NSColor(hex: "#DCD7BA"),
        fgDark: NSColor(hex: "#C8C093"),
        cyan: NSColor(hex: "#7AA89F"),
        blue: NSColor(hex: "#7E9CD8"),
        green: NSColor(hex: "#98BB6C"),
        magenta: NSColor(hex: "#957FB8"),
        red: NSColor(hex: "#E46876"),
        yellow: NSColor(hex: "#E6C384"),
        orange: NSColor(hex: "#FFA066"),
        comment: NSColor(hex: "#727169"),
        terminalPalette: [
            "#090618", "#C34043", "#76946A", "#C0A36E", "#7E9CD8", "#957FB8", "#6A9589", "#C8C093",
            "#727169", "#E82424", "#98BB6C", "#E6C384", "#7FB4CA", "#938AA9", "#7AA89F", "#DCD7BA",
        ].map { NSColor(hex: $0) }
    )

    // MARK: Rose Pine

    static let rosePine = Theme(
        name: "rose-pine",
        bg: NSColor(hex: "#191724"),
        bgDark: NSColor(hex: "#1f1d2e"),
        bgHighlight: NSColor(hex: "#26233a"),
        fg: NSColor(hex: "#e0def4"),
        fgDark: NSColor(hex: "#908caa"),
        cyan: NSColor(hex: "#9ccfd8"),
        blue: NSColor(hex: "#31748f"),
        green: NSColor(hex: "#9ccfd8"),
        magenta: NSColor(hex: "#c4a7e7"),
        red: NSColor(hex: "#eb6f92"),
        yellow: NSColor(hex: "#f6c177"),
        orange: NSColor(hex: "#ebbcba"),
        comment: NSColor(hex: "#6e6a86"),
        terminalPalette: [
            "#26233a", "#eb6f92", "#31748f", "#f6c177", "#9ccfd8", "#c4a7e7", "#ebbcba", "#e0def4",
            "#6e6a86", "#eb6f92", "#31748f", "#f6c177", "#9ccfd8", "#c4a7e7", "#ebbcba", "#e0def4",
        ].map { NSColor(hex: $0) }
    )

    // MARK: Nord

    static let nord = Theme(
        name: "nord",
        bg: NSColor(hex: "#2E3440"),
        bgDark: NSColor(hex: "#272C36"),
        bgHighlight: NSColor(hex: "#434C5E"),
        fg: NSColor(hex: "#D8DEE9"),
        fgDark: NSColor(hex: "#E5E9F0"),
        cyan: NSColor(hex: "#88C0D0"),
        blue: NSColor(hex: "#81A1C1"),
        green: NSColor(hex: "#A3BE8C"),
        magenta: NSColor(hex: "#B48EAD"),
        red: NSColor(hex: "#BF616A"),
        yellow: NSColor(hex: "#EBCB8B"),
        orange: NSColor(hex: "#D08770"),
        comment: NSColor(hex: "#4C566A"),
        terminalPalette: [
            "#3B4252", "#BF616A", "#A3BE8C", "#EBCB8B", "#81A1C1", "#B48EAD", "#88C0D0", "#E5E9F0",
            "#4C566A", "#BF616A", "#A3BE8C", "#EBCB8B", "#81A1C1", "#B48EAD", "#8FBCBB", "#ECEFF4",
        ].map { NSColor(hex: $0) }
    )

    // MARK: Gruvbox

    static let gruvbox = Theme(
        name: "gruvbox",
        bg: NSColor(hex: "#282828"),
        bgDark: NSColor(hex: "#1d2021"),
        bgHighlight: NSColor(hex: "#3c3836"),
        fg: NSColor(hex: "#ebdbb2"),
        fgDark: NSColor(hex: "#d5c4a1"),
        cyan: NSColor(hex: "#8ec07c"),
        blue: NSColor(hex: "#83a598"),
        green: NSColor(hex: "#b8bb26"),
        magenta: NSColor(hex: "#d3869b"),
        red: NSColor(hex: "#fb4934"),
        yellow: NSColor(hex: "#fabd2f"),
        orange: NSColor(hex: "#fe8019"),
        comment: NSColor(hex: "#928374"),
        terminalPalette: [
            "#282828", "#cc241d", "#98971a", "#d79921", "#458588", "#b16286", "#689d6a", "#a89984",
            "#928374", "#fb4934", "#b8bb26", "#fabd2f", "#83a598", "#d3869b", "#8ec07c", "#ebdbb2",
        ].map { NSColor(hex: $0) }
    )

    // MARK: Tokyo Night

    static let tokyoNight = Theme(
        name: "tokyo-night",
        bg: NSColor(hex: "#1a1b26"),
        bgDark: NSColor(hex: "#16161e"),
        bgHighlight: NSColor(hex: "#292e42"),
        fg: NSColor(hex: "#c0caf5"),
        fgDark: NSColor(hex: "#a9b1d6"),
        cyan: NSColor(hex: "#7dcfff"),
        blue: NSColor(hex: "#7aa2f7"),
        green: NSColor(hex: "#9ece6a"),
        magenta: NSColor(hex: "#bb9af7"),
        red: NSColor(hex: "#f7768e"),
        yellow: NSColor(hex: "#e0af68"),
        orange: NSColor(hex: "#ff9e64"),
        comment: NSColor(hex: "#565f89"),
        terminalPalette: [
            "#15161e", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#a9b1d6",
            "#414868", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#c0caf5",
        ].map { NSColor(hex: $0) }
    )

    // MARK: Tokyo Night Storm

    static let tokyoNightStorm = Theme(
        name: "tokyo-night-storm",
        bg: NSColor(hex: "#24283b"),
        bgDark: NSColor(hex: "#1f2335"),
        bgHighlight: NSColor(hex: "#292e42"),
        fg: NSColor(hex: "#c0caf5"),
        fgDark: NSColor(hex: "#a9b1d6"),
        cyan: NSColor(hex: "#7dcfff"),
        blue: NSColor(hex: "#7aa2f7"),
        green: NSColor(hex: "#9ece6a"),
        magenta: NSColor(hex: "#bb9af7"),
        red: NSColor(hex: "#f7768e"),
        yellow: NSColor(hex: "#e0af68"),
        orange: NSColor(hex: "#ff9e64"),
        comment: NSColor(hex: "#565f89"),
        terminalPalette: [
            "#1d202f", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#a9b1d6",
            "#414868", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#c0caf5",
        ].map { NSColor(hex: $0) }
    )

    // MARK: Catppuccin Mocha

    static let catppuccinMocha = Theme(
        name: "catppuccin-mocha",
        bg: NSColor(hex: "#1e1e2e"),
        bgDark: NSColor(hex: "#181825"),
        bgHighlight: NSColor(hex: "#313244"),
        fg: NSColor(hex: "#cdd6f4"),
        fgDark: NSColor(hex: "#bac2de"),
        cyan: NSColor(hex: "#94e2d5"),
        blue: NSColor(hex: "#89b4fa"),
        green: NSColor(hex: "#a6e3a1"),
        magenta: NSColor(hex: "#cba6f7"),
        red: NSColor(hex: "#f38ba8"),
        yellow: NSColor(hex: "#f9e2af"),
        orange: NSColor(hex: "#fab387"),
        comment: NSColor(hex: "#6c7086"),
        terminalPalette: [
            "#45475a", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#cba6f7", "#94e2d5", "#bac2de",
            "#585b70", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#cba6f7", "#94e2d5", "#cdd6f4",
        ].map { NSColor(hex: $0) }
    )
}

// MARK: - Monaco Theme Generation

extension Theme {
    /// Generates a `MonacoThemeDefinition` from this theme's colors, suitable
    /// for sending to the Monaco WebView via `EditorCommand.setTheme`.
    func monacoThemeDefinition() -> MonacoThemeDefinition {
        MonacoThemeDefinition(
            base: "vs-dark",
            inherit: true,
            rules: [
                MonacoTokenRule(token: "comment", foreground: commentHex, fontStyle: "italic"),
                MonacoTokenRule(token: "keyword", foreground: magentaHex),
                MonacoTokenRule(token: "keyword.control", foreground: magentaHex),
                MonacoTokenRule(token: "keyword.operator", foreground: cyanHex),
                MonacoTokenRule(token: "string", foreground: greenHex),
                MonacoTokenRule(token: "string.escape", foreground: orangeHex),
                MonacoTokenRule(token: "number", foreground: orangeHex),
                MonacoTokenRule(token: "constant", foreground: orangeHex),
                MonacoTokenRule(token: "type", foreground: yellowHex),
                MonacoTokenRule(token: "type.identifier", foreground: yellowHex),
                MonacoTokenRule(token: "class", foreground: yellowHex),
                MonacoTokenRule(token: "function", foreground: blueHex),
                MonacoTokenRule(token: "function.declaration", foreground: blueHex),
                MonacoTokenRule(token: "variable", foreground: fgHex),
                MonacoTokenRule(token: "variable.predefined", foreground: cyanHex),
                MonacoTokenRule(token: "operator", foreground: cyanHex),
                MonacoTokenRule(token: "delimiter", foreground: fgDarkHex),
                MonacoTokenRule(token: "tag", foreground: redHex),
                MonacoTokenRule(token: "attribute.name", foreground: yellowHex),
                MonacoTokenRule(token: "attribute.value", foreground: greenHex),
                MonacoTokenRule(token: "regexp", foreground: redHex),
                MonacoTokenRule(token: "annotation", foreground: yellowHex),
                MonacoTokenRule(token: "namespace", foreground: yellowHex),
            ],
            colors: MonacoThemeColors(
                editorBackground: bgHex,
                editorForeground: fgHex,
                editorLineHighlightBackground: bgHighlightHex,
                editorSelectionBackground: bgHighlightHex,
                editorCursorForeground: cyanHex,
                editorLineNumberForeground: commentHex,
                editorLineNumberActiveForeground: fgDarkHex,
                editorWidgetBackground: bgDarkHex,
                editorSuggestWidgetBackground: bgDarkHex,
                editorSuggestWidgetSelectedBackground: bgHighlightHex,
                editorHoverWidgetBackground: bgDarkHex,
                editorGutterBackground: bgDarkHex,
                minimapBackground: bgDarkHex,
                scrollbarSliderBackground: commentHex,
                scrollbarSliderHoverBackground: fgDarkHex,
                diffAddedColor: greenHex,
                diffModifiedColor: yellowHex,
                diffDeletedColor: redHex
            )
        )
    }
}
