import AppKit
import SwiftUI

/// Renders vendored Lucide SVGs as template images (tinted by the current
/// foreground style). Images are cached per icon + stroke width; SVG images
/// are vector-backed, so one cached image scales crisply to any size.
enum LucideImageCache {
  private static var cache: [String: NSImage] = [:]
  private static let lock = NSLock()

  /// Lucide draws at stroke 2 on a 24pt grid; at 12–16pt that reads heavy next
  /// to text, so chrome icons default to a lighter stroke.
  static let defaultStrokeWidth: CGFloat = 1.75

  static func image(_ icon: LucideIcon, strokeWidth: CGFloat = defaultStrokeWidth) -> NSImage {
    let key = "\(icon.rawValue)@\(strokeWidth)"
    lock.lock()
    defer { lock.unlock() }
    if let cached = cache[key] { return cached }

    let svg = icon.svg
      .replacingOccurrences(of: "stroke-width=\"2\"", with: "stroke-width=\"\(strokeWidth)\"")
      .replacingOccurrences(of: "currentColor", with: "#000000")
    let image = NSImage(data: Data(svg.utf8)) ?? NSImage(size: NSSize(width: 24, height: 24))
    image.isTemplate = true
    image.accessibilityDescription = icon.rawValue
    cache[key] = image
    return image
  }
}

/// A Lucide icon sized for chrome. Color comes from `.foregroundStyle`.
struct Icon: View {
  let icon: LucideIcon
  var size: CGFloat = Metrics.icon
  var strokeWidth: CGFloat = LucideImageCache.defaultStrokeWidth

  init(_ icon: LucideIcon, size: CGFloat = Metrics.icon, strokeWidth: CGFloat? = nil) {
    self.icon = icon
    self.size = size
    self.strokeWidth = strokeWidth ?? LucideImageCache.defaultStrokeWidth
  }

  var body: some View {
    Image(nsImage: LucideImageCache.image(icon, strokeWidth: strokeWidth))
      .renderingMode(.template)
      .resizable()
      .interpolation(.high)
      .frame(width: size, height: size)
      .accessibilityHidden(true)
  }
}

extension NSImage {
  /// A Lucide icon as an AppKit template image of the given point size.
  static func lucide(_ icon: LucideIcon, size: CGFloat = Metrics.icon) -> NSImage {
    let base = LucideImageCache.image(icon)
    let copy = base.copy() as! NSImage
    copy.size = NSSize(width: size, height: size)
    copy.isTemplate = true
    return copy
  }
}
