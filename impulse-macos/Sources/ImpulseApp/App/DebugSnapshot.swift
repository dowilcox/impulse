import AppKit
import WebKit

/// Headless visual check: `Impulse Dev --impulse-snapshot <dir>` launches,
/// waits for the UI to settle, renders every visible window to
/// `<dir>/window-<n>.png`, and quits — without restoring or saving session
/// state or settings. Launch it with `open -g` so it never takes focus:
///
///     open -g -n "dist/Impulse Dev.app" --args --impulse-snapshot /tmp/shots \
///         [--impulse-snapshot-delay 4] [--impulse-snapshot-cwd ~/Code/impulse]
///         [--impulse-snapshot-actions sidebar,palette]
///
/// Windows are made fully transparent so nothing flashes on screen; AppKit
/// still lays them out and draws them into the snapshot bitmap. WKWebView
/// content (Monaco) does not render into these snapshots.
enum DebugSnapshot {
  /// Output directory, set when launched with `--impulse-snapshot`.
  private(set) static var outputDirectory: URL?
  private(set) static var delay: TimeInterval = 4
  /// Initial working directory for the first terminal tab.
  private(set) static var initialDirectory: String?
  /// Comma-separated named UI actions to run before capturing.
  private(set) static var actions: [String] = []

  static var isActive: Bool { outputDirectory != nil }

  static func configure(arguments: [String]) {
    func value(after flag: String) -> String? {
      guard let index = arguments.firstIndex(of: flag), index + 1 < arguments.count else {
        return nil
      }
      return arguments[index + 1]
    }
    guard let dir = value(after: "--impulse-snapshot") else { return }
    outputDirectory = URL(fileURLWithPath: (dir as NSString).expandingTildeInPath, isDirectory: true)
    if let raw = value(after: "--impulse-snapshot-delay"), let seconds = TimeInterval(raw) {
      delay = seconds
    }
    if let cwd = value(after: "--impulse-snapshot-cwd") {
      initialDirectory = (cwd as NSString).expandingTildeInPath
    }
    if let raw = value(after: "--impulse-snapshot-actions") {
      actions = raw.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }
    }
  }

  /// Which views receive a click at points along the titlebar band and in the
  /// content — verifies that the custom chrome under the transparent titlebar
  /// actually gets mouse events.
  private static func hitTestReport() -> String {
    var out = ""
    for (index, window) in NSApp.windows.enumerated() where window.isVisible {
      guard let frameView = window.contentView?.superview else { continue }
      let height = frameView.bounds.height
      let width = frameView.bounds.width
      let ys: [CGFloat] = [12, 20, 30]
      let xs: [CGFloat] = [20, 95, 160, 320, 520, width / 2, width - 160, width - 30]
      out += "window-\(index) frame=\(Int(width))x\(Int(height)) titlebarHeight=\(Int(height - window.contentLayoutRect.height))\n"
      for y in ys {
        for x in xs {
          // Theme frame coordinates are y-up.
          let point = NSPoint(x: x, y: height - y)
          var chain: [String] = []
          var view = frameView.hitTest(point)
          while let current = view, chain.count < 4 {
            chain.append(String(describing: type(of: current)))
            view = current.superview
          }
          out += "  hit(\(Int(x)),\(Int(y))): \(chain.joined(separator: " < "))\n"
        }
      }
      for name in ["closeButton", "miniaturizeButton", "zoomButton"] {
        let button: NSButton?
        switch name {
        case "closeButton": button = window.standardWindowButton(.closeButton)
        case "miniaturizeButton": button = window.standardWindowButton(.miniaturizeButton)
        default: button = window.standardWindowButton(.zoomButton)
        }
        if let button, let superview = button.superview {
          let frame = superview.convert(button.frame, to: frameView)
          out += "  \(name): x=\(Int(frame.minX)) yFromTop=\(Int(height - frame.maxY)) h=\(Int(frame.height))\n"
        }
      }
    }
    return out
  }

  /// Called once windows exist. Hides them, runs actions, captures, quits.
  static func run(actionHandler: @escaping (String) -> Void) {
    guard let outputDirectory else { return }
    try? FileManager.default.createDirectory(
      at: outputDirectory, withIntermediateDirectories: true)
    for window in NSApp.windows {
      window.alphaValue = 0
      // Fixed size so snapshots are comparable run to run.
      if window.isVisible { window.setFrame(NSRect(x: 0, y: 0, width: 1440, height: 900), display: true) }
    }

    // Run actions spaced out so each one's animations settle.
    for (index, action) in actions.enumerated() {
      DispatchQueue.main.asyncAfter(deadline: .now() + delay / 2 + Double(index) * 0.6) {
        actionHandler(action)
      }
    }

    let captureAt = delay + Double(actions.count) * 0.6
    DispatchQueue.main.asyncAfter(deadline: .now() + captureAt) {
      let targets = NSApp.windows.enumerated().filter { $0.element.isVisible }
      var written: [String] = []
      let group = DispatchGroup()
      for (index, window) in targets {
        window.alphaValue = 0
        guard let view = window.contentView?.superview ?? window.contentView else { continue }
        view.layoutSubtreeIfNeeded()
        guard let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { continue }
        view.cacheDisplay(in: view.bounds, to: rep)
        // WebViews don't draw through cacheDisplay; snapshot them separately
        // and composite them in place.
        let webViews = Self.webViews(in: view)
        for webView in webViews {
          group.enter()
          webView.takeSnapshot(with: nil) { image, _ in
            defer { group.leave() }
            guard let image else { return }
            let frame = webView.convert(webView.bounds, to: view)
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
            let flipped = view.isFlipped
            let y = flipped ? view.bounds.height - frame.maxY : frame.minY
            image.draw(in: NSRect(x: frame.minX, y: y, width: frame.width, height: frame.height))
            NSGraphicsContext.restoreGraphicsState()
          }
        }
        group.notify(queue: .main) {
          guard let png = rep.representation(using: .png, properties: [:]) else { return }
          let name = "window-\(index).png"
          if (try? png.write(to: outputDirectory.appendingPathComponent(name))) != nil {
            written.append(name)
          }
        }
      }
      group.notify(queue: .main) {
        // Give the per-window notify blocks (queued first) a turn to finish.
        DispatchQueue.main.async {
          var report = "captured: \(written.sorted().joined(separator: ", "))\n"
          report += hitTestReport()
          let log = outputDirectory.appendingPathComponent("snapshot.log")
          try? report.write(to: log, atomically: true, encoding: .utf8)
          exit(0)
        }
      }
    }
  }

  private static func webViews(in view: NSView) -> [WKWebView] {
    if let web = view as? WKWebView { return web.isHidden ? [] : [web] }
    return view.subviews.flatMap { webViews(in: $0) }
  }
}
