#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseProtocol

  /// Answers every request with `answer`, recording what it was sent.
  private final class FakeChannel: ControlChannel, @unchecked Sendable {
    let answer: @Sendable (ControlRequest) -> Result<ControlResponse, ControlClient.Failure>
    let log: RequestLog
    let release = DispatchSemaphore(value: 0)
    let blocks: Bool

    init(
      log: RequestLog, blocks: Bool = false,
      answer: @escaping @Sendable (ControlRequest) -> Result<ControlResponse, ControlClient.Failure>
    ) {
      self.log = log
      self.blocks = blocks
      self.answer = answer
    }

    func send(_ request: ControlRequest) -> Result<ControlResponse, ControlClient.Failure> {
      log.append(request)
      if blocks, request.command != "mcp-tools" {
        release.wait()
        return .failure(.noAnswer)
      }
      return answer(request)
    }

    func cancel() { release.signal() }
  }

  private final class RequestLog: @unchecked Sendable {
    private let lock = NSLock()
    private var requests: [ControlRequest] = []
    func append(_ request: ControlRequest) {
      lock.lock()
      requests.append(request)
      lock.unlock()
    }
    var all: [ControlRequest] {
      lock.lock()
      defer { lock.unlock() }
      return requests
    }
  }

  /// The messages the server wrote, parsed; waits for `count` of them.
  private final class Output: @unchecked Sendable {
    private let lock = NSLock()
    private var lines: [JSONValue] = []
    private let arrived = DispatchSemaphore(value: 0)

    func write(_ data: Data) {
      lock.lock()
      lines.append(JSONValue.parse(data) ?? .null)
      lock.unlock()
      arrived.signal()
    }

    func wait(for count: Int) -> [JSONValue] {
      while true {
        lock.lock()
        let current = lines
        lock.unlock()
        if current.count >= count { return current }
        // Generous: the whole suite runs at once, and this only bounds a
        // failure.
        if arrived.wait(timeout: .now() + 20) == .timedOut { return current }
      }
    }
  }

  struct MCPServerTests {
    private let inside = [ControlProtocol.socketKey: "/tmp/impulse.sock", ControlProtocol.tokenKey: "PANE"]

    private func makeServer(
      environment: [String: String]? = nil, blocks: Bool = false,
      answer: @escaping @Sendable (ControlRequest) -> Result<ControlResponse, ControlClient.Failure> = { _ in
        .success(ControlResponse(ok: true, message: "fine"))
      }
    ) -> (MCPServer, Output, RequestLog, () -> FakeChannel?) {
      let output = Output()
      let log = RequestLog()
      let box = ChannelBox()
      let server = MCPServer(
        environment: environment ?? inside, cwd: "/repo/sub", version: "1.2.3",
        connect: {
          let channel = FakeChannel(log: log, blocks: blocks, answer: answer)
          box.set(channel)
          return channel
        },
        emit: { output.write($0) })
      return (server, output, log, { box.last })
    }

    private func send(_ server: MCPServer, _ json: String) {
      server.handle(line: Data(json.utf8))
    }

    @Test func initializeNegotiatesTheVersion() {
      let (server, output, _, _) = makeServer()
      send(server, #"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#)
      send(server, #"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#)
      let lines = output.wait(for: 2)
      #expect(lines[0]["id"] == 1)
      #expect(lines[0]["result"]?["protocolVersion"] == "2025-06-18")
      #expect(lines[0]["result"]?["serverInfo"]?["version"] == "1.2.3")
      #expect(lines[0]["result"]?["capabilities"]?["tools"] != nil)
      #expect(lines[0]["result"]?["resultType"] == nil)
      #expect(lines[1]["result"]?["protocolVersion"] == .string(MCPServer.protocolVersions[0]))
    }

    /// `_meta` naming the 2026-07-28 revision, as a stateless client sends.
    private let stateless =
      #""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}"#

    @Test func discoverDescribesTheStatelessRevision() {
      let (server, output, _, _) = makeServer()
      send(server, #"{"jsonrpc":"2.0","id":"d","method":"server/discover","params":{"# + stateless + "}}")
      let result = output.wait(for: 1)[0]["result"]
      #expect(result?["supportedVersions"] == ["2026-07-28"])
      #expect(result?["resultType"] == "complete")
      #expect(result?["capabilities"]?["tools"] != nil)
      #expect(result?["ttlMs"]?.int != nil)
      #expect(result?["cacheScope"] == "private")
      #expect(result?["_meta"]?["io.modelcontextprotocol/serverInfo"]?["name"] == "impulse")
      #expect(result?["instructions"]?.string != nil)
      // Not initialize's fields.
      #expect(result?["protocolVersion"] == nil)
      #expect(result?["serverInfo"] == nil)
    }

    @Test func statelessRequestsGetTheirRevisionsShape() {
      let (server, output, _, _) = makeServer()
      send(server, #"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"# + stateless + "}}")
      send(
        server,
        #"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_tasks","arguments":{},"# + stateless
          + "}}")
      send(
        server,
        #"{"jsonrpc":"2.0","id":3,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2099-01-01","io.modelcontextprotocol/clientCapabilities":{}}}}"#
      )
      send(
        server,
        #"{"jsonrpc":"2.0","id":4,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}"#
      )
      // A session's requests keep the older shape.
      send(server, #"{"jsonrpc":"2.0","id":5,"method":"tools/list"}"#)
      let lines = output.wait(for: 5)
      let byID = Dictionary(lines.map { ($0["id"]?.int ?? 0, $0) }, uniquingKeysWith: { a, _ in a })
      #expect(byID[1]?["result"]?["resultType"] == "complete")
      #expect(byID[1]?["result"]?["ttlMs"]?.int == MCPServer.toolsTTL)
      #expect(byID[1]?["result"]?["cacheScope"] == "private")
      #expect(byID[2]?["result"]?["resultType"] == "complete")
      #expect(byID[2]?["result"]?["isError"] == false)
      #expect(byID[2]?["result"]?["_meta"]?["io.modelcontextprotocol/serverInfo"]?["version"] == "1.2.3")
      #expect(byID[3]?["error"]?["code"] == -32022)
      #expect(byID[3]?["error"]?["data"]?["requested"] == "2099-01-01")
      #expect(byID[3]?["error"]?["data"]?["supported"] == ["2026-07-28"])
      #expect(byID[4]?["error"]?["code"] == -32602)
      #expect(byID[5]?["result"]?["tools"] != nil)
      #expect(byID[5]?["result"]?["resultType"] == nil)
      #expect(byID[5]?["result"]?["ttlMs"] == nil)
    }

    @Test func noOutputSchema() {
      // With one, clients reject a successful result without structured
      // content, and "Done." has none.
      for tool in MCPTools.all { #expect(tool.listing["outputSchema"] == nil, "\(tool.name)") }
      let result = MCPServer.toolResult(ControlResponse(ok: true))
      #expect(result["structuredContent"] == nil)
      #expect(result["isError"] == false)
    }

    @Test func notificationsGetNoAnswerAndUnknownMethodsAnError() {
      let (server, output, _, _) = makeServer()
      send(server, #"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
      send(server, #"{"jsonrpc":"2.0","id":7,"method":"resources/list"}"#)
      send(server, "not json")
      send(server, #"[{"jsonrpc":"2.0","id":8,"method":"ping"}]"#)
      send(server, #"{"jsonrpc":"2.0","id":9,"method":"ping"}"#)
      let lines = output.wait(for: 4)
      #expect(lines.count == 4)
      #expect(lines[0]["error"]?["code"] == -32601)
      #expect(lines[1]["error"]?["code"] == -32700)
      #expect(lines[2]["error"]?["code"] == -32600)
      #expect(lines[3]["result"] == [:])
    }

    @Test func toolsListHonorsTheAppsSettings() {
      let (server, output, _, _) = makeServer { request in
        guard request.command == "mcp-tools" else { return .success(ControlResponse(ok: true)) }
        return .success(ControlResponse(ok: true, data: Data(#"{"disabled":["create_task"]}"#.utf8)))
      }
      send(server, #"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
      guard case .array(let tools)? = output.wait(for: 1)[0]["result"]?["tools"] else {
        Issue.record("no tools")
        return
      }
      let names = tools.compactMap { $0["name"]?.string }
      #expect(names.contains("list_tasks"))
      #expect(names.contains("get_diagnostics"))
      #expect(!names.contains("create_task"))
      #expect(tools.first?["annotations"]?["readOnlyHint"] == true)
    }

    @Test func outsideImpulseThereAreNoTools() {
      let (server, output, log, _) = makeServer(environment: [:])
      send(server, #"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
      send(server, #"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_tasks"}}"#)
      send(server, #"{"jsonrpc":"2.0","id":3,"method":"initialize","params":{}}"#)
      let lines = output.wait(for: 3)
      let byID = Dictionary(lines.map { ($0["id"]?.int ?? 0, $0) }, uniquingKeysWith: { a, _ in a })
      #expect(byID[1]?["result"]?["tools"] == [])
      #expect(byID[2]?["result"]?["isError"] == true)
      #expect(byID[3]?["result"]?["instructions"]?.string?.contains("isn't running in an Impulse terminal") == true)
      #expect(log.all.isEmpty)
    }

    @Test func aCallBecomesAControlRequest() {
      let (server, output, log, _) = makeServer { _ in
        .success(ControlResponse(ok: true, message: "trailhead: 2 workspaces", data: Data(#"{"repository":"trailhead"}"#.utf8)))
      }
      send(server, #"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"get_task","arguments":{"task":"fix-elevation"}}}"#)
      let result = output.wait(for: 1)[0]["result"]
      #expect(result?["isError"] == false)
      #expect(result?["structuredContent"]?["repository"] == "trailhead")
      guard case .array(let content)? = result?["content"] else {
        Issue.record("no content")
        return
      }
      #expect(content.first?["text"] == "trailhead: 2 workspaces")
      let request = log.all.first
      #expect(request?.command == "tasks")
      #expect(request?.arguments["show"] == "fix-elevation")
      #expect(request?.arguments["via"] == "mcp")
      #expect(request?.token == "PANE")
      #expect(request?.cwd == "/repo/sub")
    }

    @Test func argumentsAreChecked() {
      let (server, output, log, _) = makeServer()
      send(server, #"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_task","arguments":{}}}"#)
      send(server, #"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"no_such_tool"}}"#)
      let brief = String(repeating: "x", count: ControlProtocol.briefLimit + 1)
      send(
        server,
        #"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"create_task","arguments":{"title":"t","brief":""#
          + brief + #""}}}"#)
      let lines = output.wait(for: 3)
      let byID = Dictionary(lines.map { ($0["id"]?.int ?? 0, $0) }, uniquingKeysWith: { a, _ in a })
      #expect(byID[1]?["result"]?["isError"] == true)
      #expect(byID[2]?["error"]?["code"] == -32602)
      #expect(byID[3]?["result"]?["isError"] == true)
      #expect(log.all.isEmpty)
    }

    @Test func toolArgumentsMapToTheCommandsArguments() {
      let context = MCPContext(token: "T", cwd: "/repo")
      func request(_ name: String, _ arguments: [String: JSONValue]) -> ControlRequest? {
        try? MCPTools.named(name)?.request(arguments, context: context).get()
      }
      let open = request("open_file", ["path": "src/a.ts", "line": 12])
      #expect(open?.command == "open")
      #expect(open?.arguments["path"] == "/repo/src/a.ts")
      #expect(open?.arguments["line"] == "12")
      #expect(open?.arguments["background"] == "1")
      let wait = request("wait_for_task", ["task": "x", "timeout_seconds": 100_000])
      #expect(wait?.wait == true)
      #expect(wait?.arguments["timeout"] == String(MCPTools.maxWait))
      let create = request("create_task", ["title": "Fix it", "agent": "claude", "brief": "Do it", "move_changes": true])
      #expect(create?.arguments["new"] == "Fix it")
      #expect(create?.arguments["agent"] == "claude")
      #expect(create?.arguments["brief"] == "Do it")
      #expect(create?.arguments["move"] == "1")
      #expect(create?.wait == true)
      #expect(request("set_status", ["state": "busy"]) == nil)
      #expect(request("get_block_output", ["block": "ab12:7", "lines": 5000])?.arguments["lines"] == "2000")
      #expect(request("list_review_comments", ["include_outdated": false])?.arguments["current"] == "1")
      #expect(request("resolve_review_comment", ["id": "c1"]) == nil)
      // Every tool has a schema that's an object.
      for tool in MCPTools.all {
        #expect(tool.inputSchema["type"] == "object", "\(tool.name)")
      }
      #expect(Set(MCPTools.all.map(\.name)).count == MCPTools.all.count)
    }

    @Test func failuresAreToolErrors() {
      let (server, output, _, _) = makeServer { _ in .failure(.unreachable) }
      send(server, #"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_tasks"}}"#)
      let result = output.wait(for: 1)[0]["result"]
      #expect(result?["isError"] == true)
      let (refusing, refusals, _, _) = makeServer { _ in .success(ControlResponse(ok: false, message: "Not in a git repository.")) }
      send(refusing, #"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_tasks"}}"#)
      #expect(refusals.wait(for: 1)[0]["result"]?["isError"] == true)
    }

    @Test func finishLetsAToolListGoOut() {
      let (server, output, _, _) = makeServer()
      send(server, #"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
      server.finish()
      #expect(output.wait(for: 1).first?["id"] == 1)
    }

    @Test func aCancelledCallHangsUpAndGetsNoAnswer() {
      let (server, output, log, last) = makeServer(blocks: true)
      send(server, #"{"jsonrpc":"2.0","id":"w","method":"tools/call","params":{"name":"wait_for_task","arguments":{"task":"x"}}}"#)
      // Wait for the call to reach the app.
      for _ in 0..<100 where log.all.isEmpty { usleep(10_000) }
      send(server, #"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"w"}}"#)
      send(server, #"{"jsonrpc":"2.0","id":"p","method":"ping"}"#)
      _ = output.wait(for: 1)
      usleep(100_000)
      let lines = output.wait(for: 1)
      #expect(lines.count == 1)
      #expect(lines[0]["id"] == "p")
      #expect(last() != nil)
    }
  }

  private final class ChannelBox: @unchecked Sendable {
    private let lock = NSLock()
    private var channel: FakeChannel?
    func set(_ channel: FakeChannel) {
      lock.lock()
      self.channel = channel
      lock.unlock()
    }
    var last: FakeChannel? {
      lock.lock()
      defer { lock.unlock() }
      return channel
    }
  }
#endif
