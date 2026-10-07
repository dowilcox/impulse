import AppKit
import SwiftUI

// MARK: - Chrome palette

/// Colors for Impulse's own chrome (titlebar, docks, panels, status bar),
/// derived from the active theme's seeds so every built-in and user theme gets
/// a coherent workbench without extra theme keys:
///
/// - chrome surfaces sit a step darker (dark themes) or lighter (light themes)
///   than the editor/terminal background, so content leads and chrome recedes;
/// - interactive states are tints of the foreground, not system materials;
/// - status colors come from the theme's ANSI-ish palette, separate from the
///   accent.
struct ChromePalette {
  // Surfaces
  let window: Color
  let chrome: Color
  let panel: Color
  let content: Color
  let raised: Color
  let overlay: Color
  let hairline: Color
  let hairlineStrong: Color

  // Interaction
  let hover: Color
  let pressed: Color
  let selection: Color
  let selectionStrong: Color
  let focusRing: Color

  // Text
  let text: Color
  let textSecondary: Color
  let textTertiary: Color
  let textOnAccent: Color

  // Accent + semantic
  let accent: Color
  let accentSoft: Color
  let success: Color
  let warning: Color
  let danger: Color
  let info: Color
  let working: Color
  let attention: Color

  // Git
  let gitAdded: Color
  let gitModified: Color
  let gitDeleted: Color
  let gitRenamed: Color
  let gitConflict: Color
  let gitUntracked: Color

  // AppKit counterparts for views drawn outside SwiftUI.
  let nsChrome: NSColor
  let nsPanel: NSColor
  let nsContent: NSColor
  let nsOverlay: NSColor
  let nsHairline: NSColor
  let nsText: NSColor
  let nsTextSecondary: NSColor
  let nsAccent: NSColor

  let isLight: Bool

  /// System Settings ▸ Accessibility ▸ Display ▸ Increase contrast: firmer
  /// edges, stronger selection, and secondary text closer to the foreground.
  /// (The chrome uses no translucent materials, so Reduce Transparency has
  /// nothing to turn off.)
  init(theme: Theme, increaseContrast: Bool = NSWorkspace.shared.accessibilityDisplayShouldIncreaseContrast) {
    let bg = NSColor(hex: theme.bg)
    let fg = NSColor(hex: theme.fg)
    let isLight = theme.isLight
    self.isLight = isLight
    let boost: CGFloat = increaseContrast ? 1 : 0

    // A chrome surface a little "further back" than content.
    let chromeNS = Self.mix(bg, toward: isLight ? .black : .black, amount: isLight ? 0.035 : 0.22)
    let panelNS = Self.mix(bg, toward: .black, amount: isLight ? 0.02 : 0.14)
    let raisedNS = Self.mix(bg, toward: fg, amount: isLight ? 0.06 : 0.07)
    let overlayNS = Self.mix(bg, toward: fg, amount: isLight ? 0.02 : 0.05)
    let hairlineNS = Self.mix(bg, toward: fg, amount: 0.12 + 0.2 * boost)
    let hairlineStrongNS = Self.mix(bg, toward: fg, amount: 0.2 + 0.25 * boost)
    let accentNS = NSColor(hex: theme.accent)

    window = Color(nsColor: chromeNS)
    chrome = Color(nsColor: chromeNS)
    panel = Color(nsColor: panelNS)
    content = Color(nsColor: bg)
    raised = Color(nsColor: raisedNS)
    overlay = Color(nsColor: overlayNS)
    hairline = Color(nsColor: hairlineNS)
    hairlineStrong = Color(nsColor: hairlineStrongNS)

    hover = Color(nsColor: fg).opacity((isLight ? 0.06 : 0.07) + 0.06 * boost)
    pressed = Color(nsColor: fg).opacity((isLight ? 0.1 : 0.12) + 0.08 * boost)
    selection = Color(nsColor: accentNS).opacity((isLight ? 0.14 : 0.18) + 0.14 * boost)
    selectionStrong = Color(nsColor: accentNS).opacity((isLight ? 0.24 : 0.3) + 0.15 * boost)
    focusRing = Color(nsColor: accentNS).opacity(0.75 + 0.25 * boost)

    let muted = NSColor(hex: theme.fgMuted)
    let comment = NSColor(hex: theme.fgComment)
    text = Color(nsColor: fg)
    textSecondary = Color(nsColor: increaseContrast ? Self.mix(muted, toward: fg, amount: 0.5) : muted)
    textTertiary = Color(nsColor: increaseContrast ? muted : comment)
    textOnAccent = Color(nsColor: Self.readableText(on: accentNS))

    accent = Color(nsColor: accentNS)
    accentSoft = Color(nsColor: accentNS).opacity(0.16 + 0.12 * boost)
    success = Color(nsColor: NSColor(hex: theme.green))
    warning = Color(nsColor: NSColor(hex: theme.yellow))
    danger = Color(nsColor: NSColor(hex: theme.red))
    info = Color(nsColor: NSColor(hex: theme.blue))
    working = Color(nsColor: NSColor(hex: theme.magenta))
    attention = Color(nsColor: NSColor(hex: theme.orange))

    gitAdded = Color(nsColor: NSColor(hex: theme.gitAdded))
    gitModified = Color(nsColor: NSColor(hex: theme.gitModified))
    gitDeleted = Color(nsColor: NSColor(hex: theme.gitDeleted))
    gitRenamed = Color(nsColor: NSColor(hex: theme.gitRenamed))
    gitConflict = Color(nsColor: NSColor(hex: theme.gitConflict))
    gitUntracked = Color(nsColor: NSColor(hex: theme.gitAdded)).opacity(0.8)

    nsChrome = chromeNS
    nsPanel = panelNS
    nsContent = bg
    nsOverlay = overlayNS
    nsHairline = hairlineNS
    nsText = fg
    nsTextSecondary = increaseContrast ? Self.mix(muted, toward: fg, amount: 0.5) : muted
    nsAccent = accentNS
  }

  /// Linear blend in sRGB.
  static func mix(_ base: NSColor, toward other: NSColor, amount: CGFloat) -> NSColor {
    guard let a = base.usingColorSpace(.sRGB), let b = other.usingColorSpace(.sRGB) else {
      return base
    }
    let t = max(0, min(1, amount))
    return NSColor(
      srgbRed: a.redComponent + (b.redComponent - a.redComponent) * t,
      green: a.greenComponent + (b.greenComponent - a.greenComponent) * t,
      blue: a.blueComponent + (b.blueComponent - a.blueComponent) * t,
      alpha: 1)
  }

  /// Black or white, whichever reads better on `color` (WCAG luminance).
  static func readableText(on color: NSColor) -> NSColor {
    guard let c = color.usingColorSpace(.sRGB) else { return .white }
    func lin(_ v: CGFloat) -> CGFloat {
      v <= 0.03928 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4)
    }
    let l = 0.2126 * lin(c.redComponent) + 0.7152 * lin(c.greenComponent)
      + 0.0722 * lin(c.blueComponent)
    return l > 0.4 ? NSColor(srgbRed: 0.08, green: 0.08, blue: 0.1, alpha: 1) : .white
  }
}

// MARK: - Metrics

/// Spacing, sizes and type for the chrome. One place so density stays even.
enum Metrics {
  /// Height of the titlebar band (traffic lights + tabs + actions).
  static let titlebarHeight: CGFloat = 40
  /// Leading inset reserved for the traffic lights (window not full screen).
  static let trafficLightInset: CGFloat = 78
  static let statusBarHeight: CGFloat = 24
  static let paneHeaderHeight: CGFloat = 28
  static let rowHeight: CGFloat = 24
  static let tabHeight: CGFloat = 28

  static let radiusSmall: CGFloat = 4
  static let radius: CGFloat = 6
  static let radiusLarge: CGFloat = 8

  static let iconSmall: CGFloat = 12
  static let icon: CGFloat = 14
  static let iconLarge: CGFloat = 16

  static let leftDockDefaultWidth: CGFloat = 260
  static let rightDockDefaultWidth: CGFloat = 420
  static let bottomDockDefaultHeight: CGFloat = 220
}

/// Type roles for the chrome. UI text uses the system font (SF Pro); data and
/// paths use the monospace font so columns of numbers and hashes line up.
enum ChromeFont {
  static func ui(_ size: CGFloat = 12, weight: Font.Weight = .regular) -> Font {
    .system(size: size, weight: weight)
  }

  static func mono(_ size: CGFloat = 11, weight: Font.Weight = .regular) -> Font {
    .system(size: size, weight: weight, design: .monospaced)
  }

  static let label = ui(11, weight: .semibold)
  static let body = ui(12)
  static let caption = ui(11)
}
