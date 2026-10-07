// End-to-end tests driving `ServerProcess` + `LSPRegistry` against a mock LSP
// server (a /bin/sh script speaking real Content-Length framing), asserting
// the poll-event JSON envelopes match the FFI shapes byte-for-byte.
#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseLSP

  /// Shell prelude implementing framed reads (headers + dd for the body) and
  /// framed writes.
  private let mockScriptPrelude = #"""
    #!/bin/sh
    read_msg() {
      len=0
      while IFS= read -r line; do
        line=$(printf '%s' "$line" | tr -d '\r')
        case "$line" in
          "Content-Length: "*) len=${line#Content-Length: } ;;
          "") break ;;
        esac
      done
      if [ "${len:-0}" -gt 0 ]; then
        dd bs=1 count="$len" >/dev/null 2>&1
      fi
    }
    send() {
      printf 'Content-Length: %s\r\n\r\n%s' "${#1}" "$1"
    }

    """#

  private struct MockWorkspace {
    let root: String
    let rootUri: String
    let fileUri: String
    let scriptPath: String

    func cleanup() {
      try? FileManager.default.removeItem(atPath: root)
    }
  }

  private func makeWorkspace(scriptBody: String) throws -> MockWorkspace {
    let root = NSTemporaryDirectory() + "impulse-lsp-mock-" + UUID().uuidString
    try FileManager.default.createDirectory(
      atPath: root + "/src", withIntermediateDirectories: true)
    try "{}".write(toFile: root + "/package.json", atomically: true, encoding: .utf8)
    let file = root + "/src/main.mock"
    try "hello\n".write(toFile: file, atomically: true, encoding: .utf8)

    let scriptPath = root + "/mock-lsp.sh"
    try (mockScriptPrelude + scriptBody).write(
      toFile: scriptPath, atomically: true, encoding: .utf8)
    try FileManager.default.setAttributes(
      [.posixPermissions: 0o755], ofItemAtPath: scriptPath)

    return MockWorkspace(
      root: root,
      rootUri: FileURI.fromPath(root)!,
      fileUri: FileURI.fromPath(file)!,
      scriptPath: scriptPath)
  }

  private func makeRegistry(_ workspace: MockWorkspace) -> LSPRegistry {
    var config = LSPConfig.defaultConfig()
    config.servers["mock"] = LSPServerConfig(command: workspace.scriptPath)
    config.languageServers["mocklang"] = ["mock"]
    return LSPRegistry(rootUri: workspace.rootUri, config: config)
  }

  /// Drains registry events until one matches, keeping non-matching events
  /// unconsumed-order irrelevant for the assertions below.
  private final class EventCollector {
    private let registry: LSPRegistry
    private var collected: [(raw: String, object: [String: Any])] = []

    init(_ registry: LSPRegistry) {
      self.registry = registry
    }

    func waitFor(type: String, timeout: TimeInterval = 15) -> (raw: String, object: [String: Any])? {
      let deadline = Date().addingTimeInterval(timeout)
      while Date() < deadline {
        if let match = collected.first(where: { $0.object["type"] as? String == type }) {
          collected.removeAll { $0.object["type"] as? String == type }
          return match
        }
        while let raw = registry.pollEvent() {
          if let object = JSONUtil.parse(raw) as? [String: Any] {
            collected.append((raw, object))
          }
        }
        if let match = collected.first(where: { $0.object["type"] as? String == type }) {
          collected.removeAll { $0.object["type"] as? String == type }
          return match
        }
        Thread.sleep(forTimeInterval: 0.01)
      }
      return nil
    }
  }

  struct MockServerTests {
    @Test func handshakeRequestDiagnosticsAndExit() throws {
      // Message order seen by the server: initialize request, initialized
      // notification, workspace/didChangeConfiguration notification, then
      // our test request (id 2).
      let body = #"""
        read_msg
        send '{"jsonrpc":"2.0","id":1,"result":{"capabilities":{"textDocumentSync":2}}}'
        read_msg
        read_msg
        read_msg
        send '{"jsonrpc":"2.0","id":2,"result":{"ok":true}}'
        send '{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":"file:///tmp/x.mock","version":3,"diagnostics":[{"range":{"start":{"line":1,"character":2},"end":{"line":1,"character":5}},"message":"boom","severity":2,"source":"mock"}]}}'
        exit 0
        """#
      let workspace = try makeWorkspace(scriptBody: body)
      defer { workspace.cleanup() }
      let registry = makeRegistry(workspace)
      let events = EventCollector(registry)

      let count = registry.ensureServers(languageId: "mocklang", fileUri: workspace.fileUri)
      #expect(count == 1)

      // Initialized envelope, byte-compatible with the FFI's serde output
      // (alphabetically ordered keys).
      let initialized = try #require(events.waitFor(type: "initialized"))
      let expectedClientKey = "mock@\(workspace.rootUri)"
      #expect(
        initialized.raw
          == "{\"clientKey\":\"\(expectedClientKey)\",\"serverId\":\"mock\",\"type\":\"initialized\"}"
      )

      // Blocking request/response correlation.
      let response = registry.request(
        languageId: "mocklang", fileUri: workspace.fileUri,
        method: "test/echo", paramsJSON: "{\"a\":1}")
      #expect(response == "{\"ok\":true}")

      // Diagnostics envelope: exact FFI shape.
      let diagnostics = try #require(events.waitFor(type: "diagnostics"))
      #expect(
        diagnostics.raw
          == "{\"diagnostics\":[{\"endColumn\":5,\"endLine\":1,\"message\":\"boom\",\"severity\":2,\"source\":\"mock\",\"startColumn\":2,\"startLine\":1}],\"type\":\"diagnostics\",\"uri\":\"file:///tmp/x.mock\",\"version\":3}"
      )

      // The script exits after sending diagnostics -> serverExited.
      let exited = try #require(events.waitFor(type: "serverExited"))
      #expect(
        exited.raw
          == "{\"clientKey\":\"\(expectedClientKey)\",\"serverId\":\"mock\",\"type\":\"serverExited\"}"
      )

      registry.shutdownAll()
    }

    @Test func serverCrashDrainsPendingRequests() throws {
      // Responds to initialize, then reads one more request and exits
      // without answering it.
      let body = #"""
        read_msg
        send '{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}'
        read_msg
        read_msg
        read_msg
        exit 0
        """#
      let workspace = try makeWorkspace(scriptBody: body)
      defer { workspace.cleanup() }
      let registry = makeRegistry(workspace)
      let events = EventCollector(registry)

      #expect(registry.ensureServers(languageId: "mocklang", fileUri: workspace.fileUri) == 1)
      #expect(events.waitFor(type: "initialized") != nil)

      let response = registry.request(
        languageId: "mocklang", fileUri: workspace.fileUri,
        method: "test/never-answered", paramsJSON: nil)
      #expect(response == "{\"error\":\"LSP server exited unexpectedly\"}")

      #expect(events.waitFor(type: "serverExited") != nil)
    }

    @Test func missingCommandEmitsServerErrorAndCooldown() throws {
      let workspace = try makeWorkspace(scriptBody: "exit 0\n")
      defer { workspace.cleanup() }
      var config = LSPConfig.defaultConfig()
      config.servers["missing"] = LSPServerConfig(command: "definitely-not-a-real-command-xyz")
      config.languageServers["mocklang"] = ["missing"]
      let registry = LSPRegistry(rootUri: workspace.rootUri, config: config)
      let events = EventCollector(registry)

      #expect(registry.ensureServers(languageId: "mocklang", fileUri: workspace.fileUri) == 0)

      let error = try #require(events.waitFor(type: "serverError", timeout: 5))
      let expectedClientKey = "missing@\(workspace.rootUri)"
      let expectedMessage =
        "LSP server 'missing' requires 'definitely-not-a-real-command-xyz' but it is not in PATH. Install it or override `servers.missing` in lsp.json."
      #expect(error.object["clientKey"] as? String == expectedClientKey)
      #expect(error.object["serverId"] as? String == "missing")
      #expect(error.object["message"] as? String == expectedMessage)

      // Within the 15s cooldown no new start is attempted, so no second
      // serverError event appears.
      #expect(registry.ensureServers(languageId: "mocklang", fileUri: workspace.fileUri) == 0)
      #expect(events.waitFor(type: "serverError", timeout: 1) == nil)
    }

    @Test func serverMessagesProgressAndApplyEdit() throws {
      let body = #"""
        read_msg
        send '{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}'
        read_msg
        read_msg
        send '{"jsonrpc":"2.0","method":"window/showMessage","params":{"type":1,"message":"index failed"}}'
        send '{"jsonrpc":"2.0","method":"$/progress","params":{"token":7,"value":{"kind":"begin","title":"Indexing","percentage":40}}}'
        send '{"jsonrpc":"2.0","id":"edit-1","method":"workspace/applyEdit","params":{"label":"Fix","edit":{"changes":{}}}}'
        len=0
        while IFS= read -r line; do
          line=$(printf '%s' "$line" | tr -d '\r')
          case "$line" in
            "Content-Length: "*) len=${line#Content-Length: } ;;
            "") break ;;
          esac
        done
        reply=$(dd bs=1 count="$len" 2>/dev/null)
        case "$reply" in
          *'"id":"edit-1"'*'"applied":true'*) send '{"jsonrpc":"2.0","method":"window/showMessage","params":{"type":3,"message":"answered"}}' ;;
          *) send '{"jsonrpc":"2.0","method":"window/showMessage","params":{"type":3,"message":"bad answer"}}' ;;
        esac
        sleep 1
        exit 0
        """#
      let workspace = try makeWorkspace(scriptBody: body)
      defer { workspace.cleanup() }
      let registry = makeRegistry(workspace)
      let events = EventCollector(registry)
      #expect(registry.ensureServers(languageId: "mocklang", fileUri: workspace.fileUri) == 1)
      let clientKey = "mock@\(workspace.rootUri)"

      let message = try #require(events.waitFor(type: "showMessage"))
      #expect(message.object["messageType"] as? Int == 1)
      #expect(message.object["message"] as? String == "index failed")

      let progress = try #require(events.waitFor(type: "progress"))
      #expect(progress.object["token"] as? String == "7")
      #expect(progress.object["kind"] as? String == "begin")
      #expect(progress.object["title"] as? String == "Indexing")
      #expect(progress.object["percentage"] as? Int == 40)

      let apply = try #require(events.waitFor(type: "applyEdit"))
      #expect(apply.object["id"] as? String == "edit-1")
      #expect(apply.object["label"] as? String == "Fix")
      #expect(apply.object["clientKey"] as? String == clientKey)
      registry.respond(clientKey: clientKey, id: apply.object["id"] ?? NSNull(), resultJSON: "{\"applied\":true}")

      let answered = try #require(events.waitFor(type: "showMessage"))
      #expect(answered.object["message"] as? String == "answered")
    }

    /// Reads one message body into $body (after the prelude's read_msg).
    private static let readBody = #"""
      read_body() {
        len=0
        while IFS= read -r line; do
          line=$(printf '%s' "$line" | tr -d '\r')
          case "$line" in
            "Content-Length: "*) len=${line#Content-Length: } ;;
            "") break ;;
          esac
        done
        body=$(dd bs=1 count="$len" 2>/dev/null)
      }

      """#

    private func didOpen(_ workspace: MockWorkspace) -> String {
      "{\"textDocument\":{\"uri\":\"\(workspace.fileUri)\",\"languageId\":\"mocklang\",\"version\":1,\"text\":\"hello\"}}"
    }

    @Test func aServerThatStartsLateGetsTheOpenDocuments() throws {
      let body = Self.readBody + #"""
        read_msg
        send '{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}'
        read_msg
        read_msg
        read_body
        case "$body" in
          *textDocument/didOpen*main.mock*|*main.mock*textDocument/didOpen*) send '{"jsonrpc":"2.0","method":"window/showMessage","params":{"type":3,"message":"opened"}}' ;;
          *) send '{"jsonrpc":"2.0","method":"window/showMessage","params":{"type":3,"message":"missed"}}' ;;
        esac
        sleep 2
        """#
      let workspace = try makeWorkspace(scriptBody: body)
      defer { workspace.cleanup() }
      let registry = makeRegistry(workspace)
      let events = EventCollector(registry)

      // Nothing is running yet: the notification doesn't wait for a start.
      let started = Date()
      #expect(
        registry.notify(
          languageId: "mocklang", fileUri: workspace.fileUri, method: "textDocument/didOpen",
          paramsJSON: didOpen(workspace)) == false)
      #expect(Date().timeIntervalSince(started) < 1)

      let message = try #require(events.waitFor(type: "showMessage"))
      #expect(message.object["message"] as? String == "opened")
      registry.shutdownAll()
    }

    @Test func aCrashedServerComesBackWithItsDocuments() throws {
      let body = Self.readBody + #"""
        marker="$(dirname "$0")/crashed-once"
        read_msg
        send '{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}'
        read_msg
        read_msg
        read_body
        if [ -f "$marker" ]; then
          case "$body" in
            *textDocument/didOpen*) send '{"jsonrpc":"2.0","method":"window/showMessage","params":{"type":3,"message":"back"}}' ;;
          esac
          sleep 2
          exit 0
        fi
        touch "$marker"
        exit 1
        """#
      let workspace = try makeWorkspace(scriptBody: body)
      defer { workspace.cleanup() }
      let registry = makeRegistry(workspace)
      let events = EventCollector(registry)

      #expect(registry.ensureServers(languageId: "mocklang", fileUri: workspace.fileUri) == 1)
      registry.notify(
        languageId: "mocklang", fileUri: workspace.fileUri, method: "textDocument/didOpen",
        paramsJSON: didOpen(workspace))

      let error = try #require(events.waitFor(type: "serverError"))
      #expect((error.object["message"] as? String)?.contains("stopped unexpectedly") == true)
      let back = try #require(events.waitFor(type: "showMessage", timeout: 20))
      #expect(back.object["message"] as? String == "back")
      registry.shutdownAll()
    }

    @Test func aFailedHandshakeLeavesNoProcessBehind() throws {
      let body = #"""
        echo $$ > "$(dirname "$0")/pid"
        read_msg
        send '{"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"cannot start"}}'
        exec sleep 30
        """#
      let workspace = try makeWorkspace(scriptBody: body)
      defer { workspace.cleanup() }
      let registry = makeRegistry(workspace)

      #expect(registry.ensureServers(languageId: "mocklang", fileUri: workspace.fileUri) == 0)
      let pidText = try String(contentsOfFile: workspace.root + "/pid", encoding: .utf8)
      let pid = try #require(pid_t(pidText.trimmingCharacters(in: .whitespacesAndNewlines)))
      let deadline = Date().addingTimeInterval(5)
      while Date() < deadline, kill(pid, 0) == 0 { Thread.sleep(forTimeInterval: 0.05) }
      #expect(kill(pid, 0) == -1, "the server process is gone")
    }

    /// A server that logs what it's told to `<name>.log`, answers shutdown,
    /// and runs `onOpen` / `onClose` shell code for didOpen / didClose.
    private static func loggingServer(_ name: String, onOpen: String = "", onClose: String = "") -> String {
      mockScriptPrelude + readBody + #"""
        log="$(dirname "$0")/\#(name).log"
        read_body
        send '{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}'
        while true; do
          read_body
          [ -n "$body" ] || exit 0
          case "$body" in
            *'"method":"textDocument/didOpen"'*) echo didOpen >> "$log"; \#(onOpen) ;;
            *'"method":"textDocument/didClose"'*) echo didClose >> "$log"; \#(onClose) ;;
            *'"method":"shutdown"'*)
              id=$(printf '%s' "$body" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
              send "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":null}" ;;
            *'"method":"exit"'*) exit 0 ;;
          esac
        done
        """#
    }

    private static func publish(_ uri: String, _ message: String) -> String {
      #"send '{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":"\#(uri)","diagnostics":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"message":"\#(message)"}]}}'"#
    }

    @Test func aRemappedLanguageMovesItsDocumentsToTheNewServer() throws {
      let workspace = try makeWorkspace(scriptBody: "exit 0\n")
      defer { workspace.cleanup() }
      let root = workspace.root
      let uri = workspace.fileUri
      // The old server publishes once more after closing the document: late,
      // and dropped.
      for (name, script) in [
        ("a", Self.loggingServer("a", onClose: Self.publish(uri, "stale"))),
        ("b", Self.loggingServer("b", onOpen: Self.publish(uri, "from b"))),
      ] {
        try script.write(toFile: "\(root)/\(name).sh", atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: "\(root)/\(name).sh")
      }
      try FileManager.default.createDirectory(atPath: root + "/.impulse", withIntermediateDirectories: true)
      try #"{"servers": {"a": {"command": "\#(root)/a.sh"}, "b": {"command": "\#(root)/b.sh"}}}"#
        .write(toFile: root + "/global.json", atomically: true, encoding: .utf8)
      try #"{"language_servers": {"mocklang": ["a"]}}"#
        .write(toFile: root + "/.impulse/lsp.json", atomically: true, encoding: .utf8)
      let registry = LSPRegistry(rootUri: workspace.rootUri, globalConfigPath: root + "/global.json")
      registry.configRecheckInterval = 0
      defer { registry.shutdownAll() }

      func log(_ name: String) -> [String] {
        ((try? String(contentsOfFile: "\(root)/\(name).log", encoding: .utf8)) ?? "")
          .split(separator: "\n").map(String.init)
      }
      func waitUntil(_ condition: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(15)
        while Date() < deadline {
          if condition() { return true }
          Thread.sleep(forTimeInterval: 0.02)
        }
        return false
      }

      registry.notify(
        languageId: "mocklang", fileUri: uri, method: "textDocument/didOpen", paramsJSON: didOpen(workspace))
      #expect(waitUntil { log("a") == ["didOpen"] })

      try #"{"language_servers": {"mocklang": ["b"]}}"#
        .write(toFile: root + "/.impulse/lsp.json", atomically: true, encoding: .utf8)
      registry.didChange(languageId: "mocklang", fileUri: uri, version: 2, fullText: "hello!", changesJSON: nil)

      // a closes it and its diagnostics are cleared; b gets it and publishes.
      var messages: [[String]] = []
      func drain() {
        while let raw = registry.pollEvent() {
          guard let event = JSONUtil.parse(raw) as? [String: Any], event["type"] as? String == "diagnostics" else {
            continue
          }
          messages.append((event["diagnostics"] as? [[String: Any]] ?? []).compactMap { $0["message"] as? String })
        }
      }
      #expect(
        waitUntil {
          drain()
          return messages.contains(["from b"]) && log("a").contains("didClose")
        })
      // Time for a's late publish to arrive (and be dropped).
      Thread.sleep(forTimeInterval: 0.5)
      drain()
      #expect(messages.first == [])
      #expect(!messages.contains(["stale"]))
      #expect(log("a") == ["didOpen", "didClose"])
      #expect(log("b") == ["didOpen"])
    }

    @Test func unknownLanguageHasNoClients() throws {
      let workspace = try makeWorkspace(scriptBody: "exit 0\n")
      defer { workspace.cleanup() }
      let registry = makeRegistry(workspace)

      #expect(registry.ensureServers(languageId: "nope", fileUri: workspace.fileUri) == 0)
      #expect(
        registry.request(
          languageId: "nope", fileUri: workspace.fileUri, method: "test/x", paramsJSON: nil)
          == "{\"error\":\"no LSP client available\"}")
      #expect(
        registry.notify(
          languageId: "nope", fileUri: workspace.fileUri, method: "test/x", paramsJSON: nil)
          == false)
      #expect(registry.pollEvent() == nil)
    }
  }
#endif
