// What agents' hook events mean to Impulse: Claude Code's hook events,
// Codex's notify, and the events Impulse's opencode plugin sends. An event
// may change the agent's state, and Claude Code and the opencode plugin
// also read what Impulse answers (a summary of the other tasks, notes about
// shared files and dependencies, a question before a risky merge).

import Foundation

public enum AgentHookEvents {
  /// The state change `event` reports; nil for events that don't change it
  /// (tools starting, subagents, compaction, a session's summary).
  public static func hook(agent: String?, event: String, tool: String?, message: String?) -> AgentHook? {
    switch event {
    case "SessionStart": return .sessionStarted
    case "UserPromptSubmit": return .promptSubmitted
    case "PostToolUse": return .usedTool(tool)
    case "Stop", "agent-turn-complete": return .stopped
    case "StopFailure": return .failed(message ?? "")
    case "Notification": return .notification(message ?? "")
    default: break
    }
    guard agent == "opencode" else { return nil }
    switch event {
    // The plugin loaded in the terminal: opencode started.
    case "start": return .sessionStarted
    case "session.execution.started": return .promptSubmitted
    case "permission.asked": return .notification("opencode needs your permission to use \(tool ?? "a tool")")
    // opencode's question tool asks with a form.
    case "form.created": return .notification("opencode has a question for you")
    case "tool.execute.after": return .usedTool(tool)
    // Answered (or dismissed): back to work, or to the turn's end.
    case "permission.replied", "form.replied", "form.cancelled": return .usedTool(nil)
    // An interrupted turn ends like any other.
    case "session.execution.succeeded", "session.execution.interrupted": return .stopped
    case "session.execution.failed": return .failed(message ?? "")
    default: return nil
    }
  }

  /// What Impulse can answer an event with.
  public enum Reply: Equatable, Sendable {
    /// The repository's other workspaces, when a session starts.
    case sessionSummary
    /// Whether another workspace changes the file just edited.
    case editedFile
    /// Dependency files a checkout, merge or pull just changed.
    case ranShell
    /// A question before a command that merges a branch a task is still on.
    case beforeShell
  }

  /// Which answer `event` may get; nil for events, and agents, that don't
  /// read one.
  public static func reply(agent: String?, event: String, tool: String?) -> Reply? {
    switch (agent, event, tool) {
    case ("claude", "SessionStart", _): return .sessionSummary
    case ("claude", "PostToolUse", let tool?) where claudeEditTools.contains(tool): return .editedFile
    case ("claude", "PostToolUse", "Bash"?): return .ranShell
    case ("claude", "PreToolUse", "Bash"?): return .beforeShell
    case ("opencode", "session.start", _): return .sessionSummary
    case ("opencode", "tool.execute.after", let tool?) where opencodeEditTools.contains(tool): return .editedFile
    case ("opencode", "tool.execute.after", "shell"?): return .ranShell
    case ("opencode", "tool.execute.before", "shell"?): return .beforeShell
    default: return nil
    }
  }

  static let claudeEditTools: Set<String> = ["Edit", "Write", "MultiEdit", "NotebookEdit"]
  /// opencode's tools that edit files (the plugin reports each file a
  /// patch changes).
  static let opencodeEditTools: Set<String> = ["edit", "write", "patch"]
}
