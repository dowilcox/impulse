// The conversation between the `impulse` command-line tool and the running
// app: one JSON object per line over a Unix socket. Terminals inside
// Impulse get the socket path and a token naming their pane in the
// environment, so `impulse` can act on "this pane" (and agent hooks can
// report on it).

import Foundation

public struct ControlRequest: Codable, Equatable, Sendable {
  public var command: String
  /// The calling terminal's pane token (`IMPULSE_PANE_TOKEN`), if any.
  public var token: String?
  /// The caller's working directory, for relative paths.
  public var cwd: String?
  public var arguments: [String: String]
  /// Answer only when the action completes (e.g. the edited file's tab
  /// closes).
  public var wait: Bool

  public init(
    command: String, token: String? = nil, cwd: String? = nil, arguments: [String: String] = [:],
    wait: Bool = false
  ) {
    self.command = command
    self.token = token
    self.cwd = cwd
    self.arguments = arguments
    self.wait = wait
  }
}

public struct ControlResponse: Codable, Equatable, Sendable {
  public var ok: Bool
  public var message: String?

  public init(ok: Bool, message: String? = nil) {
    self.ok = ok
    self.message = message
  }
}

public enum ControlProtocol {
  public static let socketKey = "IMPULSE_SOCKET"
  public static let tokenKey = "IMPULSE_PANE_TOKEN"
  public static let cliKey = "IMPULSE_CLI"

  /// One line of JSON.
  public static func encodeLine<T: Encodable>(_ value: T) throws -> Data {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys]
    var data = try encoder.encode(value)
    data.append(0x0A)
    return data
  }

  public static func decode<T: Decodable>(_ type: T.Type, from line: Data) throws -> T {
    try JSONDecoder().decode(type, from: line)
  }

  public static let usage = """
    impulse — talk to the Impulse window this terminal is in

      impulse open <file>[:line[:column]]   open in the editor
      impulse edit <file>                   open, and wait until its tab closes ($EDITOR)
      impulse review [last-turn|uncommitted|staged|unstaged]
      impulse split [right|down] [command…] split this pane (optionally running a command)
      impulse tab [command…]                new tab in this workspace
      impulse notify <title> [message…]     flag this pane and notify
      impulse status <working|waiting|done|idle> [message…]
      impulse checkpoint [message…]         snapshot the repository
      impulse hook <claude|codex> [event]   for agent hooks (reads hook input on stdin)

    Outside an Impulse terminal, `impulse hook` does nothing and exits 0.
    """

  public struct UsageError: Error, Equatable {
    public let message: String
    public init(_ message: String) { self.message = message }
  }

  /// Turn command-line arguments into a request. `stdin` is read only by
  /// commands that take input (agent hooks).
  public static func request(
    arguments args: [String], environment: [String: String], cwd: String,
    stdin: () -> Data? = { nil }
  ) -> Result<ControlRequest, UsageError> {
    guard let command = args.first else { return .failure(UsageError(usage)) }
    let rest = Array(args.dropFirst())
    var request = ControlRequest(
      command: command, token: environment[tokenKey], cwd: cwd, arguments: [:])

    switch command {
    case "open", "edit":
      guard let target = rest.first else { return .failure(UsageError("usage: impulse \(command) <file>")) }
      let (path, line, column) = splitLocation(target)
      request.command = "open"
      request.arguments["path"] = absolute(path, cwd: cwd)
      if let line { request.arguments["line"] = String(line) }
      if let column { request.arguments["column"] = String(column) }
      request.wait = command == "edit"

    case "review":
      let scope = rest.first ?? "uncommitted"
      guard ["last-turn", "uncommitted", "staged", "unstaged"].contains(scope) else {
        return .failure(UsageError("usage: impulse review [last-turn|uncommitted|staged|unstaged]"))
      }
      request.arguments["scope"] = scope

    case "split":
      var words = rest
      var direction = "right"
      if let first = words.first, first == "right" || first == "down" {
        direction = first
        words.removeFirst()
      }
      request.arguments["direction"] = direction
      if !words.isEmpty { request.arguments["command"] = words.joined(separator: " ") }

    case "tab":
      if !rest.isEmpty { request.arguments["command"] = rest.joined(separator: " ") }

    case "notify":
      guard let title = rest.first else { return .failure(UsageError("usage: impulse notify <title> [message…]")) }
      request.arguments["title"] = title
      request.arguments["message"] = rest.dropFirst().joined(separator: " ")

    case "status":
      guard let state = rest.first, ["working", "waiting", "done", "idle"].contains(state) else {
        return .failure(UsageError("usage: impulse status <working|waiting|done|idle> [message…]"))
      }
      request.arguments["state"] = state
      request.arguments["message"] = rest.dropFirst().joined(separator: " ")

    case "checkpoint":
      request.arguments["message"] = rest.joined(separator: " ")

    case "hook":
      guard let agent = rest.first else { return .failure(UsageError("usage: impulse hook <claude|codex> [event]")) }
      request.arguments["agent"] = agent
      // An explicit event name, and/or a JSON payload: Codex passes its
      // notification as the last argument; Claude Code writes hook input to
      // stdin.
      var event: String?
      var payload: Data?
      for word in rest.dropFirst() {
        if word.hasPrefix("{") { payload = Data(word.utf8) } else if event == nil { event = word }
      }
      if payload == nil, agent == "claude" { payload = stdin() }
      var message: String?
      if let payload,
        let object = try? JSONSerialization.jsonObject(with: payload) as? [String: Any]
      {
        event = event ?? object["hook_event_name"] as? String ?? object["type"] as? String
        message = object["message"] as? String ?? object["last-assistant-message"] as? String
        if let session = object["session_id"] as? String ?? object["thread-id"] as? String {
          request.arguments["session"] = session
        }
      }
      guard let event, !event.isEmpty else {
        return .failure(UsageError("usage: impulse hook <claude|codex> <event>"))
      }
      request.arguments["event"] = event
      if let message { request.arguments["message"] = message }

    case "-h", "--help", "help":
      return .failure(UsageError(usage))

    default:
      return .failure(UsageError("impulse: unknown command '\(command)'\n\n" + usage))
    }
    return .success(request)
  }

  /// "src/a.swift:12:3" → ("src/a.swift", 12, 3). A trailing non-number
  /// isn't a position ("C:" style names stay intact).
  static func splitLocation(_ target: String) -> (String, Int?, Int?) {
    var parts = target.components(separatedBy: ":")
    var numbers: [Int] = []
    while parts.count > 1, let last = parts.last, let value = Int(last), numbers.count < 2 {
      numbers.insert(value, at: 0)
      parts.removeLast()
    }
    let path = parts.joined(separator: ":")
    return (path, numbers.first, numbers.count > 1 ? numbers[1] : nil)
  }

  static func absolute(_ path: String, cwd: String) -> String {
    let expanded = (path as NSString).expandingTildeInPath
    if expanded.hasPrefix("/") { return (expanded as NSString).standardizingPath }
    return ((cwd as NSString).appendingPathComponent(expanded) as NSString).standardizingPath
  }

  /// Default socket location when the environment doesn't say.
  public static func defaultSocketPath(home: String = NSHomeDirectory(), dev: Bool = false) -> String {
    "\(home)/Library/Application Support/\(dev ? "impulse-dev" : "impulse")/impulse.sock"
  }
}
