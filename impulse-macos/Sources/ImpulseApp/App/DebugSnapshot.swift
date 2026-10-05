import AppKit

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

  /// Called once windows exist. Hides them, runs actions, captures, quits.
  static func run(actionHandler: @escaping (String) -> Void) {
    guard let outputDirectory else { return }
    try? FileManager.default.createDirectory(
      at: outputDirectory, withIntermediateDirectories: true)
    for window in NSApp.windows { window.alphaValue = 0 }

    // Run actions spaced out so each one's animations settle.
    for (index, action) in actions.enumerated() {
      DispatchQueue.main.asyncAfter(deadline: .now() + delay / 2 + Double(index) * 0.6) {
        actionHandler(action)
      }
    }

    let captureAt = delay + Double(actions.count) * 0.6
    DispatchQueue.main.asyncAfter(deadline: .now() + captureAt) {
      var written: [String] = []
      for (index, window) in NSApp.windows.enumerated() where window.isVisible {
        window.alphaValue = 0
        guard let view = window.contentView?.superview ?? window.contentView else { continue }
        view.layoutSubtreeIfNeeded()
        guard let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { continue }
        view.cacheDisplay(in: view.bounds, to: rep)
        guard let png = rep.representation(using: .png, properties: [:]) else { continue }
        let name = "window-\(index).png"
        let url = outputDirectory.appendingPathComponent(name)
        if (try? png.write(to: url)) != nil { written.append(name) }
      }
      let log = outputDirectory.appendingPathComponent("snapshot.log")
      try? "captured: \(written.joined(separator: ", "))\n".write(
        to: log, atomically: true, encoding: .utf8)
      exit(0)
    }
  }
}
