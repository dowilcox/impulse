import AppKit

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
    /// Monaco base theme: `"vs-dark"` for dark themes, `"vs"` for light themes.
    let base: String
    /// Editor selection background — a hex color with alpha (e.g. `"#7E9CD850"`).
    let selection: String
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
    /// Whether this is a light theme.
    var isLight: Bool { base == "vs" }
}

// MARK: - Built-in Themes

enum ThemeManager {

    /// Returns the list of all built-in theme names in display order.
    static func availableThemes() -> [String] {
        ["kanagawa", "rose-pine", "nord", "gruvbox", "tokyo-night",
         "tokyo-night-storm", "catppuccin-mocha", "dracula",
         "solarized-dark", "one-dark", "ayu-dark",
         "everforest-dark", "github-dark", "monokai-pro", "palenight",
         "solarized-light", "catppuccin-latte", "github-light"]
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
        case "dracula":
            return dracula
        case "solarized-dark", "solarized_dark", "solarizeddark":
            return solarizedDark
        case "one-dark", "one_dark", "onedark":
            return oneDark
        case "ayu-dark", "ayu_dark", "ayudark":
            return ayuDark
        case "everforest-dark", "everforest_dark", "everforestdark":
            return everforestDark
        case "github-dark", "github_dark", "githubdark":
            return githubDark
        case "monokai-pro", "monokai_pro", "monokaipro":
            return monokaiPro
        case "palenight":
            return palenight
        case "solarized-light", "solarized_light", "solarizedlight":
            return solarizedLight
        case "catppuccin-latte", "catppuccin_latte", "catppuccinlatte":
            return catppuccinLatte
        case "github-light", "github_light", "githublight":
            return githubLight
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
        base: "vs-dark",
        selection: "#7E9CD850",
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
        blue: NSColor(hex: "#4392b5"),
        green: NSColor(hex: "#9ccfd8"),
        magenta: NSColor(hex: "#c4a7e7"),
        red: NSColor(hex: "#eb6f92"),
        yellow: NSColor(hex: "#f6c177"),
        orange: NSColor(hex: "#ebbcba"),
        comment: NSColor(hex: "#6e6a86"),
        base: "vs-dark",
        selection: "#c4a7e740",
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
        base: "vs-dark",
        selection: "#81A1C150",
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
        base: "vs-dark",
        selection: "#83a59850",
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
        base: "vs-dark",
        selection: "#7aa2f740",
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
        base: "vs-dark",
        selection: "#7aa2f740",
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
        base: "vs-dark",
        selection: "#89b4fa40",
        terminalPalette: [
            "#45475a", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#cba6f7", "#94e2d5", "#bac2de",
            "#585b70", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#cba6f7", "#94e2d5", "#cdd6f4",
        ].map { NSColor(hex: $0) }
    )

    // MARK: Dracula

    static let dracula = Theme(
        name: "dracula",
        bg: NSColor(hex: "#282a36"),
        bgDark: NSColor(hex: "#21222c"),
        bgHighlight: NSColor(hex: "#44475a"),
        fg: NSColor(hex: "#f8f8f2"),
        fgDark: NSColor(hex: "#8490b7"),
        cyan: NSColor(hex: "#8be9fd"),
        blue: NSColor(hex: "#7c89b4"),
        green: NSColor(hex: "#50fa7b"),
        magenta: NSColor(hex: "#ff79c6"),
        red: NSColor(hex: "#ff5555"),
        yellow: NSColor(hex: "#f1fa8c"),
        orange: NSColor(hex: "#ffb86c"),
        comment: NSColor(hex: "#6272a4"),
        base: "vs-dark",
        selection: "#bd93f940",
        terminalPalette: [
            "#21222c", "#ff5555", "#50fa7b", "#f1fa8c", "#bd93f9", "#ff79c6", "#8be9fd", "#f8f8f2",
            "#6272a4", "#ff6e6e", "#69ff94", "#ffffa5", "#d6acff", "#ff92df", "#a4ffff", "#ffffff",
        ].map { NSColor(hex: $0) }
    )

    // MARK: Solarized Dark

    static let solarizedDark = Theme(
        name: "solarized-dark",
        bg: NSColor(hex: "#002b36"),
        bgDark: NSColor(hex: "#001e26"),
        bgHighlight: NSColor(hex: "#073642"),
        fg: NSColor(hex: "#839496"),
        fgDark: NSColor(hex: "#748e97"),
        cyan: NSColor(hex: "#2aa198"),
        blue: NSColor(hex: "#268bd2"),
        green: NSColor(hex: "#859900"),
        magenta: NSColor(hex: "#d33682"),
        red: NSColor(hex: "#dc322f"),
        yellow: NSColor(hex: "#b58900"),
        orange: NSColor(hex: "#cb4b16"),
        comment: NSColor(hex: "#586e75"),
        base: "vs-dark",
        selection: "#268bd240",
        terminalPalette: [
            "#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198", "#eee8d5",
            "#002b36", "#cb4b16", "#586e75", "#657b83", "#839496", "#6c71c4", "#93a1a1", "#fdf6e3",
        ].map { NSColor(hex: $0) }
    )

    // MARK: One Dark

    static let oneDark = Theme(
        name: "one-dark",
        bg: NSColor(hex: "#282c34"),
        bgDark: NSColor(hex: "#21252b"),
        bgHighlight: NSColor(hex: "#2c313a"),
        fg: NSColor(hex: "#abb2bf"),
        fgDark: NSColor(hex: "#8c93a1"),
        cyan: NSColor(hex: "#56b6c2"),
        blue: NSColor(hex: "#61afef"),
        green: NSColor(hex: "#98c379"),
        magenta: NSColor(hex: "#c678dd"),
        red: NSColor(hex: "#e06c75"),
        yellow: NSColor(hex: "#e5c07b"),
        orange: NSColor(hex: "#d19a66"),
        comment: NSColor(hex: "#5c6370"),
        base: "vs-dark",
        selection: "#61afef40",
        terminalPalette: [
            "#21252b", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#abb2bf",
            "#5c6370", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#ffffff",
        ].map { NSColor(hex: $0) }
    )

    // MARK: Ayu Dark

    static let ayuDark = Theme(
        name: "ayu-dark",
        bg: NSColor(hex: "#0b0e14"),
        bgDark: NSColor(hex: "#07090d"),
        bgHighlight: NSColor(hex: "#131721"),
        fg: NSColor(hex: "#bfbdb6"),
        fgDark: NSColor(hex: "#797f8e"),
        cyan: NSColor(hex: "#73b8ff"),
        blue: NSColor(hex: "#59c2ff"),
        green: NSColor(hex: "#aad94c"),
        magenta: NSColor(hex: "#d2a6ff"),
        red: NSColor(hex: "#f07178"),
        yellow: NSColor(hex: "#ffb454"),
        orange: NSColor(hex: "#ff8f40"),
        comment: NSColor(hex: "#565b66"),
        base: "vs-dark",
        selection: "#59c2ff30",
        terminalPalette: [
            "#07090d", "#f07178", "#aad94c", "#ffb454", "#59c2ff", "#d2a6ff", "#73b8ff", "#bfbdb6",
            "#565b66", "#f07178", "#aad94c", "#ffb454", "#59c2ff", "#d2a6ff", "#73b8ff", "#ffffff",
        ].map { NSColor(hex: $0) }
    )

    // MARK: Everforest Dark

    static let everforestDark = Theme(
        name: "everforest-dark",
        bg: NSColor(hex: "#2d353b"),
        bgDark: NSColor(hex: "#272e33"),
        bgHighlight: NSColor(hex: "#3d484d"),
        fg: NSColor(hex: "#d3c6aa"),
        fgDark: NSColor(hex: "#9da9a0"),
        cyan: NSColor(hex: "#83c092"),
        blue: NSColor(hex: "#7fbbb3"),
        green: NSColor(hex: "#a7c080"),
        magenta: NSColor(hex: "#d699b6"),
        red: NSColor(hex: "#e67e80"),
        yellow: NSColor(hex: "#dbbc7f"),
        orange: NSColor(hex: "#e69875"),
        comment: NSColor(hex: "#7a8478"),
        base: "vs-dark",
        selection: "#7fbbb340",
        terminalPalette: [
            "#272e33", "#e67e80", "#a7c080", "#dbbc7f", "#7fbbb3", "#d699b6", "#83c092", "#9da9a0",
            "#7a8478", "#e67e80", "#a7c080", "#dbbc7f", "#7fbbb3", "#d699b6", "#83c092", "#d3c6aa",
        ].map { NSColor(hex: $0) }
    )

    // MARK: GitHub Dark

    static let githubDark = Theme(
        name: "github-dark",
        bg: NSColor(hex: "#0d1117"),
        bgDark: NSColor(hex: "#010409"),
        bgHighlight: NSColor(hex: "#161b22"),
        fg: NSColor(hex: "#e6edf3"),
        fgDark: NSColor(hex: "#8b949e"),
        cyan: NSColor(hex: "#79c0ff"),
        blue: NSColor(hex: "#79c0ff"),
        green: NSColor(hex: "#7ee787"),
        magenta: NSColor(hex: "#d2a8ff"),
        red: NSColor(hex: "#ff7b72"),
        yellow: NSColor(hex: "#ffa657"),
        orange: NSColor(hex: "#f0883e"),
        comment: NSColor(hex: "#8b949e"),
        base: "vs-dark",
        selection: "#79c0ff30",
        terminalPalette: [
            "#010409", "#ff7b72", "#7ee787", "#ffa657", "#79c0ff", "#d2a8ff", "#a5d6ff", "#8b949e",
            "#6e7681", "#ffa198", "#7ee787", "#ffa657", "#79c0ff", "#d2a8ff", "#a5d6ff", "#e6edf3",
        ].map { NSColor(hex: $0) }
    )

    // MARK: Monokai Pro

    static let monokaiPro = Theme(
        name: "monokai-pro",
        bg: NSColor(hex: "#2d2a2e"),
        bgDark: NSColor(hex: "#221f22"),
        bgHighlight: NSColor(hex: "#403e41"),
        fg: NSColor(hex: "#fcfcfa"),
        fgDark: NSColor(hex: "#939293"),
        cyan: NSColor(hex: "#78dce8"),
        blue: NSColor(hex: "#78dce8"),
        green: NSColor(hex: "#a9dc76"),
        magenta: NSColor(hex: "#ab9df2"),
        red: NSColor(hex: "#ff6188"),
        yellow: NSColor(hex: "#ffd866"),
        orange: NSColor(hex: "#fc9867"),
        comment: NSColor(hex: "#727072"),
        base: "vs-dark",
        selection: "#ab9df240",
        terminalPalette: [
            "#221f22", "#ff6188", "#a9dc76", "#ffd866", "#78dce8", "#ab9df2", "#78dce8", "#939293",
            "#727072", "#ff6188", "#a9dc76", "#ffd866", "#78dce8", "#ab9df2", "#78dce8", "#fcfcfa",
        ].map { NSColor(hex: $0) }
    )

    // MARK: Palenight

    static let palenight = Theme(
        name: "palenight",
        bg: NSColor(hex: "#292d3e"),
        bgDark: NSColor(hex: "#1b1e2b"),
        bgHighlight: NSColor(hex: "#32374d"),
        fg: NSColor(hex: "#a6accd"),
        fgDark: NSColor(hex: "#868bab"),
        cyan: NSColor(hex: "#89ddff"),
        blue: NSColor(hex: "#82aaff"),
        green: NSColor(hex: "#c3e88d"),
        magenta: NSColor(hex: "#c792ea"),
        red: NSColor(hex: "#f07178"),
        yellow: NSColor(hex: "#ffcb6b"),
        orange: NSColor(hex: "#f78c6c"),
        comment: NSColor(hex: "#676e95"),
        base: "vs-dark",
        selection: "#82aaff35",
        terminalPalette: [
            "#1b1e2b", "#f07178", "#c3e88d", "#ffcb6b", "#82aaff", "#c792ea", "#89ddff", "#676e95",
            "#676e95", "#f07178", "#c3e88d", "#ffcb6b", "#82aaff", "#c792ea", "#89ddff", "#a6accd",
        ].map { NSColor(hex: $0) }
    )

    // MARK: Solarized Light

    static let solarizedLight = Theme(
        name: "solarized-light",
        bg: NSColor(hex: "#fdf6e3"),
        bgDark: NSColor(hex: "#eee8d5"),
        bgHighlight: NSColor(hex: "#eee8d5"),
        fg: NSColor(hex: "#657b83"),
        fgDark: NSColor(hex: "#576464"),
        cyan: NSColor(hex: "#217e77"),
        blue: NSColor(hex: "#1d6da3"),
        green: NSColor(hex: "#859900"),
        magenta: NSColor(hex: "#d33682"),
        red: NSColor(hex: "#dc322f"),
        yellow: NSColor(hex: "#b58900"),
        orange: NSColor(hex: "#cb4b16"),
        comment: NSColor(hex: "#93a1a1"),
        base: "vs",
        selection: "#268bd230",
        terminalPalette: [
            "#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198", "#eee8d5",
            "#002b36", "#cb4b16", "#586e75", "#657b83", "#839496", "#6c71c4", "#93a1a1", "#fdf6e3",
        ].map { NSColor(hex: $0) }
    )

    // MARK: Catppuccin Latte

    static let catppuccinLatte = Theme(
        name: "catppuccin-latte",
        bg: NSColor(hex: "#eff1f5"),
        bgDark: NSColor(hex: "#e6e9ef"),
        bgHighlight: NSColor(hex: "#dce0e8"),
        fg: NSColor(hex: "#4c4f69"),
        fgDark: NSColor(hex: "#65677c"),
        cyan: NSColor(hex: "#137a80"),
        blue: NSColor(hex: "#1559de"),
        green: NSColor(hex: "#40a02b"),
        magenta: NSColor(hex: "#8839ef"),
        red: NSColor(hex: "#d20f39"),
        yellow: NSColor(hex: "#df8e1d"),
        orange: NSColor(hex: "#fe640b"),
        comment: NSColor(hex: "#9ca0b0"),
        base: "vs",
        selection: "#1e66f525",
        terminalPalette: [
            "#5c5f77", "#d20f39", "#40a02b", "#df8e1d", "#1e66f5", "#8839ef", "#179299", "#acb0be",
            "#6c6f85", "#d20f39", "#40a02b", "#df8e1d", "#1e66f5", "#8839ef", "#179299", "#4c4f69",
        ].map { NSColor(hex: $0) }
    )

    // MARK: GitHub Light

    static let githubLight = Theme(
        name: "github-light",
        bg: NSColor(hex: "#ffffff"),
        bgDark: NSColor(hex: "#f6f8fa"),
        bgHighlight: NSColor(hex: "#f0f2f4"),
        fg: NSColor(hex: "#1f2328"),
        fgDark: NSColor(hex: "#656d76"),
        cyan: NSColor(hex: "#0a3069"),
        blue: NSColor(hex: "#0969da"),
        green: NSColor(hex: "#1a7f37"),
        magenta: NSColor(hex: "#8250df"),
        red: NSColor(hex: "#cf222e"),
        yellow: NSColor(hex: "#9a6700"),
        orange: NSColor(hex: "#bc4c00"),
        comment: NSColor(hex: "#6e7781"),
        base: "vs",
        selection: "#0969da25",
        terminalPalette: [
            "#24292f", "#cf222e", "#1a7f37", "#9a6700", "#0969da", "#8250df", "#0a3069", "#6e7781",
            "#57606a", "#a40e26", "#2da44e", "#bf8700", "#218bff", "#a475f9", "#0a3069", "#1f2328",
        ].map { NSColor(hex: $0) }
    )
}

// MARK: - Monaco Theme Generation

extension Theme {
    /// Generates a `MonacoThemeDefinition` from this theme's colors, suitable
    /// for sending to the Monaco WebView via `EditorCommand.setTheme`.
    func monacoThemeDefinition() -> MonacoThemeDefinition {
        MonacoThemeDefinition(
            base: base,
            inherit: true,
            rules: [
                // Comments (italic)
                MonacoTokenRule(token: "comment", foreground: commentHex, fontStyle: "italic"),
                MonacoTokenRule(token: "comment.doc", foreground: commentHex, fontStyle: "italic"),
                // Keywords (magenta)
                MonacoTokenRule(token: "keyword", foreground: magentaHex),
                MonacoTokenRule(token: "keyword.control", foreground: magentaHex),
                MonacoTokenRule(token: "keyword.declaration", foreground: magentaHex),
                MonacoTokenRule(token: "keyword.type", foreground: magentaHex),
                MonacoTokenRule(token: "keyword.other", foreground: magentaHex),
                MonacoTokenRule(token: "keyword.flow", foreground: magentaHex),
                MonacoTokenRule(token: "keyword.block", foreground: magentaHex),
                MonacoTokenRule(token: "keyword.try", foreground: magentaHex),
                MonacoTokenRule(token: "keyword.catch", foreground: magentaHex),
                MonacoTokenRule(token: "keyword.choice", foreground: magentaHex),
                MonacoTokenRule(token: "keyword.modifier", foreground: magentaHex),
                // Constants & numbers (orange)
                MonacoTokenRule(token: "keyword.constant", foreground: orangeHex),
                MonacoTokenRule(token: "number", foreground: orangeHex),
                MonacoTokenRule(token: "number.hex", foreground: orangeHex),
                MonacoTokenRule(token: "number.float", foreground: orangeHex),
                MonacoTokenRule(token: "number.binary", foreground: orangeHex),
                MonacoTokenRule(token: "number.octal", foreground: orangeHex),
                MonacoTokenRule(token: "constant", foreground: orangeHex),
                MonacoTokenRule(token: "string.escape", foreground: orangeHex),
                // Strings (green)
                MonacoTokenRule(token: "string", foreground: greenHex),
                MonacoTokenRule(token: "string.heredoc", foreground: greenHex),
                MonacoTokenRule(token: "string.raw", foreground: greenHex),
                MonacoTokenRule(token: "attribute.value", foreground: greenHex),
                // Operators, special strings, predefined (cyan)
                MonacoTokenRule(token: "string.key", foreground: cyanHex),
                MonacoTokenRule(token: "string.link", foreground: cyanHex),
                MonacoTokenRule(token: "operator", foreground: cyanHex),
                MonacoTokenRule(token: "keyword.operator", foreground: cyanHex),
                MonacoTokenRule(token: "variable.predefined", foreground: cyanHex),
                MonacoTokenRule(token: "predefined", foreground: cyanHex),
                // Types, classes, annotations (yellow)
                MonacoTokenRule(token: "type", foreground: yellowHex),
                MonacoTokenRule(token: "type.identifier", foreground: yellowHex),
                MonacoTokenRule(token: "class", foreground: yellowHex),
                MonacoTokenRule(token: "annotation", foreground: yellowHex),
                MonacoTokenRule(token: "namespace", foreground: yellowHex),
                MonacoTokenRule(token: "constructor", foreground: yellowHex),
                MonacoTokenRule(token: "attribute.name", foreground: yellowHex),
                // Functions (blue)
                MonacoTokenRule(token: "function", foreground: blueHex),
                MonacoTokenRule(token: "function.declaration", foreground: blueHex),
                MonacoTokenRule(token: "function.call", foreground: blueHex),
                MonacoTokenRule(token: "predefined.function", foreground: blueHex),
                // Tags, invalid, regexp (red)
                MonacoTokenRule(token: "string.escape.invalid", foreground: redHex),
                MonacoTokenRule(token: "string.invalid", foreground: redHex),
                MonacoTokenRule(token: "regexp", foreground: redHex),
                MonacoTokenRule(token: "tag", foreground: redHex),
                MonacoTokenRule(token: "metatag", foreground: redHex),
                MonacoTokenRule(token: "invalid", foreground: redHex),
                // Variables, emphasis (fg)
                MonacoTokenRule(token: "variable", foreground: fgHex),
                MonacoTokenRule(token: "emphasis", foreground: fgHex, fontStyle: "italic"),
                // Delimiters (fg_dark)
                MonacoTokenRule(token: "delimiter", foreground: fgDarkHex),
                // Strong (orange + bold)
                MonacoTokenRule(token: "strong", foreground: orangeHex, fontStyle: "bold"),
            ],
            colors: MonacoThemeColors(
                editorBackground: bgHex,
                editorForeground: fgHex,
                editorLineHighlightBackground: bgHighlightHex,
                editorSelectionBackground: selection,
                editorCursorForeground: cyanHex,
                editorLineNumberForeground: commentHex,
                editorLineNumberActiveForeground: fgHex,
                editorWidgetBackground: bgDarkHex,
                editorSuggestWidgetBackground: bgDarkHex,
                editorSuggestWidgetSelectedBackground: bgHighlightHex,
                editorHoverWidgetBackground: bgDarkHex,
                editorGutterBackground: bgHex,
                minimapBackground: bgDarkHex,
                scrollbarSliderBackground: commentHex + "40",
                scrollbarSliderHoverBackground: commentHex + "80",
                diffAddedColor: greenHex,
                diffModifiedColor: yellowHex,
                diffDeletedColor: redHex
            )
        )
    }
}
