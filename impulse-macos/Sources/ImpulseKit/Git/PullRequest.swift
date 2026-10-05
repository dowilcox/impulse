// The pull request for a branch, as `gh pr view --json …` reports it.

import Foundation

public struct PullRequestInfo: Equatable, Sendable {
  public enum State: String, Sendable { case open = "OPEN", closed = "CLOSED", merged = "MERGED" }
  public enum Checks: Equatable, Sendable { case none, pending, passed, failed }

  public let number: Int
  public let title: String
  public let state: State
  public let isDraft: Bool
  /// "APPROVED", "CHANGES_REQUESTED", "REVIEW_REQUIRED" or "".
  public let reviewDecision: String
  public let checks: Checks
  public let url: String

  /// The fields `parse` expects (`gh pr view --json <fields>`).
  public static let ghFields = "number,title,state,isDraft,reviewDecision,statusCheckRollup,url"

  public static func parse(_ json: Data) -> PullRequestInfo? {
    guard let object = try? JSONSerialization.jsonObject(with: json) as? [String: Any],
      let number = object["number"] as? Int,
      let state = (object["state"] as? String).flatMap(State.init(rawValue:))
    else { return nil }
    let checks = (object["statusCheckRollup"] as? [[String: Any]]) ?? []
    return PullRequestInfo(
      number: number,
      title: object["title"] as? String ?? "",
      state: state,
      isDraft: object["isDraft"] as? Bool ?? false,
      reviewDecision: object["reviewDecision"] as? String ?? "",
      checks: rollup(checks),
      url: object["url"] as? String ?? "")
  }

  /// Failed if anything failed, pending if anything is still running,
  /// passed when everything finished well.
  static func rollup(_ checks: [[String: Any]]) -> Checks {
    guard !checks.isEmpty else { return .none }
    var pending = false
    for check in checks {
      if let state = check["state"] as? String {  // StatusContext
        switch state {
        case "FAILURE", "ERROR": return .failed
        case "PENDING", "EXPECTED": pending = true
        default: break
        }
        continue
      }
      let status = check["status"] as? String ?? ""
      let conclusion = check["conclusion"] as? String ?? ""
      if status != "COMPLETED" {
        pending = true
      } else if ["FAILURE", "CANCELLED", "TIMED_OUT", "ACTION_REQUIRED", "STARTUP_FAILURE"].contains(conclusion) {
        return .failed
      }
    }
    return pending ? .pending : .passed
  }
}

/// Review threads on a pull request (GitHub GraphQL `reviewThreads`),
/// turned into review comments anchored on the PR's diff.
public enum PullRequestThreads {
  public static let query = """
    query($owner: String!, $name: String!, $number: Int!) {
      repository(owner: $owner, name: $name) {
        pullRequest(number: $number) {
          reviewThreads(first: 100) {
            nodes {
              id isResolved isOutdated path diffSide
              line startLine originalLine originalStartLine
              comments(first: 50) { nodes { author { login } body url createdAt } }
            }
          }
        }
      }
    }
    """

  /// Owner, repository name and number from a PR's web URL
  /// ("https://github.com/owner/name/pull/12").
  public static func coordinates(fromURL url: String) -> (owner: String, name: String, number: Int)? {
    guard let components = URL(string: url)?.pathComponents.filter({ $0 != "/" }),
      components.count >= 4, components[2] == "pull", let number = Int(components[3])
    else { return nil }
    return (components[0], components[1], number)
  }

  /// One comment per thread (replies folded into its text). Resolved
  /// threads are skipped unless asked for. Ids are stable per thread so a
  /// re-import replaces rather than duplicates.
  public static func parse(_ json: Data, includeResolved: Bool = false) -> [ReviewComment]? {
    guard let root = try? JSONSerialization.jsonObject(with: json) as? [String: Any],
      let data = root["data"] as? [String: Any],
      let repository = data["repository"] as? [String: Any],
      let pullRequest = repository["pullRequest"] as? [String: Any],
      let threads = (pullRequest["reviewThreads"] as? [String: Any])?["nodes"] as? [[String: Any]]
    else { return nil }
    let dates = ISO8601DateFormatter()
    return threads.compactMap { thread in
      guard let id = thread["id"] as? String, let path = thread["path"] as? String,
        includeResolved || (thread["isResolved"] as? Bool) != true
      else { return nil }
      let notes = ((thread["comments"] as? [String: Any])?["nodes"] as? [[String: Any]]) ?? []
      guard let first = notes.first else { return nil }
      // A thread whose code moved on has no current line: fall back to the
      // line it was written on and call it outdated.
      var outdated = thread["isOutdated"] as? Bool ?? false
      var end = thread["line"] as? Int
      var start = thread["startLine"] as? Int
      if end == nil {
        outdated = true
        end = thread["originalLine"] as? Int
        start = thread["originalStartLine"] as? Int
      }
      guard let end else { return nil }
      let text = notes.map { note -> String in
        let login = ((note["author"] as? [String: Any])?["login"] as? String) ?? "ghost"
        let body = (note["body"] as? String ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        return "@\(login): \(body)"
      }.joined(separator: "\n\n")
      let author = ((first["author"] as? [String: Any])?["login"] as? String) ?? "ghost"
      return ReviewComment(
        id: "gh:\(id)", path: path, side: thread["diffSide"] as? String == "LEFT" ? .old : .new,
        line: min(start ?? end, end), endLine: end, snippet: "", text: text,
        createdAt: (first["createdAt"] as? String).flatMap(dates.date(from:)) ?? Date(),
        remote: .init(author: author, url: first["url"] as? String ?? "", isOutdated: outdated))
    }
  }
}

/// One row of `gh pr list --json <PullRequestSummary.ghFields>`.
public struct PullRequestSummary: Equatable, Sendable {
  public let number: Int
  public let title: String
  /// The PR's branch name in its head repository.
  public let headBranch: String
  public let author: String
  public let isDraft: Bool

  public static let ghFields = "number,title,headRefName,author,isDraft"

  public static func parseList(_ json: Data) -> [PullRequestSummary]? {
    guard let list = try? JSONSerialization.jsonObject(with: json) as? [[String: Any]] else { return nil }
    return list.compactMap { object in
      guard let number = object["number"] as? Int, let head = object["headRefName"] as? String else {
        return nil
      }
      return PullRequestSummary(
        number: number, title: object["title"] as? String ?? "", headBranch: head,
        author: ((object["author"] as? [String: Any])?["login"] as? String) ?? "",
        isDraft: object["isDraft"] as? Bool ?? false)
    }
  }

  /// Local branch name for checking it out: its own name unless that's
  /// taken (or is a default branch, as fork PRs often are), then
  /// `pr-<number>-<name>`.
  public func localBranch(taken: Set<String>) -> String {
    let reserved: Set<String> = ["main", "master", "trunk", "develop"]
    if !taken.contains(headBranch), !reserved.contains(headBranch) { return headBranch }
    return "pr-\(number)-\(headBranch)"
  }
}
