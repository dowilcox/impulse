import Foundation

/// Fetches the repositories open in windows every few minutes (the
/// `git_auto_fetch_minutes` setting; off by default), so ahead/behind counts
/// and History's incoming commits stay current without a manual Fetch.
final class AutoFetch {
  static let shared = AutoFetch()

  /// The repositories to keep fresh (each window's workspaces).
  var repositories: () -> [GitRepositoryState] = { [] }
  private var timer: Timer?

  func start() {
    guard timer == nil else { return }
    let timer = Timer(timeInterval: 60, repeats: true) { [weak self] _ in self?.tick() }
    timer.tolerance = 15
    RunLoop.main.add(timer, forMode: .common)
    self.timer = timer
  }

  private func tick() {
    let minutes = SettingsStore.shared.settings.gitAutoFetchMinutes
    guard minutes > 0 else { return }
    let interval = TimeInterval(minutes * 60)
    var seen = Set<String>()
    for repository in repositories() where seen.insert(repository.root).inserted {
      if let last = repository.lastFetch, Date().timeIntervalSince(last) < interval { continue }
      repository.fetchQuietly()
    }
  }
}
