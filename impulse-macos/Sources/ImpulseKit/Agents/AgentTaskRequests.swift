// Tasks an agent asks for (`impulse tasks new`, the MCP server's
// create_task): where their brief is kept, and whether Impulse creates one
// straight away, asks you first, or refuses. Creating a task starts
// processes (a setup script, an agent that spends money and edits files),
// so the answer is "ask" unless the user chose otherwise, and some cases
// always ask whatever they chose.

import Foundation

public enum TaskBrief {
  /// `.git/impulse/tasks/<task>/brief.md`: beside the task's Compose
  /// override, outside its folder, so it's never committed by accident.
  public static func path(task: String, commonGitDirectory: String) -> String {
    ((TaskRegistry.directory(commonGitDirectory: commonGitDirectory) as NSString)
      .appendingPathComponent("tasks/\(task)") as NSString).appendingPathComponent("brief.md")
  }

  public static func read(task: String, commonGitDirectory: String) -> String? {
    FileManager.default.contents(atPath: path(task: task, commonGitDirectory: commonGitDirectory))
      .map { String(decoding: $0, as: UTF8.self) }
  }

  @discardableResult
  public static func write(_ brief: String, task: String, commonGitDirectory: String) -> Bool {
    let file = path(task: task, commonGitDirectory: commonGitDirectory)
    do {
      try FileManager.default.createDirectory(
        atPath: (file as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
      try Data(brief.utf8).write(to: URL(fileURLWithPath: file), options: .atomic)
      return true
    } catch {
      return false
    }
  }

  /// Its first non-empty line, shortened, for listings.
  public static func summary(_ brief: String, limit: Int = 120) -> String? {
    guard
      let line = brief.components(separatedBy: .newlines).map({ $0.trimmingCharacters(in: .whitespaces) })
        .first(where: { !$0.isEmpty })
    else { return nil }
    return line.count > limit ? String(line.prefix(limit - 1)) + "…" : line
  }
}

public enum AgentTaskPolicy {
  /// `agent_task_creation`.
  public enum Setting: String, CaseIterable, Sendable {
    case ask
    /// Create without asking in a trusted repository whose project
    /// settings are trusted; otherwise ask.
    case trustedProjects = "trusted_projects"
    case off

    public init(setting: String) {
      self = Setting(rawValue: setting) ?? .ask
    }
  }

  public enum Decision: Equatable, Sendable {
    /// Show the New Task sheet, prefilled, and wait for the user.
    case ask
    /// Create it now, with a toast and Undo.
    case create
    case refuse(String)
  }

  /// The default for `agent_task_limit`: open tasks agents asked for, per
  /// repository.
  public static let defaultLimit = 4

  /// What to do with an agent's request.
  /// - Parameters:
  ///   - repositoryTrusted: the repository is a trusted folder.
  ///   - projectTrusted: every project settings file with commands is
  ///     trusted (or there are none).
  ///   - agentTasks: open tasks agents asked for in the repository.
  ///   - fromAgentTask: the request comes from inside a task an agent asked
  ///     for: a chain, which always asks.
  ///   - movesChanges: it takes the main checkout's uncommitted files.
  public static func decide(
    setting: Setting, repositoryTrusted: Bool, projectTrusted: Bool, agentTasks: Int, limit: Int,
    fromAgentTask: Bool, movesChanges: Bool
  ) -> Decision {
    if setting == .off {
      return .refuse("Starting tasks from agents is turned off in Impulse's settings (agent_task_creation).")
    }
    if limit > 0, agentTasks >= limit {
      return .refuse(
        "This repository already has \(agentTasks) open task\(agentTasks == 1 ? "" : "s") agents asked for (the limit is \(limit)). Finish or archive one first, or ask the user to raise agent_task_limit.")
    }
    guard setting == .trustedProjects, repositoryTrusted, projectTrusted, !fromAgentTask, !movesChanges else {
      return .ask
    }
    return .create
  }

  /// The first terminal's command for a task started with `agent`, with
  /// the brief saved at `briefPath` as its first prompt for the agents whose
  /// command takes one. The brief is read from its file when the command
  /// runs, so a long or many-line brief is never typed into the shell (or
  /// kept in its history), after `--` so a brief that starts with a dash
  /// (a Markdown list) isn't taken for an option, with `command cat` so a
  /// function or alias called cat (`cat=bat`) can't change or lose it. Nil
  /// when the agent's command takes no prompt: the brief is typed into the
  /// agent once it's up.
  public static func command(agent: AgentKind, briefPath: String?, shell: ShellType) -> String? {
    guard let name = agent.names.first else { return nil }
    guard let briefPath else { return name }
    guard takesPrompt.contains(agent.id) else { return nil }
    let file = ShellQuoting.quoted(briefPath, for: shell)
    switch shell {
    case .fish: return "\(name) -- (command cat \(file) | string collect)"
    case .bash, .zsh: return "\(name) -- \"$(command cat \(file))\""
    }
  }

  /// Agents whose command line takes a first prompt as its argument and
  /// stays interactive.
  public static let takesPrompt: Set<String> = ["claude", "codex"]
}
