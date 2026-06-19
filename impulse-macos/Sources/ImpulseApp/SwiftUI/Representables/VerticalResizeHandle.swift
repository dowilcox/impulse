import AppKit
import SwiftUI

/// A horizontal drag handle used to vertically resize the view stacked above
/// it (e.g. the sidebar's tab section).
///
/// This is AppKit-backed on purpose. A SwiftUI `DragGesture` jitters here
/// because the handle itself moves as the section above it grows/shrinks —
/// each frame the pointer "catches up" to the relocated handle and the local
/// translation oscillates. AppKit reports the pointer in *window* coordinates
/// (stable regardless of the handle moving) and, during a drag, keeps routing
/// `mouseDragged` events to this view even when the pointer leaves its bounds.
/// It also gives a reliable resize cursor across the entire hit area via a
/// tracking area, instead of the fragile `NSCursor.push()/pop()` dance.
struct VerticalResizeHandle: NSViewRepresentable {
  /// Hover entered/exited — drives the visual highlight.
  var onHoverChanged: (Bool) -> Void
  /// Total drag delta in points since mouse-down, downward positive.
  var onChanged: (CGFloat) -> Void
  /// Mouse released — commit the live height.
  var onEnded: () -> Void
  /// Double-click — reset to the automatic height.
  var onReset: () -> Void

  func makeNSView(context: Context) -> HandleNSView {
    let view = HandleNSView()
    view.apply(
      onHoverChanged: onHoverChanged, onChanged: onChanged, onEnded: onEnded, onReset: onReset)
    return view
  }

  func updateNSView(_ nsView: HandleNSView, context: Context) {
    nsView.apply(
      onHoverChanged: onHoverChanged, onChanged: onChanged, onEnded: onEnded, onReset: onReset)
  }

  final class HandleNSView: NSView {
    private var onHoverChanged: ((Bool) -> Void)?
    private var onChanged: ((CGFloat) -> Void)?
    private var onEnded: (() -> Void)?
    private var onReset: (() -> Void)?
    private var dragStartY: CGFloat = 0

    func apply(
      onHoverChanged: @escaping (Bool) -> Void,
      onChanged: @escaping (CGFloat) -> Void,
      onEnded: @escaping () -> Void,
      onReset: @escaping () -> Void
    ) {
      self.onHoverChanged = onHoverChanged
      self.onChanged = onChanged
      self.onEnded = onEnded
      self.onReset = onReset
    }

    override func updateTrackingAreas() {
      super.updateTrackingAreas()
      trackingAreas.forEach(removeTrackingArea)
      addTrackingArea(
        NSTrackingArea(
          rect: .zero,
          options: [.inVisibleRect, .activeInActiveApp, .cursorUpdate, .mouseEnteredAndExited],
          owner: self,
          userInfo: nil
        )
      )
    }

    // Reliable resize cursor whenever the pointer is over the handle.
    override func cursorUpdate(with event: NSEvent) {
      NSCursor.resizeUpDown.set()
    }

    override func mouseEntered(with event: NSEvent) {
      onHoverChanged?(true)
    }

    override func mouseExited(with event: NSEvent) {
      onHoverChanged?(false)
    }

    override func mouseDown(with event: NSEvent) {
      if event.clickCount == 2 {
        onReset?()
        return
      }
      dragStartY = event.locationInWindow.y
      NSCursor.resizeUpDown.set()
    }

    override func mouseDragged(with event: NSEvent) {
      // Window coordinates: y grows upward, so dragging the pointer *down*
      // lowers y. Report downward drags as positive so the caller can add the
      // delta directly to the section height.
      NSCursor.resizeUpDown.set()
      onChanged?(dragStartY - event.locationInWindow.y)
    }

    override func mouseUp(with event: NSEvent) {
      onEnded?()
    }
  }
}
