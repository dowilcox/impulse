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
// backup and Remove as its notify hook. opencode's config (JSON, or JSONC
// with comments) gets an `impulse` entry under `mcp`, edited in place so
// nothing else in the file changes. (opencode only writes that file when
// asked to, and `opencode mcp add` can't remove a server.)

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

  /// A line without its line ending, surrounding blanks and trailing
  /// comment (a `#` outside quotes is rare enough in these lines to cut at).
  private static func bare(_ line: String) -> String {
    let content = line.split(separator: "#", maxSplits: 1, omittingEmptySubsequences: false).first.map(String.init) ?? ""
    return content.trimmingCharacters(in: .whitespacesAndNewlines)
  }

  /// `[mcp_servers.impulse]`, however it's quoted or spaced.
  private static func isImpulseHeader(_ line: String) -> Bool {
    let compact = bare(line).filter { $0 != " " && $0 != "\t" }
    return ["[mcp_servers.impulse]", "[mcp_servers.\"impulse\"]", "[mcp_servers.'impulse']"].contains(compact)
  }

  /// The lines of `toml` that hold the `[mcp_servers.impulse]` table (with
  /// Impulse's comment above it, when it's there); nil without one.
  private static func codexTableRange(_ lines: [String]) -> (range: Range<Int>, isOurs: Bool)? {
    guard let header = lines.firstIndex(where: isImpulseHeader) else { return nil }
    var end = header + 1
    while end < lines.count, !lines[end].trimmingCharacters(in: .whitespaces).hasPrefix("[") { end += 1 }
    // Blank lines before the next table aren't the table's, but the one
    // after it is (installing put it there).
    while end > header + 1, lines[end - 1].trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { end -= 1 }
    let isOurs =
      header > 0 && lines[header - 1].trimmingCharacters(in: .whitespacesAndNewlines) == codexComment
      && lines[header].trimmingCharacters(in: .whitespacesAndNewlines) == codexHeader
    return ((isOurs ? header - 1 : header)..<end, isOurs)
  }

  /// `mcp_servers.impulse` defined some other way than its own table: a
  /// dotted key, or `impulse = { … }` in `[mcp_servers]`. A second
  /// definition would stop Codex from reading the file.
  private static func definesServerElsewhere(_ lines: [String]) -> Bool {
    var inServers = false
    for line in lines {
      let content = bare(line)
      if content.hasPrefix("[") {
        inServers = content.filter { $0 != " " } == "[mcp_servers]"
        continue
      }
      let key = content.filter { $0 != " " && $0 != "\t" }
      if key.hasPrefix("mcp_servers.impulse") || key.hasPrefix("mcp_servers.\"impulse\"") { return true }
      if inServers, key.hasPrefix("impulse=") || key.hasPrefix("impulse.") || key.hasPrefix("\"impulse\"") {
        return true
      }
    }
    return false
  }

  public static func codexStatus(_ toml: String?, fallbackCLI: String) -> Status {
    guard let toml else { return .notInstalled }
    let lines = toml.components(separatedBy: "\n")
    guard let found = codexTableRange(lines) else {
      return definesServerElsewhere(lines) ? .outdated : .notInstalled
    }
    let table = lines[found.range].map { $0.trimmingCharacters(in: .newlines) }.joined(separator: "\n") + "\n"
    return found.isOurs && table == codexTable(fallbackCLI: fallbackCLI) ? .installed : .outdated
  }

  /// config.toml with Impulse's table at the end (an older one of Impulse's
  /// replaced). Refuses when `mcp_servers.impulse` is defined in a way
  /// Impulse didn't write.
  public static func installingCodexMCP(into toml: String?, fallbackCLI: String) -> Result<String, AgentHookInstaller.InstallError> {
    var text = toml ?? ""
    let lines = text.components(separatedBy: "\n")
    if let found = codexTableRange(lines) {
      guard found.isOurs else {
        return .failure(.notifyInUse("config.toml already has an [mcp_servers.impulse] table; change it by hand"))
      }
      text = removingCodexMCP(from: text) ?? text
    } else if definesServerElsewhere(lines) {
      return .failure(.notifyInUse("config.toml already defines mcp_servers.impulse; change it by hand"))
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
    let isBlank = { (index: Int) in lines[index].trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    if range.lowerBound > 0, isBlank(range.lowerBound - 1) {
      range = (range.lowerBound - 1)..<range.upperBound
    } else if range.lowerBound == 0, range.upperBound + 1 < lines.count, isBlank(range.upperBound) {
      range = range.lowerBound..<(range.upperBound + 1)
    }
    lines.removeSubrange(range)
    return lines.joined(separator: "\n")
  }

  // MARK: opencode

  /// opencode's global config folder: `$XDG_CONFIG_HOME/opencode`, else
  /// `~/.config/opencode` (on macOS too).
  public static func opencodeConfigDirectory(environment: [String: String], home: String) -> String {
    if let xdg = environment["XDG_CONFIG_HOME"], xdg.hasPrefix("/") {
      return (xdg as NSString).appendingPathComponent("opencode")
    }
    return (home as NSString).appendingPathComponent(".config/opencode")
  }

  /// The global config file opencode itself would write to (it reads all
  /// three): opencode.jsonc, opencode.json or config.json, whichever exists
  /// first; a new opencode.json when there's none.
  public static func opencodeConfigFile(
    directory: String, exists: (String) -> Bool = { FileManager.default.fileExists(atPath: $0) }
  ) -> String {
    let candidates = ["opencode.jsonc", "opencode.json", "config.json"].map {
      (directory as NSString).appendingPathComponent($0)
    }
    return candidates.first(where: exists) ?? candidates[1]
  }

  /// The server's entry under `mcp`. opencode starts local servers with its
  /// own environment, the terminal's, so nothing needs forwarding.
  static func opencodeServer(fallbackCLI: String) -> KeyValuePairs<String, Any> {
    ["type": "local", "command": ["sh", "-c", launcher(fallbackCLI: fallbackCLI)]]
  }

  /// What to add by hand when Impulse can't edit the file.
  public static func opencodeSnippet(fallbackCLI: String) -> String {
    let servers: KeyValuePairs<String, Any> = [serverName: opencodeServer(fallbackCLI: fallbackCLI)]
    return "{\n  " + JSONCEdit.format("mcp", servers, indent: "  ", unit: "  ") + "\n}"
  }

  /// The `mcp` object and Impulse's entry in it, if the text has them.
  private static func opencodeEntry(_ bytes: [UInt8]) throws -> (
    root: JSONCEdit.Object, mcp: JSONCEdit.Member?, servers: JSONCEdit.Object?, entry: JSONCEdit.Member?
  ) {
    let root = try JSONCEdit.root(bytes)
    guard let mcp = root.member("mcp") else { return (root, nil, nil, nil) }
    guard bytes[mcp.valueStart] == UInt8(ascii: "{") else { throw JSONCEdit.Malformed() }
    let servers = try JSONCEdit.object(bytes, mcp.valueStart)
    return (root, mcp, servers, servers.member(serverName))
  }

  /// An entry Impulse wrote (perhaps with another fallback path), rather
  /// than another server someone named impulse.
  private static func isOurOpencodeEntry(_ value: Any?) -> Bool {
    guard let entry = value as? [String: Any], entry["type"] as? String == "local",
      let command = entry["command"] as? [String], command.count == 3, command[0] == "sh", command[1] == "-c"
    else { return false }
    return command[2].contains("IMPULSE_CLI") && command[2].hasSuffix(" mcp")
  }

  public static func opencodeStatus(_ text: String?, fallbackCLI: String) -> Status {
    let bytes = Array((text ?? "").utf8)
    guard let found = try? opencodeEntry(bytes), let entry = found.entry else { return .notInstalled }
    let value = JSONCEdit.value(bytes, entry.valueStart, entry.valueEnd) as? NSDictionary
    let current: NSDictionary = [
      "type": "local", "command": ["sh", "-c", launcher(fallbackCLI: fallbackCLI)],
    ]
    return value == current ? .installed : .outdated
  }

  /// The config with Impulse's server under `mcp` (an older entry of
  /// Impulse's replaced), and nothing else changed. Refuses a file it can't
  /// read as JSONC, and a server named impulse that isn't Impulse's.
  public static func installingOpencodeMCP(into text: String?, fallbackCLI: String) -> Result<
    String, AgentHookInstaller.InstallError
  > {
    let server = opencodeServer(fallbackCLI: fallbackCLI)
    guard let text, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
      // A new file: the schema line opencode would otherwise add itself.
      let servers: KeyValuePairs<String, Any> = [serverName: server]
      return .success(
        "{\n  \"$schema\": \"https://opencode.ai/config.json\",\n  "
          + JSONCEdit.format("mcp", servers, indent: "  ", unit: "  ") + "\n}\n")
    }
    let bytes = Array(text.utf8)
    guard let found = try? opencodeEntry(bytes) else {
      return .failure(
        .unreadable(
          "Impulse can't edit this file without risking the rest of it (it isn't a JSON object with an \"mcp\" object, comments allowed). Add Impulse's server to it yourself:\n\n"
            + opencodeSnippet(fallbackCLI: fallbackCLI)))
    }
    let unit = JSONCEdit.indentUnit(bytes)
    let changes: [JSONCEdit.Change]
    if let servers = found.servers {
      if let entry = found.entry {
        guard isOurOpencodeEntry(JSONCEdit.value(bytes, entry.valueStart, entry.valueEnd)) else {
          return .failure(.notifyInUse("opencode's config already has an MCP server named impulse; change it by hand"))
        }
        let indent = JSONCEdit.lineIndent(bytes, entry.keyStart).indent
        changes = [(entry.valueStart..<entry.valueEnd, JSONCEdit.format(server, indent: indent, unit: unit))]
      } else {
        changes = JSONCEdit.inserting(into: servers, bytes, unit: unit) {
          JSONCEdit.format(serverName, server, indent: $0, unit: unit)
        }
      }
    } else {
      let servers: KeyValuePairs<String, Any> = [serverName: server]
      changes = JSONCEdit.inserting(into: found.root, bytes, unit: unit) {
        JSONCEdit.format("mcp", servers, indent: $0, unit: unit)
      }
    }
    return .success(String(decoding: JSONCEdit.applying(changes, to: bytes), as: UTF8.self))
  }

  /// The config without Impulse's server (and without an `mcp` it leaves
  /// empty); nil when the entry isn't Impulse's (or isn't there).
  public static func removingOpencodeMCP(from text: String) -> String? {
    let bytes = Array(text.utf8)
    guard let found = try? opencodeEntry(bytes), let servers = found.servers, let entry = found.entry,
      isOurOpencodeEntry(JSONCEdit.value(bytes, entry.valueStart, entry.valueEnd))
    else { return nil }
    var result = JSONCEdit.applying(JSONCEdit.removing(entry, from: servers, bytes), to: bytes)
    if let left = try? opencodeEntry(result), let mcp = left.mcp, let servers = left.servers, servers.members.isEmpty,
      result[(servers.open + 1)..<servers.close].allSatisfy({ [0x20, 0x09, 0x0A, 0x0D].contains($0) })
    {
      result = JSONCEdit.applying(JSONCEdit.removing(mcp, from: left.root, result), to: result)
    }
    return String(decoding: result, as: UTF8.self)
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
