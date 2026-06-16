import AppKit
import SwiftUI

/// A Warp-style path-completion dropdown rendered as a borderless, non-activating
/// child `NSPanel` floating above the terminal input bar.
///
/// Why a panel and not a SwiftUI `.popover`: the terminal grid is a pure AppKit
/// `NSView` embedded via `NSViewRepresentable`, and SwiftUI overlays do not
/// reliably compose above sibling AppKit views. A child `NSWindow` lives in the
/// window-server Z-order, so it is guaranteed to float above the terminal.
///
/// The panel is configured to NEVER become key or main (`canBecomeKey == false`),
/// so the input `TextField` in `TerminalContextBarView` keeps first-responder
/// status — ↑/↓/Tab/Enter/Esc continue to route to the field. Rows are still
/// mouse-clickable (a non-key window receives clicks); selection is reported
/// through `CompletionPopupView`'s `onSelect`.
final class CompletionPanel {

  /// Fixed content width — matches `CompletionPopupView`'s list width.
  static let contentWidth: CGFloat = CompletionPopupView.listWidth
  /// Outer padding around the hosted list, giving the dark rounded card a small
  /// inset margin and room for the border/shadow.
  private static let cornerRadius: CGFloat = 10

  private var panel: NSPanel?
  private var hostingView: NSHostingView<CompletionPopupView>?
  private weak var parentWindow: NSWindow?

  /// True while the panel is attached and visible.
  var isShown: Bool { panel?.isVisible ?? false }

  // MARK: - Lifecycle

  /// Shows (or updates) the panel hosting `content`, anchored so its BOTTOM edge
  /// sits `gap` points above `anchorScreenRect`'s top edge, left-aligned to the
  /// anchor's leading edge. `anchorScreenRect` is in screen coordinates (the
  /// input field's frame converted via the window).
  func show(
    content: CompletionPopupView,
    anchorScreenRect: NSRect,
    height: CGFloat,
    in parent: NSWindow,
    gap: CGFloat = 6
  ) {
    let panel = ensurePanel(in: parent)
    hostingView?.rootView = content

    let size = NSSize(width: Self.contentWidth, height: height)
    // Bottom of the panel = top of the anchor + gap; grow upward.
    let originX = anchorScreenRect.minX
    let originY = anchorScreenRect.maxY + gap
    let frame = NSRect(origin: NSPoint(x: originX, y: originY), size: size)
    panel.setFrame(frame, display: true)

    if panel.parent !== parent {
      panel.parent?.removeChildWindow(panel)
      parent.addChildWindow(panel, ordered: .above)
    }
    // orderFront (NOT makeKey) so the input field keeps first-responder status.
    panel.orderFront(nil)
  }

  /// Hides the panel without tearing it down (so it can be reused cheaply).
  func hide() {
    guard let panel else { return }
    panel.parent?.removeChildWindow(panel)
    panel.orderOut(nil)
  }

  // MARK: - Setup

  private func ensurePanel(in parent: NSWindow) -> NSPanel {
    if let panel {
      self.parentWindow = parent
      return panel
    }

    let hosting = NSHostingView(rootView: CompletionPopupView.empty)
    hosting.wantsLayer = true
    hosting.layer?.backgroundColor = NSColor.clear.cgColor
    hosting.layer?.cornerRadius = Self.cornerRadius
    hosting.layer?.masksToBounds = true

    // A container that draws the dark rounded card + subtle border behind the
    // hosted SwiftUI list (which itself paints `theme.colorBgDark`). The layer
    // mask clips the SwiftUI content to the rounded shape; the shadow lives on
    // the panel's own `hasShadow`.
    let container = NSView()
    container.wantsLayer = true
    container.layer?.cornerRadius = Self.cornerRadius
    container.layer?.masksToBounds = true
    container.layer?.borderWidth = 1
    container.layer?.borderColor = NSColor.white.withAlphaComponent(0.08).cgColor

    hosting.translatesAutoresizingMaskIntoConstraints = false
    container.addSubview(hosting)
    NSLayoutConstraint.activate([
      hosting.leadingAnchor.constraint(equalTo: container.leadingAnchor),
      hosting.trailingAnchor.constraint(equalTo: container.trailingAnchor),
      hosting.topAnchor.constraint(equalTo: container.topAnchor),
      hosting.bottomAnchor.constraint(equalTo: container.bottomAnchor),
    ])

    let panel = NonKeyPanel(
      contentRect: NSRect(x: 0, y: 0, width: Self.contentWidth, height: 120),
      styleMask: [.borderless, .nonactivatingPanel],
      backing: .buffered,
      defer: true
    )
    panel.isFloatingPanel = true
    panel.level = .floating
    panel.isOpaque = false
    panel.backgroundColor = .clear
    panel.hasShadow = true
    panel.hidesOnDeactivate = false
    panel.isReleasedWhenClosed = false
    panel.worksWhenModal = true
    panel.becomesKeyOnlyIfNeeded = true
    panel.contentView = container

    self.panel = panel
    self.hostingView = hosting
    self.parentWindow = parent
    return panel
  }
}

/// An `NSPanel` that refuses key/main status so the terminal input field always
/// keeps first-responder focus, even when the panel is clicked.
private final class NonKeyPanel: NSPanel {
  override var canBecomeKey: Bool { false }
  override var canBecomeMain: Bool { false }
}

extension CompletionPopupView {
  /// A placeholder instance used to seed the hosting view before the first real
  /// content is supplied.
  fileprivate static var empty: CompletionPopupView {
    CompletionPopupView(
      candidates: [],
      matchedPrefix: "",
      selectedIndex: nil,
      theme: ThemeManager.theme(forName: "nord"),
      iconCache: nil,
      onSelect: { _ in }
    )
  }
}

// MARK: - Window anchor

/// A zero-cost `NSViewRepresentable` that tracks its own frame in SCREEN
/// coordinates and the hosting `NSWindow`, reporting both through `onChange`.
///
/// `TerminalContextBarView` overlays this on the input field so the completion
/// panel can be positioned and re-anchored as the field moves (window resize /
/// move, layout changes). It is fully transparent and never intercepts events.
struct CompletionAnchorView: NSViewRepresentable {
  /// Called whenever the anchor's screen rect or window changes. `nil` window
  /// means the view is detached.
  let onChange: (_ screenRect: NSRect, _ window: NSWindow?) -> Void

  func makeNSView(context: Context) -> AnchorNSView {
    let view = AnchorNSView()
    view.onChange = onChange
    return view
  }

  func updateNSView(_ nsView: AnchorNSView, context: Context) {
    nsView.onChange = onChange
    nsView.report()
  }

  final class AnchorNSView: NSView {
    var onChange: ((NSRect, NSWindow?) -> Void)?
    private var observers: [NSObjectProtocol] = []
    private weak var observedWindow: NSWindow?

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func viewDidMoveToWindow() {
      super.viewDidMoveToWindow()
      reinstallWindowObservers()
      report()
    }

    override func setFrameOrigin(_ newOrigin: NSPoint) {
      super.setFrameOrigin(newOrigin)
      report()
    }

    override func setFrameSize(_ newSize: NSSize) {
      super.setFrameSize(newSize)
      report()
    }

    /// Reports the current screen rect + window to the SwiftUI side.
    func report() {
      guard let window else {
        onChange?(.zero, nil)
        return
      }
      let inWindow = convert(bounds, to: nil)
      let screenRect = window.convertToScreen(inWindow)
      onChange?(screenRect, window)
    }

    private func reinstallWindowObservers() {
      for observer in observers { NotificationCenter.default.removeObserver(observer) }
      observers.removeAll()
      observedWindow = window
      guard let window else { return }

      let center = NotificationCenter.default
      for name in [NSWindow.didResizeNotification, NSWindow.didMoveNotification] {
        let token = center.addObserver(forName: name, object: window, queue: .main) {
          [weak self] _ in
          self?.report()
        }
        observers.append(token)
      }
    }

    deinit {
      for observer in observers { NotificationCenter.default.removeObserver(observer) }
    }
  }
}
