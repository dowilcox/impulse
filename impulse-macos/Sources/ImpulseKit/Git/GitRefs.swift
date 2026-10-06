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

/// Web pages for a remote: commits, tags and branches on GitHub, GitLab,
/// Bitbucket, Gitea/Codeberg and Azure DevOps, from the remote's URL.
public struct RemoteWebURL: Equatable, Sendable {
  public enum Host: Equatable, Sendable { case github, gitlab, bitbucket, gitea, azure, other }

  /// `https://host/owner/repo`, no trailing slash or `.git`.
  public let base: String
  public let host: Host

  /// The service's name for menus ("GitHub"), or the host name.
  public var displayName: String {
    switch host {
    case .github: return "GitHub"
    case .gitlab: return "GitLab"
    case .bitbucket: return "Bitbucket"
    case .gitea: return URL(string: base)?.host == "codeberg.org" ? "Codeberg" : "Gitea"
    case .azure: return "Azure DevOps"
    case .other: return URL(string: base)?.host ?? "Remote"
    }
  }

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

    if hostName == "ssh.dev.azure.com" || hostName.hasSuffix("vs-ssh.visualstudio.com") {
      // v3/org/project/repo → dev.azure.com/org/project/_git/repo
      var parts = path.split(separator: "/").map(String.init)
      if parts.first == "v3" { parts.removeFirst() }
      guard parts.count == 3 else { return nil }
      self.base = "https://dev.azure.com/\(parts[0])/\(parts[1])/_git/\(parts[2])"
      self.host = .azure
      return
    }
    self.base = "\(webScheme)://\(hostName)/\(path)"
    if hostName == "github.com" || hostName.hasPrefix("github.") {
      host = .github
    } else if hostName == "gitlab.com" || hostName.hasPrefix("gitlab.") {
      host = .gitlab
    } else if hostName == "bitbucket.org" {
      host = .bitbucket
    } else if hostName == "codeberg.org" || hostName.hasPrefix("gitea.") {
      host = .gitea
    } else if hostName == "dev.azure.com" {
      host = .azure
    } else {
      host = .other
    }
  }

  public var repository: URL? { URL(string: base) }

  public func commit(_ sha: String) -> URL? {
    switch host {
    case .gitlab: return URL(string: "\(base)/-/commit/\(sha)")
    case .bitbucket: return URL(string: "\(base)/commits/\(sha)")
    case .azure: return URL(string: "\(base)/commit/\(sha)")
    case .github, .gitea, .other: return URL(string: "\(base)/commit/\(sha)")
    }
  }

  public func tag(_ name: String) -> URL? {
    let name = Self.escape(name)
    switch host {
    case .github: return URL(string: "\(base)/releases/tag/\(name)")
    case .gitlab: return URL(string: "\(base)/-/tags/\(name)")
    case .bitbucket: return URL(string: "\(base)/src/\(name)")
    case .gitea: return URL(string: "\(base)/src/tag/\(name)")
    case .azure: return URL(string: "\(base)?version=GT\(name)")
    case .other: return URL(string: "\(base)/tree/\(name)")
    }
  }

  public func branch(_ name: String) -> URL? {
    let name = Self.escape(name)
    switch host {
    case .github, .other: return URL(string: "\(base)/tree/\(name)")
    case .gitlab: return URL(string: "\(base)/-/tree/\(name)")
    case .bitbucket: return URL(string: "\(base)/branch/\(name)")
    case .gitea: return URL(string: "\(base)/src/branch/\(name)")
    case .azure: return URL(string: "\(base)?version=GB\(name)")
    }
  }

  private static func escape(_ name: String) -> String {
    var allowed = CharacterSet.urlPathAllowed
    allowed.remove(charactersIn: "?#")
    return name.addingPercentEncoding(withAllowedCharacters: allowed) ?? name
  }
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
