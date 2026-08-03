import Foundation
import TOMLKit

// Ported from the theme resolution + registry half of impulse-core/src/theme.rs:
// `parse_theme`, `resolve_theme`, `builtin_theme_names`, `builtin_theme`,
// `discover_user_themes`, `load_user_theme`, `available_themes`, `get_theme`,
// `theme_display_name`, and the HSL color math helpers.

public struct ThemeError: Error, CustomStringConvertible {
  public let message: String
  public var description: String { message }
}

public enum ThemeStore {
  // MARK: - Parsing & resolution

  /// Parse a TOML theme string into a `ThemeFile`.
  public static func parseTheme(_ tomlString: String) throws -> ThemeFile {
    do {
      return try TOMLDecoder().decode(ThemeFile.self, from: tomlString)
    } catch {
      throw ThemeError(message: "Failed to parse theme TOML: \(error)")
    }
  }

  /// Resolve a parsed `ThemeFile` into a fully-populated `ResolvedTheme`.
  /// Fills in all defaults and derived values from the palette. The `id`
  /// parameter is the lookup key (e.g. `"rose-pine"`, not the display name).
  public static func resolveTheme(id: String, file tf: ThemeFile) -> ResolvedTheme {
    let p = tf.palette
    let isLight = tf.variant == "light"

    // Derive background layers
    let bgSurface =
      tf.ui.bgSurface ?? p.surface
      ?? (isLight ? shiftLightness(p.bg, 0.08) : shiftLightness(p.bg, -0.10))
    let bgDark =
      tf.ui.bgDark
      ?? (isLight ? shiftLightness(p.bg, 0.04) : shiftLightness(p.bg, -0.05))
    let bgHighlight =
      tf.ui.bgHighlight
      ?? (isLight ? shiftLightness(p.bg, -0.05) : shiftLightness(p.bg, 0.08))
    let border =
      tf.ui.border ?? p.overlay
      ?? (isLight ? shiftLightness(p.bg, -0.08) : shiftLightness(p.bg, 0.04))

    // Derive foreground shades
    let fgMuted = tf.ui.fgMuted ?? p.muted ?? muteColor(p.fg, saturationFactor: 0.6, lightnessTarget: 0.55)
    let fgComment =
      tf.ui.fgComment ?? p.subtle
      ?? muteColor(p.fg, saturationFactor: 0.4, lightnessTarget: isLight ? 0.60 : 0.40)

    // Selection & cursor
    let selection = tf.ui.selection ?? "\(p.accent)40"
    let cursor = tf.ui.cursor ?? p.accent

    // Git indicators
    let gitAdded = tf.ui.gitAdded ?? p.green
    let gitModified = tf.ui.gitModified ?? p.yellow
    let gitDeleted = tf.ui.gitDeleted ?? p.red
    let gitRenamed = tf.ui.gitRenamed ?? p.blue
    let gitConflict = tf.ui.gitConflict ?? p.orange
    let gitIgnored = tf.ui.gitIgnored ?? fgMuted

    // Syntax colors
    let syntaxComment = tf.syntax.comment ?? fgComment

    // Terminal palette
    let terminalPalette: [String]
    if let tp = tf.terminal {
      terminalPalette = [
        tp.black, tp.red, tp.green, tp.yellow,
        tp.blue, tp.magenta, tp.cyan, tp.white,
        tp.brightBlack, tp.brightRed, tp.brightGreen, tp.brightYellow,
        tp.brightBlue, tp.brightMagenta, tp.brightCyan, tp.brightWhite,
      ]
    } else {
      // Derive terminal palette from theme palette
      terminalPalette = [
        shiftLightness(p.bg, 0.10),
        p.red, p.green, p.yellow, p.blue, p.magenta, p.cyan,
        fgMuted,
        fgComment,
        p.red, p.green, p.yellow, p.blue, p.magenta, p.cyan,
        p.fg,
      ]
    }

    let surfaceStyle: String
    if let s = tf.ui.surfaceStyle, s == "card" || s == "flat" {
      surfaceStyle = s
    } else {
      surfaceStyle = "flat"
    }

    return ResolvedTheme(
      id: id,
      name: tf.name,
      isLight: isLight,
      bg: p.bg,
      bgDark: bgDark,
      bgHighlight: bgHighlight,
      bgSurface: bgSurface,
      border: border,
      fg: p.fg,
      fgMuted: fgMuted,
      fgComment: fgComment,
      accent: p.accent,
      selection: selection,
      cursor: cursor,
      red: p.red,
      orange: p.orange,
      yellow: p.yellow,
      green: p.green,
      cyan: p.cyan,
      blue: p.blue,
      magenta: p.magenta,
      gitAdded: gitAdded,
      gitModified: gitModified,
      gitDeleted: gitDeleted,
      gitRenamed: gitRenamed,
      gitConflict: gitConflict,
      gitIgnored: gitIgnored,
      syntaxKeyword: tf.syntax.keyword ?? p.magenta,
      syntaxFunction: tf.syntax.function ?? p.blue,
      syntaxType: tf.syntax.type ?? p.yellow,
      syntaxString: tf.syntax.string ?? p.green,
      syntaxNumber: tf.syntax.number ?? p.orange,
      syntaxConstant: tf.syntax.constant ?? p.orange,
      syntaxComment: syntaxComment,
      syntaxOperator: tf.syntax.operator ?? p.cyan,
      syntaxTag: tf.syntax.tag ?? p.red,
      syntaxAttribute: tf.syntax.attribute ?? p.yellow,
      syntaxVariable: tf.syntax.variable ?? p.fg,
      syntaxDelimiter: tf.syntax.delimiter ?? fgMuted,
      syntaxEscape: tf.syntax.escape ?? p.orange,
      syntaxRegexp: tf.syntax.regexp ?? p.red,
      syntaxLink: tf.syntax.link ?? p.blue,
      terminalFg: p.fg,
      terminalBg: p.bg,
      terminalPalette: terminalPalette,
      surfaceStyle: surfaceStyle
    )
  }

  // MARK: - Built-in themes

  /// Built-in theme IDs in display order (mirrors `BUILTIN_THEMES` in Rust).
  private static let builtinIDs: [String] = [
    "kanagawa",
    "rose-pine",
    "nord",
    "gruvbox",
    "tokyo-night",
    "tokyo-night-storm",
    "catppuccin-mocha",
    "dracula",
    "solarized-dark",
    "one-dark",
    "ayu-dark",
    "everforest-dark",
    "github-dark",
    "monokai-pro",
    "palenight",
    "solarized-light",
    "catppuccin-latte",
    "github-light",
    "harbor",
  ]

  private static let cacheLock = NSLock()
  nonisolated(unsafe) private static var builtinCache: [String: ResolvedTheme] = [:]

  /// Return the list of built-in theme IDs in display order.
  public static func builtinThemeNames() -> [String] {
    builtinIDs
  }

  /// Load and resolve a built-in theme by ID.
  public static func builtinTheme(_ name: String) -> ResolvedTheme? {
    let normalized = normalizeThemeID(name)
    guard builtinIDs.contains(normalized) else { return nil }

    cacheLock.lock()
    defer { cacheLock.unlock() }
    if let cached = builtinCache[normalized] {
      return cached
    }

    guard
      let url = Bundle.module.url(
        forResource: normalized, withExtension: "toml", subdirectory: "Resources/Themes"),
      let contents = try? String(contentsOf: url, encoding: .utf8)
    else {
      return nil
    }
    guard let tf = try? parseTheme(contents) else {
      return nil
    }
    let resolved = resolveTheme(id: normalized, file: tf)
    builtinCache[normalized] = resolved
    return resolved
  }

  // MARK: - User themes

  /// Discover user themes from `~/Library/Application Support/impulse/themes`.
  /// Returns `(theme_id, file_path)` pairs. The theme ID is the filename stem.
  public static func discoverUserThemes() -> [(id: String, path: URL)] {
    let dir = userThemesDir()
    var isDir: ObjCBool = false
    guard FileManager.default.fileExists(atPath: dir.path, isDirectory: &isDir),
      isDir.boolValue
    else {
      return []
    }

    var themes: [(id: String, path: URL)] = []
    if let entries = try? FileManager.default.contentsOfDirectory(
      at: dir, includingPropertiesForKeys: nil)
    {
      for path in entries where path.pathExtension == "toml" {
        let stem = path.deletingPathExtension().lastPathComponent
        themes.append((id: stem, path: path))
      }
    }
    themes.sort { $0.id < $1.id }
    return themes
  }

  /// Load a user theme from a file path.
  /// The theme ID is derived from the filename stem (`my-theme.toml` → `"my-theme"`).
  public static func loadUserTheme(at path: URL) throws -> ResolvedTheme {
    let stem = path.deletingPathExtension().lastPathComponent
    let id = stem.isEmpty ? "custom" : stem
    let contents: String
    do {
      contents = try String(contentsOf: path, encoding: .utf8)
    } catch {
      throw ThemeError(message: "Failed to read theme file: \(error)")
    }
    let tf = try parseTheme(contents)
    return resolveTheme(id: id, file: tf)
  }

  /// Return all available theme names: built-in first, then user themes.
  public static func availableThemes() -> [String] {
    var names = builtinThemeNames()
    for (id, _) in discoverUserThemes() where !names.contains(id) {
      names.append(id)
    }
    return names
  }

  /// Resolve a theme by name. Checks user themes first (allows overrides),
  /// then built-in themes, then falls back to Nord.
  public static func getTheme(_ name: String) -> ResolvedTheme {
    let normalized = normalizeThemeID(name)

    // Check user themes first
    for (id, path) in discoverUserThemes() where id == normalized {
      if let theme = try? loadUserTheme(at: path) {
        return theme
      }
      break
    }

    // Built-in themes
    if let theme = builtinTheme(normalized) {
      return theme
    }

    // Fallback
    guard let nord = builtinTheme("nord") else {
      fatalError("Nord theme must always be available")
    }
    return nord
  }

  /// Convert a theme ID like `"tokyo-night-storm"` to a display name like
  /// `"Tokyo Night Storm"`.
  public static func themeDisplayName(_ id: String) -> String {
    switch id {
    case "rose-pine": return "Rosé Pine"
    case "catppuccin-mocha": return "Catppuccin Mocha"
    case "catppuccin-latte": return "Catppuccin Latte"
    case "github-dark": return "GitHub Dark"
    case "github-light": return "GitHub Light"
    case "monokai-pro": return "Monokai Pro"
    default:
      return
        id
        .split(separator: "-", omittingEmptySubsequences: false)
        .map { word -> String in
          guard let first = word.first else { return "" }
          return String(first).uppercased() + word.dropFirst()
        }
        .joined(separator: " ")
    }
  }

  // MARK: - Internals

  /// Normalize theme name variants (underscore, no-separator) to canonical
  /// kebab-case ID.
  static func normalizeThemeID(_ name: String) -> String {
    let lower = name.lowercased()
    switch lower {
    case "rose_pine", "rosepine": return "rose-pine"
    case "gruvbox_dark", "gruvbox-dark": return "gruvbox"
    case "tokyo_night", "tokyonight": return "tokyo-night"
    case "tokyo_night_storm", "tokyonightstorm", "tokyo-night-storm": return "tokyo-night-storm"
    case "catppuccin_mocha", "catppuccinmocha": return "catppuccin-mocha"
    case "solarized_dark", "solarizeddark": return "solarized-dark"
    case "one_dark", "onedark": return "one-dark"
    case "ayu_dark", "ayudark": return "ayu-dark"
    case "everforest_dark", "everforestdark": return "everforest-dark"
    case "github_dark", "githubdark": return "github-dark"
    case "monokai_pro", "monokaipro": return "monokai-pro"
    case "solarized_light", "solarizedlight": return "solarized-light"
    case "catppuccin_latte", "catppuccinlatte": return "catppuccin-latte"
    case "github_light", "githublight": return "github-light"
    default: return lower
    }
  }

  static func userThemesDir() -> URL {
    FileManager.default.homeDirectoryForCurrentUser
      .appendingPathComponent("Library/Application Support/impulse/themes")
  }
}

// MARK: - HSL color math (ported verbatim from theme.rs)

struct Hsl {
  var h: Double
  var s: Double
  var l: Double
}

func hexToRgb(_ hex: String) -> (UInt8, UInt8, UInt8) {
  let stripped = hex.drop(while: { $0 == "#" })
  func component(_ offset: Int) -> UInt8 {
    guard stripped.count >= offset + 2 else { return 0 }
    let start = stripped.index(stripped.startIndex, offsetBy: offset)
    let end = stripped.index(start, offsetBy: 2)
    return UInt8(stripped[start..<end], radix: 16) ?? 0
  }
  return (component(0), component(2), component(4))
}

func rgbToHsl(_ r8: UInt8, _ g8: UInt8, _ b8: UInt8) -> Hsl {
  let r = Double(r8) / 255.0
  let g = Double(g8) / 255.0
  let b = Double(b8) / 255.0
  let maxC = max(r, g, b)
  let minC = min(r, g, b)
  let l = (maxC + minC) / 2.0

  if abs(maxC - minC) < .ulpOfOne {
    return Hsl(h: 0.0, s: 0.0, l: l)
  }

  let d = maxC - minC
  let s = l > 0.5 ? d / (2.0 - maxC - minC) : d / (maxC + minC)

  let h: Double
  if abs(maxC - r) < .ulpOfOne {
    var hh = (g - b) / d
    if g < b {
      hh += 6.0
    }
    h = hh
  } else if abs(maxC - g) < .ulpOfOne {
    h = (b - r) / d + 2.0
  } else {
    h = (r - g) / d + 4.0
  }

  return Hsl(h: h * 60.0, s: s, l: l)
}

func hslToRgb(_ hsl: Hsl) -> (UInt8, UInt8, UInt8) {
  func toByte(_ v: Double) -> UInt8 {
    // Mirrors Rust's saturating `as u8` cast of a rounded f64.
    UInt8(min(255.0, max(0.0, v.rounded())))
  }

  if abs(hsl.s) < .ulpOfOne {
    let v = toByte(hsl.l * 255.0)
    return (v, v, v)
  }

  let q = hsl.l < 0.5 ? hsl.l * (1.0 + hsl.s) : hsl.l + hsl.s - hsl.l * hsl.s
  let p = 2.0 * hsl.l - q
  let h = hsl.h / 360.0

  func hueToRgb(_ p: Double, _ q: Double, _ t0: Double) -> Double {
    var t = t0
    if t < 0.0 {
      t += 1.0
    }
    if t > 1.0 {
      t -= 1.0
    }
    if t < 1.0 / 6.0 {
      return p + (q - p) * 6.0 * t
    }
    if t < 1.0 / 2.0 {
      return q
    }
    if t < 2.0 / 3.0 {
      return p + (q - p) * (2.0 / 3.0 - t) * 6.0
    }
    return p
  }

  let r = toByte(hueToRgb(p, q, h + 1.0 / 3.0) * 255.0)
  let g = toByte(hueToRgb(p, q, h) * 255.0)
  let b = toByte(hueToRgb(p, q, h - 1.0 / 3.0) * 255.0)
  return (r, g, b)
}

func rgbToHex(_ r: UInt8, _ g: UInt8, _ b: UInt8) -> String {
  String(format: "#%02x%02x%02x", Int(r), Int(g), Int(b))
}

/// Shift the lightness of a hex color by `delta` (range: -1.0 to 1.0).
func shiftLightness(_ hex: String, _ delta: Double) -> String {
  let (r, g, b) = hexToRgb(hex)
  var hsl = rgbToHsl(r, g, b)
  hsl.l = min(1.0, max(0.0, hsl.l + delta))
  let (r2, g2, b2) = hslToRgb(hsl)
  return rgbToHex(r2, g2, b2)
}

/// Desaturate/mute a color by reducing saturation and shifting lightness
/// toward a target.
func muteColor(_ hex: String, saturationFactor: Double, lightnessTarget: Double) -> String {
  let (r, g, b) = hexToRgb(hex)
  var hsl = rgbToHsl(r, g, b)
  hsl.s *= saturationFactor
  hsl.l = hsl.l + (lightnessTarget - hsl.l) * 0.3
  let (r2, g2, b2) = hslToRgb(hsl)
  return rgbToHex(r2, g2, b2)
}
