import AppKit
import SwiftUI

/// Hosts the palette in a borderless, themed child panel centered near the
/// top of its window. It takes key status (for typing) and closes when it
/// loses it, on Escape, or after running a row.
final class PalettePanelController: NSObject, NSWindowDelegate {
  let model = PaletteModel()
  private var panel: PalettePanel?
  private weak var parentWindow: NSWindow?

  var isVisible: Bool { panel?.isVisible ?? false }

  func show(in window: NSWindow, prefix: String, palette: ChromePalette) {
    parentWindow = window
    model.onDismiss = { [weak self] in self?.close() }
    model.prepare(prefix: prefix)

    let panel = self.panel ?? makePanel()
    self.panel = panel
    let host = NSHostingView(rootView: PaletteView(model: model, palette: palette))
    host.sizingOptions = [.intrinsicContentSize]
    host.safeAreaRegions = []
    panel.contentView = host
    host.layoutSubtreeIfNeeded()

    position(panel, in: window, size: host.fittingSize)
    if panel.parent == nil { window.addChildWindow(panel, ordered: .above) }
    panel.makeKeyAndOrderFront(nil)
    panel.invalidateShadow()

    // Track content height changes (results list grows/shrinks).
    sizeObserver = ObservationLoop(owner: self) { [weak self] in
      guard let self else { return }
      _ = self.model.rows.count
      _ = self.model.emptyMessage
      DispatchQueue.main.async { self.resizeToFit() }
    }
  }

  private var sizeObserver: ObservationLoop?

  func close() {
    sizeObserver?.cancel()
    sizeObserver = nil
    guard let panel, panel.isVisible else { return }
    parentWindow?.removeChildWindow(panel)
    panel.orderOut(nil)
    parentWindow?.makeKeyAndOrderFront(nil)
  }

  private func makePanel() -> PalettePanel {
    let panel = PalettePanel(
      contentRect: NSRect(x: 0, y: 0, width: PaletteView.width, height: 120),
      styleMask: [.borderless, .fullSizeContentView],
      backing: .buffered, defer: false)
    panel.isFloatingPanel = false
    panel.hidesOnDeactivate = true
    panel.backgroundColor = .clear
    panel.isOpaque = false
    panel.hasShadow = true
    panel.delegate = self
    panel.isReleasedWhenClosed = false
    if DebugSnapshot.isActive {
      // Headless snapshot: never show on screen or hide with the inactive app.
      panel.alphaValue = 0
      panel.hidesOnDeactivate = false
    }
    return panel
  }

  private func resizeToFit() {
    guard let panel, panel.isVisible, let host = panel.contentView, let window = parentWindow
    else { return }
    host.layoutSubtreeIfNeeded()
    position(panel, in: window, size: host.fittingSize)
    panel.invalidateShadow()
  }

  /// Horizontally centered, its top edge a little below the titlebar.
  private func position(_ panel: NSPanel, in window: NSWindow, size: NSSize) {
    let frame = window.frame
    let top = frame.maxY - Metrics.titlebarHeight - 56
    let origin = NSPoint(x: frame.midX - size.width / 2, y: top - size.height)
    panel.setFrame(NSRect(origin: origin, size: size), display: true)
  }

  func windowDidResignKey(_ notification: Notification) {
    close()
  }
}

/// Borderless panels can't become key by default; the palette must, to type.
final class PalettePanel: NSPanel {
  override var canBecomeKey: Bool { true }
  override var canBecomeMain: Bool { false }
}
