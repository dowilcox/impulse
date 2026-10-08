// Edits agent configuration so the agent tells Impulse what it's doing:
// Claude Code hooks in settings.json, Codex's `notify` program in
// config.toml. Every hook calls the bundled CLI through $IMPULSE_CLI, which
// only Impulse terminals set — anywhere else the hook is a no-op.

import Foundation

public enum AgentHookInstaller {
  /// The Claude Code events Impulse listens to, each with the tools a tool
  /// event is limited to: before shell commands (a merge of a branch still
  /// being worked on asks first) and after edits and shell commands (notes
  /// about files other tasks change, and dependencies that changed).
  public static let claudeHooks: [(event: String, matcher: String?)] = [
    ("SessionStart", nil), ("UserPromptSubmit", nil), ("Notification", nil), ("Stop", nil),
    ("PreToolUse", "Bash"), ("PostToolUse", "Edit|Write|MultiEdit|NotebookEdit|Bash"),
  ]

  public static var claudeEvents: [String] { claudeHooks.map(\.event) }

  /// Whether Impulse's Claude Code hooks are in a settings file: all of them,
  /// some (installed by an older Impulse), or none.
  public enum Status: Equatable, Sendable { case notInstalled, outdated, installed }

  /// The shell command each Claude Code hook runs.
  public static let claudeCommand = #"[ -n "$IMPULSE_CLI" ] && "$IMPULSE_CLI" hook claude || true"#

  /// Codex appends its notification JSON as the last argument.
  public static let codexNotify = [
    "sh", "-c", #"[ -n "$IMPULSE_CLI" ] && "$IMPULSE_CLI" hook codex "$1" || true"#, "impulse-hook",
  ]

  public enum InstallError: Error, Equatable {
    /// The existing file isn't the JSON/TOML shape expected.
    case unreadable(String)
    /// Codex already runs another notify program.
    case notifyInUse(String)
  }

  // MARK: Claude Code (settings.json)

  public static func claudeHooksInstalled(_ json: Data?) -> Bool {
    claudeHookStatus(json) == .installed
  }

  public static func claudeHookStatus(_ json: Data?) -> Status {
    guard let json, let settings = try? JSONSerialization.jsonObject(with: json) as? [String: Any],
      let hooks = settings["hooks"] as? [String: Any]
    else { return .notInstalled }
    let present = claudeEvents.filter { event in
      (hooks[event] as? [[String: Any]])?.contains(where: isOurClaudeEntry) == true
    }
    return present.isEmpty ? .notInstalled : present.count == claudeEvents.count ? .installed : .outdated
  }

  /// settings.json with Impulse's hooks added (other settings and hooks
  /// kept). Already-installed hooks aren't duplicated.
  public static func installingClaudeHooks(into json: Data?) -> Result<Data, InstallError> {
    var settings: [String: Any] = [:]
    if let json, !json.isEmpty {
      guard let object = try? JSONSerialization.jsonObject(with: json) as? [String: Any] else {
        return .failure(.unreadable("settings.json isn't a JSON object"))
      }
      settings = object
    }
    var hooks = settings["hooks"] as? [String: Any] ?? [:]
    if settings["hooks"] != nil, !(settings["hooks"] is [String: Any]) {
      return .failure(.unreadable("\"hooks\" isn't an object"))
    }
    for (event, matcher) in claudeHooks {
      var entries = hooks[event] as? [[String: Any]] ?? []
      if !entries.contains(where: isOurClaudeEntry) {
        var entry: [String: Any] = ["hooks": [["type": "command", "command": claudeCommand]]]
        if let matcher { entry["matcher"] = matcher }
        entries.append(entry)
      }
      hooks[event] = entries
    }
    settings["hooks"] = hooks
    return .success(serialize(settings))
  }

  /// settings.json without Impulse's hooks (empty events and an empty
  /// "hooks" object are removed).
  public static func removingClaudeHooks(from json: Data) -> Result<Data, InstallError> {
    guard var settings = try? JSONSerialization.jsonObject(with: json) as? [String: Any] else {
      return .failure(.unreadable("settings.json isn't a JSON object"))
    }
    guard var hooks = settings["hooks"] as? [String: Any] else { return .success(json) }
    for (event, value) in hooks {
      guard let entries = value as? [[String: Any]] else { continue }
      let kept = entries.compactMap { entry -> [String: Any]? in
        guard var inner = entry["hooks"] as? [[String: Any]] else { return entry }
        inner.removeAll { ($0["command"] as? String) == claudeCommand }
        if inner.isEmpty { return nil }
        var copy = entry
        copy["hooks"] = inner
        return copy
      }
      hooks[event] = kept.isEmpty ? nil : kept
    }
    settings["hooks"] = hooks.isEmpty ? nil : hooks
    return .success(serialize(settings))
  }

  private static func isOurClaudeEntry(_ entry: [String: Any]) -> Bool {
    (entry["hooks"] as? [[String: Any]])?.contains { ($0["command"] as? String) == claudeCommand }
      == true
  }

  private static func serialize(_ object: [String: Any]) -> Data {
    var data =
      (try? JSONSerialization.data(
        withJSONObject: object, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]))
      ?? Data("{}".utf8)
    data.append(0x0A)
    return data
  }

  // MARK: Codex (config.toml)

  public static func codexNotifyInstalled(_ toml: String?) -> Bool {
    toml?.contains(#"hook codex"#) == true
  }

  /// config.toml with Impulse as the `notify` program. Top-level keys must
  /// come before any table, so it's added at the top. Refuses when another
  /// notify program is configured.
  public static func installingCodexNotify(into toml: String?) -> Result<String, InstallError> {
    let text = toml ?? ""
    if codexNotifyInstalled(text) { return .success(text) }
    let hasNotify = text.split(whereSeparator: \.isNewline).contains { line in
      line.trimmingCharacters(in: .whitespaces).hasPrefix("notify")
        && line.contains("=")
    }
    if hasNotify {
      return .failure(.notifyInUse("config.toml already sets notify; add Impulse's command by hand"))
    }
    let array = codexNotify.map { "\"" + $0.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"") + "\"" }
      .joined(separator: ", ")
    let line = "\(codexComment)\nnotify = [\(array)]\n"
    return .success(text.isEmpty ? line : line + "\n" + text)
  }

  /// config.toml without Impulse's `notify` line, the comment above it and
  /// the blank line installing put after it. Another program's `notify` is
  /// left alone.
  public static func removingCodexNotify(from toml: String) -> String {
    var lines = toml.components(separatedBy: "\n")
    guard
      var index = lines.firstIndex(where: {
        $0.trimmingCharacters(in: .whitespaces).hasPrefix("notify") && $0.contains("hook codex")
      })
    else { return toml }
    lines.remove(at: index)
    if index > 0, lines[index - 1] == codexComment {
      index -= 1
      lines.remove(at: index)
    }
    if index == 0, lines.count > 1, lines[0].isEmpty { lines.removeFirst() }
    return lines.joined(separator: "\n")
  }

  private static let codexComment = "# Tells Impulse when Codex finishes a turn (no-op outside Impulse)."

  // MARK: Preview

  /// A unified-style line diff of a config change, for confirmation.
  public static func diffLines(before: String, after: String) -> [String] {
    let old = before.components(separatedBy: "\n")
    let new = after.components(separatedBy: "\n")
    let difference = new.difference(from: old)
    var removed = Set<Int>()
    var inserted = Set<Int>()
    for change in difference {
      switch change {
      case .remove(let offset, _, _): removed.insert(offset)
      case .insert(let offset, _, _): inserted.insert(offset)
      }
    }
    var lines: [String] = []
    var i = 0
    var j = 0
    while i < old.count || j < new.count {
      if i < old.count, removed.contains(i) {
        lines.append("- " + old[i])
        i += 1
      } else if j < new.count, inserted.contains(j) {
        lines.append("+ " + new[j])
        j += 1
      } else {
        if i < old.count { lines.append("  " + old[i]) }
        i += 1
        j += 1
      }
    }
    return lines
  }
}
