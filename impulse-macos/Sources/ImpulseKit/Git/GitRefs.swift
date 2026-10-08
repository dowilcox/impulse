import Foundation

/// Names git accepts for branches and tags (the `git check-ref-format`
/// rules that matter for a single name typed by a person).
public enum GitRefName {
  public static func isValid(_ name: String) -> Bool {
    guard !name.isEmpty, name != "@", !name.hasPrefix("-"), !name.hasPrefix("/"), !name.hasSuffix("/"),
      !name.hasSuffix(".lock"), !name.hasSuffix("."), !name.contains(".."), !name.contains("//"),
      !name.contains("@{")
    else { return false }
    // No component may start with a dot.
    if name.split(separator: "/").contains(where: { $0.hasPrefix(".") }) { return false }
    let forbidden = CharacterSet(charactersIn: " ~^:?*[\\").union(.controlCharacters)
    return name.unicodeScalars.allSatisfy { !forbidden.contains($0) }
  }
}

/// One `git log --decorate=short` decoration.
public enum RefDecoration: Equatable, Sendable {
  /// `HEAD -> main`: the checked-out branch.
  case head(branch: String)
  /// `HEAD` alone: detached.
  case detachedHead
  case localBranch(String)
  case remoteBranch(remote: String, branch: String)
  case tag(String)

  /// Parse a decoration; `remotes` tells `origin/main` from a local
  /// `feature/x`.
  public static func parse(_ text: String, remotes: [String]) -> RefDecoration {
    if text == "HEAD" { return .detachedHead }
    if text.hasPrefix("HEAD -> ") { return .head(branch: String(text.dropFirst(8))) }
    if text.hasPrefix("tag: ") { return .tag(String(text.dropFirst(5))) }
    // The longest remote name that prefixes it (remotes may contain "/").
    if let remote = remotes.filter({ text.hasPrefix($0 + "/") }).max(by: { $0.count < $1.count }) {
      return .remoteBranch(remote: remote, branch: String(text.dropFirst(remote.count + 1)))
    }
    return .localBranch(text)
  }

  /// The name shown on a chip.
  public var label: String {
    switch self {
    case .head(let branch): return branch
    case .detachedHead: return "HEAD"
    case .localBranch(let name): return name
    case .remoteBranch(let remote, let branch): return "\(remote)/\(branch)"
    case .tag(let name): return name
    }
  }
}

/// A remote's repository page, from its URL alone: `git@host:owner/repo.git`
/// becomes `https://host/owner/repo`. Impulse uses only git, so it knows
/// nothing about the host behind the address (no per-host page layouts).
public struct RemoteWebURL: Equatable, Sendable {
  /// `https://host/owner/repo`, no trailing slash or `.git`.
  public let base: String

  /// Parse `git@host:owner/repo.git`, `ssh://git@host[:port]/owner/repo`,
  /// `https://user@host/owner/repo.git` and the like. Nil for local paths.
  public init?(remote: String) {
    var text = remote.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !text.isEmpty else { return nil }
    var hostName: String
    var path: String
    // Plain-http servers stay http; everything else is browsed over https.
    var webScheme = "https"
    if let schemeEnd = text.range(of: "://") {
      let scheme = text[..<schemeEnd.lowerBound].lowercased()
      if scheme == "http" { webScheme = "http" }
      guard ["https", "http", "ssh", "git", "git+ssh", "ssh+git"].contains(scheme) else { return nil }
      text = String(text[schemeEnd.upperBound...])
      guard let slash = text.firstIndex(of: "/") else { return nil }
      hostName = String(text[..<slash])
      path = String(text[text.index(after: slash)...])
      if let at = hostName.lastIndex(of: "@") { hostName = String(hostName[hostName.index(after: at)...]) }
      // An ssh port isn't the web port.
      if let colon = hostName.firstIndex(of: ":"), scheme != "https" && scheme != "http" {
        hostName = String(hostName[..<colon])
      }
    } else if let colon = text.firstIndex(of: ":"), !text[..<colon].contains("/") {
      // scp-like: [user@]host:path
      hostName = String(text[..<colon])
      path = String(text[text.index(after: colon)...])
      if let at = hostName.lastIndex(of: "@") { hostName = String(hostName[hostName.index(after: at)...]) }
    } else {
      return nil
    }
    hostName = hostName.lowercased()
    path = path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
    if path.hasSuffix(".git") { path = String(path.dropLast(4)) }
    // A Windows drive ("C:\\repo") isn't a host.
    guard !hostName.isEmpty, !path.isEmpty, !path.contains("\\"), hostName.count > 1 else { return nil }
    self.base = "\(webScheme)://\(hostName)/\(path)"
  }

  public var repository: URL? { URL(string: base) }
}

/// A name for the next tag: the newest version-like tag with its last
/// number bumped ("v1.4.2" → "v1.4.3", "release-7" → "release-8").
public enum TagNameSuggestion {
  /// `tags` newest first.
  public static func next(after tags: [String]) -> String? {
    for tag in tags {
      // The last run of digits, with a version-ish prefix before it.
      guard let lastDigit = tag.lastIndex(where: \.isNumber) else { continue }
      var start = lastDigit
      while start > tag.startIndex, tag[tag.index(before: start)].isNumber { start = tag.index(before: start) }
      let suffix = tag[tag.index(after: lastDigit)...]
      // Pre-release suffixes ("-rc1", "-beta") aren't bumped into.
      guard suffix.isEmpty, let number = Int(tag[start...lastDigit]) else { continue }
      let prefix = tag[..<start]
      guard prefix.isEmpty || prefix.last == "." || prefix.last == "v" || prefix.last == "-" || prefix.last == "_"
      else { continue }
      return "\(prefix)\(number + 1)"
    }
    return nil
  }
}
