import AppKit
import SwiftUI

// SwiftUI sheets (New Task, Branches, Agent Hooks). The sheet window takes
// its content's size and follows it (an error line appearing grows it), and
// is painted in the theme's overlay color with the theme's light/dark
// appearance, so no system window background shows around the content.

extension NSWindow {
  /// An empty sheet window themed for `palette`. Made before its content so
  /// the content's callbacks can close it.
  static func themedSheet(palette: ChromePalette) -> NSWindow {
    let sheet = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 480, height: 320), styleMask: [.titled],
      backing: .buffered, defer: true)
    sheet.backgroundColor = palette.nsOverlay
    sheet.appearance = NSAppearance(named: palette.isLight ? .aqua : .darkAqua)
    return sheet
  }

  /// Show `content` in `sheet` (from `themedSheet`) as a sheet on this window.
  func beginThemedSheet<Content: View>(_ sheet: NSWindow, palette: ChromePalette, content: Content) {
    let host = NSHostingView(rootView: content.environment(\.chrome, palette))
    // The default sizing options (min, intrinsic and max size) pin the window
    // to the content; `.preferredContentSize` alone left it at its initial
    // size with the content centered in it.
    sheet.contentView = host
    sheet.setContentSize(host.fittingSize)
    beginSheet(sheet)
  }
}
