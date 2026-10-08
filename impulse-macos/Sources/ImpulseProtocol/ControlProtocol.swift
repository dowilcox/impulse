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
                                            report an agent state from a program running
                                            in this pane (it lasts while that program runs)
      impulse checkpoint [message…]         snapshot the repository
      impulse tasks [--json]                this repository's workspaces: branches, agents,
                                            uncommitted files, files shared with yours
      impulse tasks wait <task>             wait until that task's agent stops working
      impulse hook <claude|codex> [event]   for agent hooks (reads hook input on stdin)

    Outside an Impulse terminal, `impulse hook` does nothing and exits 0.
    """

  public struct UsageError: Error, Equatable {
    public let message: String
    /// `-h` / `--help`: the usage was asked for (not a mistake).
    public let isHelp: Bool
    public init(_ message: String, isHelp: Bool = false) {
      self.message = message
      self.isHelp = isHelp
    }
  }

  /// A word where an option could go: `-h`/`--help` asks for the usage, and
  /// any other `-x` is an option `impulse` doesn't have (rather than, say, a
  /// file named `--wait`). A lone `-` isn't an option.
  static func optionError(_ word: String?, command: String) -> UsageError? {
    guard let word, word.hasPrefix("-"), word != "-" else { return nil }
    if word == "-h" || word == "--help" { return UsageError(usage, isHelp: true) }
    return UsageError("impulse \(command): unknown option '\(word)'\n\n" + usage)
  }

  /// A command's arguments without the `--` that ends options, or the first
  /// option among them as an error. Only the first word is checked unless
  /// `checkAll` (the rest may be a command with options of its own).
  static func operands(_ args: [String], command: String, checkAll: Bool = false) -> Result<[String], UsageError> {
    guard let first = args.first else { return .success([]) }
    if first == "--" { return .success(Array(args.dropFirst())) }
    guard checkAll else {
      return optionError(first, command: command).map { .failure($0) } ?? .success(args)
    }
    var operands: [String] = []
    var optionsEnded = false
    for word in args {
      if !optionsEnded, word == "--" {
        optionsEnded = true
        continue
      }
      if !optionsEnded, let error = optionError(word, command: command) { return .failure(error) }
      operands.append(word)
    }
    return .success(operands)
  }

  /// Turn command-line arguments into a request. `stdin` is read only by
  /// commands that take input (agent hooks).
  public static func request(
    arguments args: [String], environment: [String: String], cwd: String,
    stdin: () -> Data? = { nil }
  ) -> Result<ControlRequest, UsageError> {
    guard let command = args.first else { return .failure(UsageError(usage)) }
    var rest = Array(args.dropFirst())
    var request = ControlRequest(
      command: command, token: environment[tokenKey], cwd: cwd, arguments: [:])

    if command != "hook", let first = rest.first, first == "-h" || first == "--help" {
      return .failure(UsageError(usage, isHelp: true))
    }
    // Commands whose arguments are free words: an option among them is a
    // mistake (`impulse edit --wait file` mustn't open a file named
    // `--wait`), and `--` lets a word start with a dash. (`split` checks
    // after its direction.)
    if ["open", "edit", "tab", "notify", "checkpoint"].contains(command) {
      switch operands(rest, command: command, checkAll: command == "open" || command == "edit") {
      case .success(let words): rest = words
      case .failure(let error): return .failure(error)
      }
    }

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
      switch operands(words, command: command) {
      case .success(let operands): words = operands
      case .failure(let error): return .failure(error)
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

    case "tasks":
      var words = rest
      if words.first == "--json" {
        request.arguments["json"] = "1"
        words.removeFirst()
      }
      if words.first == "wait" {
        guard words.count == 2 else { return .failure(UsageError("usage: impulse tasks wait <task>")) }
        request.arguments["wait"] = words[1]
        request.wait = true
      } else if let word = words.first {
        return .failure(UsageError("impulse tasks: unknown argument '\(word)'\n\n" + usage))
      }

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
        // Tool events: which tool, on what (a shell command, a file).
        if let tool = object["tool_name"] as? String { request.arguments["tool"] = tool }
        if let input = object["tool_input"] as? [String: Any] {
          if let command = input["command"] as? String { request.arguments["toolCommand"] = command }
          if let file = input["file_path"] as? String ?? input["notebook_path"] as? String {
            request.arguments["toolFile"] = file
          }
        }
        if let hookCwd = object["cwd"] as? String { request.arguments["hookCwd"] = hookCwd }
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
