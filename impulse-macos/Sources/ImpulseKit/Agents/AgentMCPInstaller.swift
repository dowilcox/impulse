// Registering Impulse's MCP server (`impulse mcp`) with an agent. The entry
// runs the bundled CLI through $IMPULSE_CLI, which only Impulse terminals
// set, falling back to the app's own copy so the server still starts (with
// no tools) in any other terminal: registered for every project, it never
// shows up as broken.
//
// Claude Code keeps its servers in ~/.claude.json, a large file it rewrites
// itself while it runs, so Impulse only reads it (for the status) and
// changes it with `claude mcp add-json` / `claude mcp remove`. Codex's
// config.toml gets an `[mcp_servers.impulse]` table, with the same preview,
// backup and Remove as its notify hook.

import Foundation

public enum AgentMCPInstaller {
  public static let serverName = "impulse"
  /// What the server needs to find the window and pane it's called from.
  public static let forwardedVariables = ["IMPULSE_SOCKET", "IMPULSE_PANE_TOKEN", "IMPULSE_CLI"]
  /// Where the app usually is, when the running copy can't say.
  public static let defaultCLI = "/Applications/Impulse.app/Contents/Resources/bin/impulse"

  public typealias Status = AgentHookInstaller.Status

  /// The `sh -c` script that starts the server: the CLI of the Impulse the
  /// terminal belongs to, else `fallbackCLI`.
  public static func launcher(fallbackCLI: String) -> String {
    var escaped = ""
    for character in fallbackCLI {
      // Inside double quotes, these four are special to sh.
      if "\"\\$`".contains(character) { escaped.append("\\") }
      escaped.append(character)
    }
    return "exec \"${IMPULSE_CLI:-\(escaped)}\" mcp"
  }

  // MARK: Claude Code

  public enum ClaudeScope: String, Sendable {
    /// Every project (`--scope user`).
    case user
    /// This project only, for you alone (`--scope local`); never `.mcp.json`,
    /// which would give every clone a server that only works in Impulse.
    case local
  }

  /// The server's entry, as `claude mcp add-json` takes it.
  public static func claudeServer(fallbackCLI: String) -> [String: Any] {
    [
      "type": "stdio",
      "command": "sh",
      "args": ["-c", launcher(fallbackCLI: fallbackCLI)],
      "env": Dictionary(uniqueKeysWithValues: forwardedVariables.map { ($0, "${\($0):-}") }),
    ]
  }

  public static func claudeServerJSON(fallbackCLI: String) -> String {
    let data = (try? JSONSerialization.data(
      withJSONObject: claudeServer(fallbackCLI: fallbackCLI), options: [.sortedKeys, .withoutEscapingSlashes]))
      ?? Data("{}".utf8)
    return String(decoding: data, as: UTF8.self)
  }

  public static func claudeAddArguments(scope: ClaudeScope, fallbackCLI: String) -> [String] {
    ["mcp", "add-json", "--scope", scope.rawValue, serverName, claudeServerJSON(fallbackCLI: fallbackCLI)]
  }

  public static func claudeRemoveArguments(scope: ClaudeScope) -> [String] {
    ["mcp", "remove", "--scope", scope.rawValue, serverName]
  }

  /// Whether ~/.claude.json has Impulse's server for `scope` (`project`:
  /// the folder a local server belongs to): the current entry, an older
  /// one (another fallback path, a hand-edited entry), or none.
  public static func claudeStatus(_ json: Data?, scope: ClaudeScope, project: String?, fallbackCLI: String) -> Status {
    guard let json, let config = try? JSONSerialization.jsonObject(with: json) as? [String: Any] else {
      return .notInstalled
    }
    let servers: [String: Any]?
    switch scope {
    case .user:
      servers = config["mcpServers"] as? [String: Any]
    case .local:
      let projects = config["projects"] as? [String: Any]
      servers = project.flatMap { (projects?[$0] as? [String: Any])?["mcpServers"] as? [String: Any] }
    }
    guard let entry = servers?[serverName] as? [String: Any] else { return .notInstalled }
    let args = entry["args"] as? [String]
    return entry["command"] as? String == "sh" && args == ["-c", launcher(fallbackCLI: fallbackCLI)]
      ? .installed : .outdated
  }

  // MARK: Codex

  private static let codexComment = "# Impulse's MCP server: tasks, review comments, problems (no tools outside Impulse)."
  private static let codexHeader = "[mcp_servers.\(serverName)]"

  /// The table Impulse adds to config.toml.
  public static func codexTable(fallbackCLI: String) -> String {
    func quoted(_ text: String) -> String {
      "\"" + text.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"") + "\""
    }
    return [
      codexComment,
      codexHeader,
      "command = \"sh\"",
      "args = [\"-c\", \(quoted(launcher(fallbackCLI: fallbackCLI)))]",
      "env_vars = [\(forwardedVariables.map(quoted).joined(separator: ", "))]",
    ].joined(separator: "\n") + "\n"
  }

  /// The lines of `toml` that hold the `[mcp_servers.impulse]` table (with
  /// Impulse's comment above it, when it's there); nil without one.
  private static func codexTableRange(_ lines: [String]) -> (range: Range<Int>, isOurs: Bool)? {
    guard let header = lines.firstIndex(where: { $0.trimmingCharacters(in: .whitespaces) == codexHeader }) else {
      return nil
    }
    var end = header + 1
    while end < lines.count, !lines[end].trimmingCharacters(in: .whitespaces).hasPrefix("[") { end += 1 }
    // Blank lines before the next table aren't the table's, but the one
    // after it is (installing put it there).
    while end > header + 1, lines[end - 1].trimmingCharacters(in: .whitespaces).isEmpty { end -= 1 }
    let isOurs = header > 0 && lines[header - 1] == codexComment
    return ((isOurs ? header - 1 : header)..<end, isOurs)
  }

  public static func codexStatus(_ toml: String?, fallbackCLI: String) -> Status {
    guard let toml else { return .notInstalled }
    let lines = toml.components(separatedBy: "\n")
    guard let found = codexTableRange(lines) else { return .notInstalled }
    let table = lines[found.range].joined(separator: "\n") + "\n"
    return table == codexTable(fallbackCLI: fallbackCLI) ? .installed : .outdated
  }

  /// config.toml with Impulse's table at the end (an older one of Impulse's
  /// replaced). Refuses when an `[mcp_servers.impulse]` table Impulse
  /// didn't write is there.
  public static func installingCodexMCP(into toml: String?, fallbackCLI: String) -> Result<String, AgentHookInstaller.InstallError> {
    var text = toml ?? ""
    if let found = codexTableRange(text.components(separatedBy: "\n")) {
      guard found.isOurs else {
        return .failure(.notifyInUse("config.toml already has an [mcp_servers.impulse] table; change it by hand"))
      }
      text = removingCodexMCP(from: text) ?? text
    }
    let table = codexTable(fallbackCLI: fallbackCLI)
    if text.isEmpty { return .success(table) }
    if !text.hasSuffix("\n") { text += "\n" }
    return .success(text + "\n" + table)
  }

  /// config.toml without Impulse's table (and the blank line before it);
  /// nil when the table isn't Impulse's (or isn't there).
  public static func removingCodexMCP(from toml: String) -> String? {
    var lines = toml.components(separatedBy: "\n")
    guard let found = codexTableRange(lines), found.isOurs else { return nil }
    var range = found.range
    // The blank line installing put before it; at the top of the file, the
    // one after it instead.
    if range.lowerBound > 0, lines[range.lowerBound - 1].isEmpty {
      range = (range.lowerBound - 1)..<range.upperBound
    } else if range.lowerBound == 0, range.upperBound + 1 < lines.count, lines[range.upperBound].isEmpty {
      range = range.lowerBound..<(range.upperBound + 1)
    }
    lines.removeSubrange(range)
    return lines.joined(separator: "\n")
  }

  // MARK: Other agents

  /// The entry for agents Impulse doesn't configure itself, in the
  /// `mcpServers` form most of them read (their own config files differ).
  public static func genericSnippet(fallbackCLI: String) -> String {
    let object: [String: Any] = ["mcpServers": [serverName: claudeServer(fallbackCLI: fallbackCLI)]]
    let data = (try? JSONSerialization.data(
      withJSONObject: object, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])) ?? Data("{}".utf8)
    return String(decoding: data, as: UTF8.self)
  }
}
