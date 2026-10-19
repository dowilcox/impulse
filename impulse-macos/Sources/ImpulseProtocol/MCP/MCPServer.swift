// `impulse mcp`: a Model Context Protocol server on standard input and
// output, one JSON-RPC message per line. Agents start it like any stdio MCP
// server; each tool call becomes one control request to the Impulse window
// the agent's terminal is in (see MCPTools), so the app decides and acts
// and this side keeps no state between calls.
//
// It answers both protocol eras. `initialize` starts a session in one of
// the revisions up to 2025-11-25. In the stateless 2026-07-28 revision,
// `server/discover` says what the server offers, and every request names
// its revision in `_meta`: those get its result shape (`resultType`, cache
// hints, the server's name in `_meta`). It offers tools only: no sampling
// (Impulse never calls a model), resources, prompts or logging.

import Foundation

/// One connection to the app, for one request: `ControlClient`, or a stand-in
/// in tests.
public protocol ControlChannel: Sendable {
  func send(_ request: ControlRequest) -> Result<ControlResponse, ControlClient.Failure>
  func cancel()
}

extension ControlClient: ControlChannel {}

public final class MCPServer: @unchecked Sendable {
  /// The revisions answered with `initialize`, newest first.
  public static let protocolVersions = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"]
  /// The stateless revisions, named in each request's `_meta`.
  public static let statelessVersions = ["2026-07-28"]

  static let versionKey = "io.modelcontextprotocol/protocolVersion"
  static let capabilitiesKey = "io.modelcontextprotocol/clientCapabilities"
  static let serverInfoKey = "io.modelcontextprotocol/serverInfo"
  /// How long a stateless client may keep the tool list (the settings can
  /// turn tools off) and what `server/discover` says (fixed for the
  /// process).
  static let toolsTTL = 60_000
  static let discoverTTL = 3_600_000

  private let context: MCPContext
  /// Whether the agent runs in an Impulse terminal (IMPULSE_SOCKET set).
  private let insideImpulse: Bool
  private let version: String
  private let connect: @Sendable () -> ControlChannel
  private let emit: @Sendable (Data) -> Void
  private let queue = DispatchQueue(label: "impulse.mcp.calls", attributes: .concurrent)
  /// Work under way on `queue`, for `finish`.
  private let pending = DispatchGroup()
  private let lock = NSLock()
  /// Tool calls under way, by request id (as JSON), for cancellation.
  private var inFlight: [String: ControlChannel] = [:]
  private var cancelled: Set<String> = []
  /// Seconds between progress notifications while a tool waits.
  public var progressInterval: TimeInterval = 15

  /// - Parameters:
  ///   - environment: the server's environment (IMPULSE_SOCKET,
  ///     IMPULSE_PANE_TOKEN).
  ///   - connect: a new connection to the app, for each request.
  ///   - emit: writes one message (a line, newline included) to the agent.
  ///     Called from several threads; it must write each line whole.
  public init(
    environment: [String: String], cwd: String, version: String,
    connect: @escaping @Sendable () -> ControlChannel, emit: @escaping @Sendable (Data) -> Void
  ) {
    let socket = environment[ControlProtocol.socketKey] ?? ""
    insideImpulse = !socket.isEmpty
    context = MCPContext(token: environment[ControlProtocol.tokenKey].flatMap { $0.isEmpty ? nil : $0 }, cwd: cwd)
    self.version = version
    self.connect = connect
    self.emit = emit
  }

  /// The server for this process: the app's socket from the environment.
  public static func standard(
    environment: [String: String], cwd: String, version: String, emit: @escaping @Sendable (Data) -> Void
  ) -> MCPServer {
    let socket = environment[ControlProtocol.socketKey] ?? ""
    return MCPServer(
      environment: environment, cwd: cwd, version: version, connect: { ControlClient(socketPath: socket) },
      emit: emit)
  }

  // MARK: Messages

  /// Handle one line from the agent. Answers go out through `emit`, tool
  /// calls once they finish (on another thread).
  public func handle(line: Data) {
    let trimmed = line.filter { $0 != 0x0A && $0 != 0x0D }
    guard !trimmed.isEmpty else { return }
    guard let message = JSONValue.parse(trimmed) else {
      return send(error: -32700, "Parse error", id: .null)
    }
    guard case .object(let object) = message else {
      // Batches were dropped from the protocol in 2025-06-18.
      return send(error: -32600, "Invalid request: send one message per line, not a batch.", id: .null)
    }
    let id = object["id"]
    guard let method = object["method"]?.string else {
      // A response to a request of ours (there are none), or nonsense.
      if id == nil || object["result"] != nil || object["error"] != nil { return }
      return send(error: -32600, "Invalid request", id: id ?? .null)
    }
    let params = object["params"]?.object ?? [:]
    // A notification: nothing to answer.
    guard let id, id != .null else {
      if method == "notifications/cancelled", let target = params["requestId"] {
        cancel(requestID: target)
      }
      return
    }
    // A stateless request names its revision; one that doesn't is in the
    // session `initialize` started.
    let meta = params["_meta"]?.object
    let era: Era
    if let requested = meta?[Self.versionKey]?.string {
      guard Self.statelessVersions.contains(requested) else {
        return send(
          error: -32022, "Unsupported protocol version: \(requested)", id: id,
          data: [
            "supported": .array(Self.statelessVersions.map { .string($0) }), "requested": .string(requested),
          ])
      }
      guard meta?[Self.capabilitiesKey]?.object != nil else {
        return send(error: -32602, "Invalid params: _meta needs \(Self.capabilitiesKey).", id: id)
      }
      era = .stateless
    } else {
      era = .session
    }
    switch method {
    case "initialize":
      let requested = params["protocolVersion"]?.string
      let version =
        requested.flatMap { Self.protocolVersions.contains($0) ? $0 : nil } ?? Self.protocolVersions[0]
      var result = description
      result["serverInfo"] = serverInfo
      result["protocolVersion"] = .string(version)
      send(result: .object(result), id: id)
    case "server/discover":
      var result = description
      result["supportedVersions"] = .array(Self.statelessVersions.map { .string($0) })
      result["ttlMs"] = .int(Self.discoverTTL)
      result["cacheScope"] = "private"
      send(result: .object(result), id: id, era: .stateless)
    case "ping":
      send(result: [:], id: id, era: era)
    case "tools/list":
      pending.enter()
      queue.async { [self] in
        defer { pending.leave() }
        var result: [String: JSONValue] = ["tools": .array(enabledTools().map(\.listing))]
        if era == .stateless {
          result["ttlMs"] = .int(Self.toolsTTL)
          result["cacheScope"] = "private"
        }
        send(result: .object(result), id: id, era: era)
      }
    case "tools/call":
      call(params, id: id, era: era)
    default:
      send(error: -32601, "Method not found: \(method)", id: id)
    }
  }

  /// Hang up every call under way (the agent closed standard input).
  public func cancelAll() {
    lock.lock()
    let channels = Array(inFlight.values)
    cancelled.formUnion(inFlight.keys)
    lock.unlock()
    channels.forEach { $0.cancel() }
  }

  /// The agent closed standard input: hang up the calls under way (they get
  /// no answer), and give what's left (a tool list being made) up to
  /// `timeout` to go out.
  public func finish(timeout: TimeInterval = 2) {
    cancelAll()
    _ = pending.wait(timeout: .now() + timeout)
  }

  /// Which revision a request is answered in.
  private enum Era {
    /// `initialize`'s session (2025-11-25 and before).
    case session
    /// 2026-07-28: the request named it in `_meta`.
    case stateless
  }

  private var serverInfo: JSONValue {
    ["name": "impulse", "title": "Impulse", "version": .string(version)]
  }

  /// What `initialize` and `server/discover` share.
  private var description: [String: JSONValue] {
    [
      "capabilities": ["tools": ["listChanged": false]],
      "instructions": .string(instructions),
    ]
  }

  private var instructions: String {
    guard insideImpulse else {
      return "This agent isn't running in an Impulse terminal, so Impulse has no tools for it. Start the agent in an Impulse terminal to use them."
    }
    return """
      Impulse is the terminal IDE you're running in. Its tools describe the repository you're in: its tasks \
      (branches in their own folders, often with another agent working in them), the user's review comments, \
      the language servers' problems, the commands and output in the user's other terminals, files with \
      unsaved edits, and the project's scripts. Check list_tasks before merging a branch or making sweeping \
      edits. create_task, finish_task and archive_task ask the user in Impulse before anything happens.
      """
  }

  /// The tools to offer: none outside Impulse; otherwise all but the ones
  /// the user's settings turn off (an older app doesn't say: all).
  private func enabledTools() -> [MCPTool] {
    guard insideImpulse else { return [] }
    var request = ControlRequest(command: "mcp-tools", token: context.token, cwd: context.cwd)
    request.arguments["via"] = "mcp"
    guard case .success(let response) = connect().send(request), response.ok,
      let data = response.data, let disabled = JSONValue.parse(data)?["disabled"], case .array(let names) = disabled
    else { return MCPTools.all }
    let off = Set(names.compactMap(\.string))
    return MCPTools.all.filter { !off.contains($0.name) }
  }

  // MARK: Tool calls

  private func call(_ params: [String: JSONValue], id: JSONValue, era: Era) {
    guard let name = params["name"]?.string, let tool = MCPTools.named(name) else {
      return send(error: -32602, "Unknown tool: \(params["name"]?.string ?? "(none)")", id: id)
    }
    guard insideImpulse else {
      return send(
        result: Self.toolResult(text: "Not inside an Impulse terminal: Impulse's tools only work there.", isError: true),
        id: id, era: era)
    }
    let arguments = params["arguments"]?.object ?? [:]
    let request: ControlRequest
    switch tool.request(arguments, context: context) {
    case .success(let made): request = made
    case .failure(let error):
      return send(result: Self.toolResult(text: error.message, isError: true), id: id, era: era)
    }
    let key = Self.key(id)
    let channel = connect()
    lock.lock()
    inFlight[key] = channel
    lock.unlock()
    let progress = tool.waits ? params["_meta"]?["progressToken"] : nil
    pending.enter()
    queue.async { [self] in
      defer { pending.leave() }
      let ticker = progress.map { startProgress(token: $0, key: key) }
      let outcome = channel.send(request)
      // No progress after the answer (or once cancelled).
      ticker?.stop()
      lock.lock()
      inFlight[key] = nil
      let wasCancelled = cancelled.remove(key) != nil
      lock.unlock()
      // A cancelled request gets no answer.
      guard !wasCancelled else { return }
      switch outcome {
      case .success(let response):
        send(result: Self.toolResult(response), id: id, era: era)
      case .failure(.unreachable):
        send(
          result: Self.toolResult(text: "Couldn't reach Impulse (is it still running?).", isError: true), id: id,
          era: era)
      case .failure(.noAnswer):
        send(result: Self.toolResult(text: "Impulse didn't answer.", isError: true), id: id, era: era)
      }
    }
  }

  private func cancel(requestID: JSONValue) {
    let key = Self.key(requestID)
    lock.lock()
    let channel = inFlight[key]
    if channel != nil { cancelled.insert(key) }
    lock.unlock()
    channel?.cancel()
  }

  private func isCancelled(_ key: String) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    return cancelled.contains(key)
  }

  /// Progress notifications while a tool waits, so the agent (and its
  /// timeout) can tell it's alive.
  private final class Ticker: @unchecked Sendable {
    let queue = DispatchQueue(label: "impulse.mcp.progress")
    let timer: DispatchSourceTimer
    private var stopped = false

    init(interval: TimeInterval, tick: @escaping @Sendable () -> Void) {
      timer = DispatchSource.makeTimerSource(queue: queue)
      timer.schedule(deadline: .now() + interval, repeating: interval)
      timer.setEventHandler { [unowned self] in
        if !stopped { tick() }
      }
      timer.resume()
    }

    /// Once this returns, no more ticks go out.
    func stop() {
      queue.sync {
        stopped = true
        timer.cancel()
      }
    }
  }

  private func startProgress(token: JSONValue, key: String) -> Ticker {
    let started = Date()
    return Ticker(interval: progressInterval) { [weak self] in
      guard let self, !self.isCancelled(key) else { return }
      let elapsed = Int(Date().timeIntervalSince(started))
      let message: JSONValue = [
        "jsonrpc": "2.0", "method": "notifications/progress",
        "params": ["progressToken": token, "progress": .int(elapsed), "message": .string("Waiting (\(elapsed) s)")],
      ]
      self.emit(message.line())
    }
  }

  /// A control response as a tool result: its text, and its JSON as
  /// structured content.
  static func toolResult(_ response: ControlResponse) -> JSONValue {
    var text = response.message ?? ""
    let structured = response.data.flatMap(JSONValue.parse)
    if text.isEmpty { text = response.ok ? "Done." : "Impulse couldn't do that." }
    var result = toolResult(text: text, isError: !response.ok)
    if case .object(let object)? = structured, case .object(var fields) = result {
      fields["structuredContent"] = .object(object)
      result = .object(fields)
    }
    return result
  }

  static func toolResult(text: String, isError: Bool) -> JSONValue {
    ["content": [["type": "text", "text": .string(text)]], "isError": .bool(isError)]
  }

  // MARK: Output

  /// A result, in the shape of the request's revision: a stateless one
  /// says it's complete, and who answered.
  private func send(result: JSONValue, id: JSONValue, era: Era = .session) {
    var result = result
    if era == .stateless, case .object(var fields) = result {
      fields["resultType"] = "complete"
      var meta = fields["_meta"]?.object ?? [:]
      meta[Self.serverInfoKey] = serverInfo
      fields["_meta"] = .object(meta)
      result = .object(fields)
    }
    emit(JSONValue.object(["jsonrpc": "2.0", "id": id, "result": result]).line())
  }

  private func send(error code: Int, _ message: String, id: JSONValue, data: JSONValue? = nil) {
    var error: [String: JSONValue] = ["code": .int(code), "message": .string(message)]
    if let data { error["data"] = data }
    emit(JSONValue.object(["jsonrpc": "2.0", "id": id, "error": .object(error)]).line())
  }

  private static func key(_ id: JSONValue) -> String {
    String(decoding: id.line(), as: UTF8.self)
  }
}
