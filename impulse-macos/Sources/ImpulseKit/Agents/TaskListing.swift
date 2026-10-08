// What `impulse tasks` tells an agent (or you) about a repository's
// workspaces: the main checkout and each task Impulse made, with its branch,
// base, agent, uncommitted files, how far it is from its base, and the files
// it shares with the caller. Agents only know their own folder; this is how
// one learns what the others are doing. The session-start hook sends a short
// form of it, and the hooks' replies are built here too.

import Foundation

public struct TaskListing: Codable, Equatable, Sendable {
  public struct Workspace: Codable, Equatable, Sendable {
    public var name: String
    public var path: String
    public var branch: String?
    /// The branch it was made from (`origin/main`); nil for the main checkout.
    public var base: String?
    public var head: String?
    public var isMainCheckout: Bool
    /// The caller's own workspace.
    public var isCaller: Bool
    public var agent: String?
    /// "working", "needs input", "finished" or "idle".
    public var agentState: String?
    public var uncommitted: Int
    /// Commits on it that its base lacks, and the reverse.
    public var ahead: Int?
    public var behind: Int?
    /// Files it changes that the caller's workspace changes too.
    public var sharedWithCaller: [String]

    public init(
      name: String, path: String, branch: String?, base: String?, head: String?, isMainCheckout: Bool,
      isCaller: Bool, agent: String? = nil, agentState: String? = nil, uncommitted: Int = 0, ahead: Int? = nil,
      behind: Int? = nil, sharedWithCaller: [String] = []
    ) {
      self.name = name
      self.path = path
      self.branch = branch
      self.base = base
      self.head = head
      self.isMainCheckout = isMainCheckout
      self.isCaller = isCaller
      self.agent = agent
      self.agentState = agentState
      self.uncommitted = uncommitted
      self.ahead = ahead
      self.behind = behind
      self.sharedWithCaller = sharedWithCaller
    }
  }

  public var repository: String
  public var workspaces: [Workspace]

  public init(repository: String, workspaces: [Workspace]) {
    self.repository = repository
    self.workspaces = workspaces
  }

  public func json() -> String {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
    return (try? encoder.encode(self)).map { String(decoding: $0, as: UTF8.self) } ?? "{}"
  }

  /// One block per workspace, for people and agents alike.
  public func text() -> String {
    var lines = ["\(repository): \(workspaces.count) workspace\(workspaces.count == 1 ? "" : "s")"]
    for workspace in workspaces {
      lines.append("")
      var title = workspace.name + (workspace.isMainCheckout ? " (main checkout)" : " (task)")
      if workspace.isCaller { title += " ← you" }
      lines.append(title)
      var branch = "  branch \(workspace.branch ?? "detached")"
      if let base = workspace.base { branch += " from \(base)" }
      if let ahead = workspace.ahead, let behind = workspace.behind {
        branch += ", \(ahead) ahead, \(behind) behind"
      }
      if let head = workspace.head { branch += " at \(head.prefix(7))" }
      lines.append(branch)
      lines.append("  \(workspace.path)")
      if let agent = workspace.agent { lines.append("  \(agent): \(workspace.agentState ?? "running")") }
      if workspace.uncommitted > 0 {
        lines.append("  \(workspace.uncommitted) uncommitted file\(workspace.uncommitted == 1 ? "" : "s")")
      }
      if !workspace.sharedWithCaller.isEmpty {
        let shown = workspace.sharedWithCaller.prefix(8).joined(separator: ", ")
        let more = workspace.sharedWithCaller.count > 8 ? " and \(workspace.sharedWithCaller.count - 8) more" : ""
        lines.append("  also changes files you change: \(shown)\(more)")
      }
    }
    return lines.joined(separator: "\n")
  }

  /// A few lines for an agent starting in the caller's workspace; nil when
  /// it's the only workspace (nothing to coordinate with).
  public func sessionSummary() -> String? {
    guard workspaces.count > 1, let me = workspaces.first(where: \.isCaller) else { return nil }
    var lines: [String] = []
    if me.isMainCheckout {
      lines.append("Impulse: you're in the main checkout of \(repository), which has \(workspaces.count - 1) task\(workspaces.count == 2 ? "" : "s") (git worktrees) other agents may be working in.")
    } else {
      lines.append("Impulse: you're in the task \(me.name) of \(repository): a git worktree on branch \(me.branch ?? "?")\(me.base.map { ", made from \($0)" } ?? "").")
    }
    let others = workspaces.filter { !$0.isCaller }.map { other -> String in
      var parts: [String] = []
      if let agent = other.agent { parts.append("\(agent) \(other.agentState ?? "running")") }
      if other.uncommitted > 0 { parts.append("\(other.uncommitted) uncommitted") }
      if !other.sharedWithCaller.isEmpty { parts.append("changes \(other.sharedWithCaller.count) of your files") }
      return other.name + (other.isMainCheckout ? " (main checkout)" : "") + (parts.isEmpty ? "" : " — " + parts.joined(separator: ", "))
    }
    lines.append("Other workspaces: " + others.joined(separator: "; ") + ".")
    lines.append("Run `impulse tasks` before merging a branch or making sweeping edits, to see what they're changing.")
    return lines.joined(separator: "\n")
  }
}

/// The JSON Claude Code hooks reply with.
public enum AgentHookReply {
  /// A note added to the agent's context after a tool ran (PostToolUse).
  public static func context(_ text: String, event: String = "PostToolUse") -> String {
    encode(["hookSpecificOutput": ["hookEventName": event, "additionalContext": text]])
  }

  /// Make the agent ask the user before running a tool (PreToolUse), with
  /// the reason shown in the prompt.
  public static func ask(_ reason: String) -> String {
    encode([
      "hookSpecificOutput": [
        "hookEventName": "PreToolUse", "permissionDecision": "ask", "permissionDecisionReason": reason,
      ]
    ])
  }

  private static func encode(_ object: [String: Any]) -> String {
    (try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys, .withoutEscapingSlashes]))
      .map { String(decoding: $0, as: UTF8.self) } ?? "{}"
  }
}
