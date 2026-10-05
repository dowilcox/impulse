import AppKit
import SwiftUI

/// Factory for the workbench's SwiftUI hosting views.
enum WorkbenchHosting {
  /// A hosting view sized entirely by AppKit constraints: it reports no
  /// SwiftUI min/max size (which would clamp the window) and ignores safe-area
  /// insets (the transparent titlebar band would otherwise push content down).
  /// With `intrinsicHeight`, it still reports its content's height (banner,
  /// input bar) but stays horizontally flexible.
  static func make<Content: View>(_ view: Content, intrinsicHeight: Bool = false) -> NSView {
    let host = intrinsicHeight ? FlexibleWidthHostingView(rootView: view) : NSHostingView(rootView: view)
    host.sizingOptions = intrinsicHeight ? [.intrinsicContentSize] : []
    host.safeAreaRegions = []
    return host
  }
}

/// Reports only an intrinsic height; width always comes from constraints, so
/// wide content can't push its container (or the window) wider.
private final class FlexibleWidthHostingView<Content: View>: NSHostingView<Content> {
  override var intrinsicContentSize: NSSize {
    NSSize(width: NSView.noIntrinsicMetric, height: super.intrinsicContentSize.height)
  }
}
