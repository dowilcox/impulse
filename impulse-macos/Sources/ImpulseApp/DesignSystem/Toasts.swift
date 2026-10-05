import AppKit
import Observation
import SwiftUI

/// A transient message, optionally with one action (usually "Undo").
struct Toast: Identifiable {
  enum Kind { case info, success, warning, error }

  let id = UUID()
  var kind: Kind = .info
  var message: String
  var detail: String? = nil
  var actionTitle: String? = nil
  var action: (() -> Void)? = nil
  /// Seconds before it dismisses itself (nil: stays until dismissed).
  var lifetime: TimeInterval? = 6
}

/// Per-window toast queue, shown bottom-center in a child panel so toasts
/// float above the terminal and WebViews (which SwiftUI overlays can't).
@Observable
final class ToastCenter {
  private(set) var toasts: [Toast] = []

  @ObservationIgnored private var presenter: ToastPresenter?

  func attach(to window: NSWindow, palette: @escaping () -> ChromePalette) {
    presenter = ToastPresenter(center: self, window: window, palette: palette)
  }

  func show(_ toast: Toast) {
    toasts.append(toast)
    if toasts.count > 3 { toasts.removeFirst(toasts.count - 3) }
    presenter?.update()
    if let lifetime = toast.lifetime {
      DispatchQueue.main.asyncAfter(deadline: .now() + lifetime) { [weak self] in
        self?.dismiss(toast.id)
      }
    }
  }

  func dismiss(_ id: UUID) {
    toasts.removeAll { $0.id == id }
    presenter?.update()
  }
}

private final class ToastPresenter {
  private weak var center: ToastCenter?
  private weak var window: NSWindow?
  private let palette: () -> ChromePalette
  private var panel: NSPanel?
  private var host: NSHostingView<ToastStack>?
  private var observers: [NSObjectProtocol] = []

  init(center: ToastCenter, window: NSWindow, palette: @escaping () -> ChromePalette) {
    self.center = center
    self.window = window
    self.palette = palette
    for name in [NSWindow.didResizeNotification, NSWindow.didMoveNotification] {
      observers.append(
        NotificationCenter.default.addObserver(forName: name, object: window, queue: .main) {
          [weak self] _ in self?.position()
        })
    }
  }

  deinit {
    observers.forEach { NotificationCenter.default.removeObserver($0) }
  }

  func update() {
    guard let center, let window else { return }
    if center.toasts.isEmpty {
      if let panel {
        window.removeChildWindow(panel)
        panel.orderOut(nil)
      }
      return
    }
    let panel = self.panel ?? makePanel()
    self.panel = panel
    let stack = ToastStack(center: center, palette: palette())
    if let host {
      host.rootView = stack
    } else {
      let host = NSHostingView(rootView: stack)
      host.sizingOptions = [.intrinsicContentSize]
      host.safeAreaRegions = []
      panel.contentView = host
      self.host = host
    }
    if panel.parent == nil { window.addChildWindow(panel, ordered: .above) }
    position()
    panel.orderFront(nil)
  }

  private func makePanel() -> NSPanel {
    let panel = ToastPanel(
      contentRect: NSRect(x: 0, y: 0, width: 420, height: 60),
      styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
    panel.backgroundColor = .clear
    panel.isOpaque = false
    panel.hasShadow = false
    panel.isReleasedWhenClosed = false
    panel.hidesOnDeactivate = false
    if DebugSnapshot.isActive { panel.alphaValue = 0 }
    return panel
  }

  private func position() {
    guard let panel, let host, let window else { return }
    host.layoutSubtreeIfNeeded()
    let size = host.fittingSize
    let frame = window.frame
    let origin = NSPoint(
      x: frame.midX - size.width / 2, y: frame.minY + Metrics.statusBarHeight + 18)
    panel.setFrame(NSRect(origin: origin, size: size), display: true)
  }
}

/// Never takes key focus, so clicking Undo doesn't steal the keyboard.
private final class ToastPanel: NSPanel {
  override var canBecomeKey: Bool { false }
  override var canBecomeMain: Bool { false }
}

private struct ToastStack: View {
  var center: ToastCenter
  let palette: ChromePalette

  var body: some View {
    VStack(spacing: 8) {
      ForEach(center.toasts) { toast in
        ToastView(toast: toast, palette: palette) { center.dismiss(toast.id) }
      }
    }
    .padding(10)
    .environment(\.chrome, palette)
  }
}

private struct ToastView: View {
  let toast: Toast
  let palette: ChromePalette
  let dismiss: () -> Void

  var body: some View {
    HStack(spacing: 10) {
      Icon(icon, size: 14).foregroundStyle(tint)
      VStack(alignment: .leading, spacing: 2) {
        Text(toast.message)
          .font(ChromeFont.ui(12, weight: .medium))
          .foregroundStyle(palette.text)
          .lineLimit(2)
        if let detail = toast.detail {
          Text(detail)
            .font(ChromeFont.ui(11))
            .foregroundStyle(palette.textSecondary)
            .lineLimit(3)
        }
      }
      .frame(minWidth: 200, maxWidth: 380, alignment: .leading)
      if let title = toast.actionTitle, let action = toast.action {
        ChromeButton(title: title, kind: .secondary) {
          action()
          dismiss()
        }
      }
      ChromeIconButton(icon: .x, help: "Dismiss", size: 20, iconSize: 11, action: dismiss)
    }
    .padding(.leading, 12)
    .padding(.trailing, 6)
    .padding(.vertical, 8)
    .background(
      RoundedRectangle(cornerRadius: Metrics.radiusLarge, style: .continuous).fill(palette.overlay)
    )
    .overlay(
      RoundedRectangle(cornerRadius: Metrics.radiusLarge, style: .continuous)
        .strokeBorder(palette.hairlineStrong, lineWidth: 1)
    )
    .shadow(color: .black.opacity(0.25), radius: 12, y: 4)
  }

  private var icon: LucideIcon {
    switch toast.kind {
    case .info: return .info
    case .success: return .circleCheck
    case .warning: return .triangleAlert
    case .error: return .circleX
    }
  }

  private var tint: Color {
    switch toast.kind {
    case .info: return palette.info
    case .success: return palette.success
    case .warning: return palette.warning
    case .error: return palette.danger
    }
  }
}
