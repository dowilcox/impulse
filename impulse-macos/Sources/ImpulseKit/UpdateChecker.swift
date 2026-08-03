import Foundation

// Ported from impulse-core/src/update.rs (ureq → URLSession).

public struct UpdateInfo: Equatable {
  public let version: String
  public let currentVersion: String
  public let url: String
}

public enum UpdateChecker {
  static let githubOwner = "dowilcox"
  static let githubRepo = "impulse"
  static let checkIntervalSecs: UInt64 = 24 * 60 * 60
  static let requestTimeoutSecs: TimeInterval = 5

  private struct GitHubRelease: Decodable {
    let tagName: String
    let htmlUrl: String

    enum CodingKeys: String, CodingKey {
      case tagName = "tag_name"
      case htmlUrl = "html_url"
    }
  }

  private static var cachePath: URL? {
    FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first?
      .appendingPathComponent("impulse")
      .appendingPathComponent("last_update_check")
  }

  private static func shouldCheck() -> Bool {
    guard let path = cachePath,
      let contents = try? String(contentsOf: path, encoding: .utf8),
      let lastCheck = UInt64(contents.trimmingCharacters(in: .whitespacesAndNewlines))
    else { return true }
    let now = UInt64(Date().timeIntervalSince1970)
    return now >= lastCheck && now - lastCheck >= checkIntervalSecs
  }

  private static func writeCheckTimestamp() {
    guard let path = cachePath else { return }
    try? FileManager.default.createDirectory(
      at: path.deletingLastPathComponent(), withIntermediateDirectories: true)
    let now = UInt64(Date().timeIntervalSince1970)
    try? String(now).write(to: path, atomically: true, encoding: .utf8)
  }

  static func parseVersion(_ tag: String) -> (UInt32, UInt32, UInt32)? {
    let v = tag.hasPrefix("v") ? String(tag.dropFirst()) : tag
    let parts = v.split(separator: ".", omittingEmptySubsequences: false)
    guard parts.count == 3,
      let major = UInt32(parts[0]),
      let minor = UInt32(parts[1]),
      let patch = UInt32(parts[2])
    else { return nil }
    return (major, minor, patch)
  }

  static func isNewer(latest: String, current: String) -> Bool {
    guard let l = parseVersion(latest), let c = parseVersion(current) else { return false }
    return l > c
  }

  /// Check GitHub Releases for a newer version. Synchronous — call from a
  /// background queue. Returns nil when up to date, checked recently, or on
  /// error. Respects a 24-hour cache interval.
  public static func checkForUpdate(currentVersion: String) -> UpdateInfo? {
    guard shouldCheck() else { return nil }

    let urlString =
      "https://api.github.com/repos/\(githubOwner)/\(githubRepo)/releases/latest"
    guard let url = URL(string: urlString) else { return nil }

    var request = URLRequest(url: url, timeoutInterval: requestTimeoutSecs)
    request.setValue("application/vnd.github.v3+json", forHTTPHeaderField: "Accept")
    request.setValue("impulse/\(currentVersion)", forHTTPHeaderField: "User-Agent")

    let semaphore = DispatchSemaphore(value: 0)
    var responseData: Data?
    let task = URLSession.shared.dataTask(with: request) { data, response, _ in
      if let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) {
        responseData = data
      }
      semaphore.signal()
    }
    task.resume()
    _ = semaphore.wait(timeout: .now() + requestTimeoutSecs + 1)

    guard let data = responseData,
      let release = try? JSONDecoder().decode(GitHubRelease.self, from: data)
    else { return nil }

    writeCheckTimestamp()

    guard isNewer(latest: release.tagName, current: currentVersion) else {
      NSLog(
        "No update available (current: %@, latest: %@)", currentVersion, release.tagName)
      return nil
    }

    let version =
      release.tagName.hasPrefix("v") ? String(release.tagName.dropFirst()) : release.tagName
    NSLog("Update available: %@ -> %@", currentVersion, version)
    return UpdateInfo(version: version, currentVersion: currentVersion, url: release.htmlUrl)
  }
}
