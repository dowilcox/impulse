import Foundation

/// Locations of Impulse's per-user data.
///
/// The dev build ("Impulse Dev", bundle id `dev.impulse.Impulse.Devel`) keeps
/// its settings and session state in `Application Support/impulse-dev` so work
/// in progress can never rewrite the daily-driver app's files (e.g. a newer
/// session-state schema). On first launch the dev directory is seeded with a
/// copy of the release settings so the dev app starts with the user's
/// preferences. Themes and managed language servers stay shared.
enum AppPaths {
  /// `~/Library/Application Support/impulse` (or `impulse-dev` for dev builds).
  static let dataDirectory: URL = {
    let base =
      FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
      ?? FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent("Library/Application Support")
    let release = base.appendingPathComponent("impulse", isDirectory: true)
    let dir = AppState.isDev ? base.appendingPathComponent("impulse-dev", isDirectory: true) : release
    let fm = FileManager.default
    let existed = fm.fileExists(atPath: dir.path)
    try? fm.createDirectory(at: dir, withIntermediateDirectories: true)
    try? fm.setAttributes([.posixPermissions: 0o700], ofItemAtPath: dir.path)
    if AppState.isDev && !existed {
      let releaseSettings = release.appendingPathComponent("settings.json")
      let devSettings = dir.appendingPathComponent("settings.json")
      if fm.fileExists(atPath: releaseSettings.path) {
        try? fm.copyItem(at: releaseSettings, to: devSettings)
      }
    }
    return dir
  }()
}
