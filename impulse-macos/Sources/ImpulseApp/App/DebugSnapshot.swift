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
///         [--impulse-snapshot-actions-file actions.json]
///         [--impulse-snapshot-session saved-session.json]
///         [--impulse-snapshot-size 1280x800] [--impulse-snapshot-no-lsp]
///         [--impulse-snapshot-persist]
///
/// Windows are made fully transparent so nothing flashes on screen; AppKit
/// still lays them out and draws them into the snapshot bitmap, and web views
/// (Monaco, previews) are snapshotted separately and composited in place.
///
/// The smoke checks (scripts/smoke/run.py) use the same mode: their actions
/// (a JSON list of strings, since scripts contain commas) wait for things
/// (`until=`), run JavaScript in an editor and write the app's state as JSON
/// (`dump=`); `--impulse-snapshot-persist` saves settings and the session as
/// usual, so a second launch can restore them.
enum DebugSnapshot {
  /// Output directory, set when launched with `--impulse-snapshot`.
  private(set) static var outputDirectory: URL?
  private(set) static var delay: TimeInterval = 4
  /// Initial working directory for the first terminal tab.
  private(set) static var initialDirectory: String?
  /// Comma-separated named UI actions to run before capturing.
  private(set) static var actions: [String] = []
  /// A session file to restore (read-only) instead of starting fresh.
  private(set) static var sessionFile: URL?
  /// Size of the main window(s) in points.
  private(set) static var windowSize = NSSize(width: 1440, height: 900)
  /// Start no language servers (their absence would otherwise show notices).
  private(set) static var withoutLanguageServers = false
  /// Save settings, trust and the session as a normal launch does.
  private(set) static var persists = false

  /// What smoke checks read back in a `dump=`: every toast shown (they
  /// dismiss themselves), the results of `js=` actions, and waits that
  /// timed out.
  static var toastLog: [Toast] = []
  static var results: [String: Any] = [:]
  static var failures: [String] = []
  /// The New Task sheet that's up (its SwiftUI content is out of the
  /// accessibility walk's reach): what it holds, and its buttons.
  static var taskSheet: (model: TaskSheetModel, create: () -> Void, cancel: () -> Void)?
  /// The Agent Integration sheet that's up, and how to close it.
  static var agentSheet: (model: AgentHooksModel, close: () -> Void)?
  /// The action running now (for reports).
  private static var currentAction = "launch"

  static var isActive: Bool { outputDirectory != nil }

  static func noteToast(_ toast: Toast) {
    if isActive { toastLog.append(toast) }
  }

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
    if let path = value(after: "--impulse-snapshot-session") {
      sessionFile = URL(fileURLWithPath: (path as NSString).expandingTildeInPath)
    }
    withoutLanguageServers = arguments.contains("--impulse-snapshot-no-lsp")
    persists = arguments.contains("--impulse-snapshot-persist")
    if let raw = value(after: "--impulse-snapshot-size") {
      let parts = raw.split(separator: "x").compactMap { Double($0) }
      if parts.count == 2, parts[0] >= 400, parts[1] >= 300 {
        windowSize = NSSize(width: parts[0], height: parts[1])
      }
    }
    if let raw = value(after: "--impulse-snapshot-actions") {
      actions = raw.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }
    }
    if let path = value(after: "--impulse-snapshot-actions-file"),
      let data = FileManager.default.contents(atPath: (path as NSString).expandingTildeInPath),
      let list = try? JSONDecoder().decode([String].self, from: data)
    {
      actions = list
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
  /// The handler calls its second argument when the action has finished.
  static func run(actionHandler: @escaping (String, @escaping () -> Void) -> Void) {
    guard let outputDirectory else { return }
    try? FileManager.default.createDirectory(
      at: outputDirectory, withIntermediateDirectories: true)
    keepWindowsTransparent()
    for window in NSApp.windows {
      window.alphaValue = 0
      window.ignoresMouseEvents = true
      // Fixed size so snapshots are comparable run to run.
      if window.isVisible { window.setFrame(NSRect(origin: .zero, size: windowSize), display: true) }
    }

    // Each action starts 0.6 s after the one before it finished, so its
    // animations settle; the capture follows the last one by delay/2.
    func perform(_ index: Int) {
      guard index < actions.count else {
        DispatchQueue.main.asyncAfter(deadline: .now() + delay / 2) { capture(into: outputDirectory) }
        return
      }
      var finished = false
      currentAction = String(actions[index].prefix(60))
      actionHandler(actions[index]) {
        guard !finished else { return }
        finished = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) { perform(index + 1) }
      }
    }
    DispatchQueue.main.asyncAfter(deadline: .now() + delay / 2) { perform(0) }
  }

  private static func capture(into outputDirectory: URL) {
    let targets = NSApp.windows.enumerated().filter { $0.element.isVisible }
    var written: [String] = []
    var webNotes: [String] = []
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
        // Never wait forever on a page that doesn't paint.
        var left = false
        let leave = {
          guard !left else { return }
          left = true
          group.leave()
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 5) {
          if !left { NSLog("DebugSnapshot: a web view didn't answer takeSnapshot") }
          leave()
        }
        webView.takeSnapshot(with: nil) { image, error in
          defer { leave() }
          webNotes.append(
            "web view \(Int(webView.frame.width))x\(Int(webView.frame.height)) url=\(webView.url?.lastPathComponent ?? "-")"
              + " image=\(image.map { "\(Int($0.size.width))x\(Int($0.size.height))" } ?? "nil") error=\(error.map { "\($0)" } ?? "-")")
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
        report += webNotes.map { $0 + "\n" }.joined()
        for (index, window) in targets {
          let f = window.frame
          report += "frame window-\(index) x=\(f.minX) y=\(f.minY) w=\(f.width) h=\(f.height)"
          report += " level=\(window.level.rawValue) sheet=\(window.isSheet) class=\(type(of: window))\n"
        }
        report += hitTestReport()
        let log = outputDirectory.appendingPathComponent("snapshot.log")
        try? report.write(to: log, atomically: true, encoding: .utf8)
        exit(0)
      }
    }
  }

  private static func webViews(in view: NSView) -> [WKWebView] {
    if let web = view as? WKWebView { return web.isHidden ? [] : [web] }
    return view.subviews.flatMap { webViews(in: $0) }
  }

  /// Windows made after the run starts (toasts, popovers, the quick
  /// terminal) must not show on screen either: hide each one as soon as the
  /// app updates its windows, and on a short timer as a fallback. Hidden
  /// windows still sit on screen, so they also let clicks through: a click
  /// on one would make Impulse the active app, and the keys typed next, meant
  /// for another app, would land in it. Should it become active anyway, it
  /// steps back and the run reports it.
  private static func keepWindowsTransparent() {
    let hide = {
      for window in NSApp.windows where window.alphaValue != 0 || !window.ignoresMouseEvents {
        window.alphaValue = 0
        window.ignoresMouseEvents = true
      }
    }
    NotificationCenter.default.addObserver(
      forName: NSApplication.didUpdateNotification, object: nil, queue: .main) { _ in hide() }
    Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) { _ in hide() }
    NotificationCenter.default.addObserver(
      forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main) { _ in
      failures.append("Impulse became the active app (during “\(currentAction)”)")
      NSApp.deactivate()
    }
  }
}
