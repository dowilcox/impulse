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
