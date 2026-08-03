import Foundation

// Ported from impulse-core/src/close_risk.rs.

public enum CloseRiskAction: String, Codable {
  case quit
  case closeWindow = "close_window"
  case closeTab = "close_tab"
}

/// One command that is currently running in a terminal.
public struct RunningCommandRisk: Codable, Equatable {
  public var command: String?
  public var cwd: String?
  public var startedAtMs: UInt64

  public init(command: String? = nil, cwd: String? = nil, startedAtMs: UInt64 = 0) {
    self.command = command
    self.cwd = cwd
    self.startedAtMs = startedAtMs
  }

  enum CodingKeys: String, CodingKey {
    case command
    case cwd
    case startedAtMs = "started_at_ms"
  }
}

/// Inputs a frontend can collect before closing a window or quitting.
public struct CloseRiskInput: Codable, Equatable {
  public var action: CloseRiskAction
  public var unsavedEditorCount: Int
  public var runningTerminalProcessCount: Int
  public var runningCommands: [RunningCommandRisk]
  public var nowMs: UInt64
  public var longCommandThresholdSeconds: UInt64

  public init(
    action: CloseRiskAction,
    unsavedEditorCount: Int = 0,
    runningTerminalProcessCount: Int = 0,
    runningCommands: [RunningCommandRisk] = [],
    nowMs: UInt64 = 0,
    longCommandThresholdSeconds: UInt64 = 30
  ) {
    self.action = action
    self.unsavedEditorCount = unsavedEditorCount
    self.runningTerminalProcessCount = runningTerminalProcessCount
    self.runningCommands = runningCommands
    self.nowMs = nowMs
    self.longCommandThresholdSeconds = longCommandThresholdSeconds
  }

  public init(from decoder: Decoder) throws {
    let container = try decoder.container(keyedBy: CodingKeys.self)
    action = try container.decode(CloseRiskAction.self, forKey: .action)
    unsavedEditorCount = try container.decodeIfPresent(Int.self, forKey: .unsavedEditorCount) ?? 0
    runningTerminalProcessCount =
      try container.decodeIfPresent(Int.self, forKey: .runningTerminalProcessCount) ?? 0
    runningCommands =
      try container.decodeIfPresent([RunningCommandRisk].self, forKey: .runningCommands) ?? []
    nowMs = try container.decodeIfPresent(UInt64.self, forKey: .nowMs) ?? 0
    longCommandThresholdSeconds =
      try container.decodeIfPresent(UInt64.self, forKey: .longCommandThresholdSeconds) ?? 30
  }

  enum CodingKeys: String, CodingKey {
    case action
    case unsavedEditorCount = "unsaved_editor_count"
    case runningTerminalProcessCount = "running_terminal_process_count"
    case runningCommands = "running_commands"
    case nowMs = "now_ms"
    case longCommandThresholdSeconds = "long_command_threshold_seconds"
  }

  public func summarize() -> CloseRiskSummary {
    CloseRisk.summarize(self)
  }
}

/// One normalized command entry suitable for display.
public struct CloseRiskCommandSummary: Codable, Equatable {
  public var command: String
  public var cwd: String?
  public var durationSeconds: UInt64
  public var isLongRunning: Bool

  enum CodingKeys: String, CodingKey {
    case command
    case cwd
    case durationSeconds = "duration_seconds"
    case isLongRunning = "is_long_running"
  }
}

/// Summary used by frontends to decide whether to prompt and what to show.
public struct CloseRiskSummary: Codable, Equatable {
  public var hasRisk: Bool
  public var title: String
  public var informativeText: String
  public var detailLines: [String]
  public var destructiveActionTitle: String
  public var cancelTitle: String
  public var unsavedEditorCount: Int
  public var runningTerminalProcessCount: Int
  public var runningCommandCount: Int
  public var longRunningCommandCount: Int
  public var commands: [CloseRiskCommandSummary]

  enum CodingKeys: String, CodingKey {
    case hasRisk = "has_risk"
    case title
    case informativeText = "informative_text"
    case detailLines = "detail_lines"
    case destructiveActionTitle = "destructive_action_title"
    case cancelTitle = "cancel_title"
    case unsavedEditorCount = "unsaved_editor_count"
    case runningTerminalProcessCount = "running_terminal_process_count"
    case runningCommandCount = "running_command_count"
    case longRunningCommandCount = "long_running_command_count"
    case commands
  }
}

public enum CloseRisk {
  public static func summarize(_ input: CloseRiskInput) -> CloseRiskSummary {
    let threshold = max(input.longCommandThresholdSeconds, 1)
    let commands = summarizeCommands(input: input, threshold: threshold)
    let runningCommandCount = commands.count
    let longRunningCommandCount = commands.filter(\.isLongRunning).count
    let hasRisk =
      input.unsavedEditorCount > 0 || input.runningTerminalProcessCount > 0
      || runningCommandCount > 0

    let destructiveActionTitle: String
    switch input.action {
    case .quit: destructiveActionTitle = "Quit"
    case .closeWindow: destructiveActionTitle = "Close Window"
    case .closeTab: destructiveActionTitle = "Close Tab"
    }

    if !hasRisk {
      return CloseRiskSummary(
        hasRisk: false,
        title: "",
        informativeText: "",
        detailLines: [],
        destructiveActionTitle: destructiveActionTitle,
        cancelTitle: "Cancel",
        unsavedEditorCount: 0,
        runningTerminalProcessCount: 0,
        runningCommandCount: 0,
        longRunningCommandCount: 0,
        commands: commands
      )
    }

    return CloseRiskSummary(
      hasRisk: true,
      title: closeTitle(input: input),
      informativeText: closeInformativeText(
        input: input, runningCommandCount: runningCommandCount),
      detailLines: closeDetailLines(input: input, commands: commands),
      destructiveActionTitle: destructiveActionTitle,
      cancelTitle: "Cancel",
      unsavedEditorCount: input.unsavedEditorCount,
      runningTerminalProcessCount: input.runningTerminalProcessCount,
      runningCommandCount: runningCommandCount,
      longRunningCommandCount: longRunningCommandCount,
      commands: commands
    )
  }

  private static func summarizeCommands(input: CloseRiskInput, threshold: UInt64)
    -> [CloseRiskCommandSummary]
  {
    input.runningCommands.map { command in
      let durationSeconds =
        input.nowMs >= command.startedAtMs ? (input.nowMs - command.startedAtMs) / 1000 : 0
      return CloseRiskCommandSummary(
        command: displayCommand(command.command),
        cwd: command.cwd,
        durationSeconds: durationSeconds,
        isLongRunning: durationSeconds >= threshold
      )
    }
  }

  private static func closeTitle(input: CloseRiskInput) -> String {
    let action: String
    switch input.action {
    case .quit: action = "Quit Impulse"
    case .closeWindow: action = "Close window"
    case .closeTab: action = "Close tab"
    }
    let hasUnsaved = input.unsavedEditorCount > 0
    let hasTerminal = input.runningTerminalProcessCount > 0 || !input.runningCommands.isEmpty

    switch (hasUnsaved, hasTerminal) {
    case (true, true): return "\(action) with unsaved changes and running terminal work?"
    case (true, false): return "\(action) with unsaved changes?"
    case (false, true): return "\(action) with running terminal work?"
    case (false, false): return ""
    }
  }

  private static func closeInformativeText(input: CloseRiskInput, runningCommandCount: Int)
    -> String
  {
    var sentences: [String] = []
    if input.unsavedEditorCount == 1 {
      sentences.append("1 editor has unsaved changes that may be lost.")
    } else if input.unsavedEditorCount > 1 {
      sentences.append("\(input.unsavedEditorCount) editors have unsaved changes that may be lost.")
    }

    if input.runningTerminalProcessCount == 1 {
      sentences.append("1 terminal process will be terminated.")
    } else if input.runningTerminalProcessCount > 1 {
      sentences.append("\(input.runningTerminalProcessCount) terminal processes will be terminated.")
    }

    if runningCommandCount == 1 {
      sentences.append("1 running command will be stopped.")
    } else if runningCommandCount > 1 {
      sentences.append("\(runningCommandCount) running commands will be stopped.")
    }

    return sentences.joined(separator: " ")
  }

  private static func closeDetailLines(input: CloseRiskInput, commands: [CloseRiskCommandSummary])
    -> [String]
  {
    var lines: [String] = []
    if input.unsavedEditorCount > 0 {
      lines.append(pluralLine(count: input.unsavedEditorCount, noun: "unsaved editor"))
    }
    if input.runningTerminalProcessCount > 0 {
      lines.append(
        pluralLine(count: input.runningTerminalProcessCount, noun: "running terminal process"))
    }

    for command in commands.prefix(3) {
      lines.append("\(command.command) running for \(formatDuration(command.durationSeconds))")
    }
    if commands.count > 3 {
      lines.append("\(commands.count - 3) more running commands")
    }

    return lines
  }

  private static func pluralLine(count: Int, noun: String) -> String {
    if count == 1 {
      return "1 \(noun)"
    } else if noun.hasSuffix("process") {
      return "\(count) \(noun)es"
    } else {
      return "\(count) \(noun)s"
    }
  }

  private static func displayCommand(_ command: String?) -> String {
    let trimmed = (command ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    if trimmed.isEmpty {
      return "Running command"
    }
    let maxCommandChars = 80
    if trimmed.count <= maxCommandChars {
      return trimmed
    }
    return String(trimmed.prefix(maxCommandChars - 3)) + "..."
  }

  static func formatDuration(_ seconds: UInt64) -> String {
    if seconds < 60 {
      return "\(seconds)s"
    }
    let minutes = seconds / 60
    let remainingSeconds = seconds % 60
    if minutes < 60 {
      return String(format: "%dm %02ds", minutes, remainingSeconds)
    }
    let hours = minutes / 60
    let remainingMinutes = minutes % 60
    return String(format: "%dh %02dm", hours, remainingMinutes)
  }
}
