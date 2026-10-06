// Port of `LspClient` from impulse-core/src/lsp.rs: child process with piped
// stdio, LSP base-protocol framing (`Content-Length` headers), request/id
// correlation with per-method timeouts, the initialize handshake, server
// request/notification handling, stderr logging, and exit detection.

import Foundation

// MARK: - Frame parser

/// Streaming parser for the LSP base protocol, ported from `reader_task`.
/// Feed arbitrary chunks; completed message bodies come back in order. A
/// protocol violation that Rust answered by dropping the connection sets
/// `failed` (the caller must stop reading).
final class FrameParser {
  static let maxHeaderLine = 8192
  static let maxHeaderCount = 32
  static let maxMessageSize = 32 * 1024 * 1024

  private enum Mode {
    case headers
    case body
  }

  private var mode: Mode = .headers
  private var buffer: [UInt8] = []
  private var pos = 0
  private var contentLength = 0
  private var headerCount = 0
  private var discardRemaining = 0
  private(set) var failed = false

  func feed<S: Sequence>(_ data: S) -> [[UInt8]] where S.Element == UInt8 {
    guard !failed else { return [] }
    buffer.append(contentsOf: data)
    var messages: [[UInt8]] = []

    loop: while true {
      if discardRemaining > 0 {
        // Draining an oversized body to keep the stream in sync.
        let take = min(discardRemaining, buffer.count - pos)
        pos += take
        discardRemaining -= take
        if discardRemaining > 0 { break loop }
        continue
      }

      switch mode {
      case .body:
        if buffer.count - pos < contentLength { break loop }
        messages.append(Array(buffer[pos..<(pos + contentLength)]))
        pos += contentLength
        contentLength = 0
        headerCount = 0
        mode = .headers

      case .headers:
        guard let newlineIndex = buffer[pos...].firstIndex(of: 0x0A) else {
          // Bounded header line to prevent unbounded memory allocation from
          // a malicious server sending no newline.
          if buffer.count - pos > Self.maxHeaderLine {
            lspLog(
              "LSP header line too long (\(buffer.count - pos) bytes), dropping connection")
            failed = true
          }
          break loop
        }
        let lineBytes = buffer[pos...newlineIndex]
        pos = newlineIndex + 1
        let line = String(decoding: lineBytes, as: UTF8.self)
        let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)

        if trimmed.isEmpty {
          // End of headers for this message.
          if contentLength == 0 {
            headerCount = 0
            continue
          }
          if contentLength > Self.maxMessageSize {
            // Reject absurdly large messages to prevent memory exhaustion.
            lspLog("LSP message too large (\(contentLength) bytes), skipping")
            discardRemaining = contentLength
            contentLength = 0
            headerCount = 0
            continue
          }
          mode = .body
          continue
        }

        headerCount += 1
        if headerCount > Self.maxHeaderCount {
          lspLog("Too many LSP headers, dropping connection")
          failed = true
          break loop
        }
        // Exact, case-sensitive prefix match like lsp.rs.
        if trimmed.hasPrefix("Content-Length: ") {
          let rest = String(trimmed.dropFirst("Content-Length: ".count))
          if contentLength > 0 {
            lspLog(
              "Duplicate Content-Length header in LSP message, keeping first value (\(contentLength))"
            )
          } else if !rest.isEmpty,
            rest.utf8.allSatisfy({ $0 >= 0x30 && $0 <= 0x39 }),
            let value = Int(rest)
          {
            contentLength = value
          }
        }
      }
    }

    // Compact the consumed prefix.
    if pos > 0 {
      buffer.removeFirst(pos)
      pos = 0
    }
    return messages
  }
}

// MARK: - Pending request

private final class PendingRequest {
  private let semaphore = DispatchSemaphore(value: 0)
  private let lock = NSLock()
  private var result: Result<Any, String>?

  func complete(_ value: Result<Any, String>) {
    lock.lock()
    let alreadyCompleted = result != nil
    if !alreadyCompleted { result = value }
    lock.unlock()
    if !alreadyCompleted { semaphore.signal() }
  }

  func wait(timeout: TimeInterval) -> Result<Any, String>? {
    guard semaphore.wait(timeout: .now() + timeout) == .success else { return nil }
    lock.lock()
    defer { lock.unlock() }
    return result
  }
}

// MARK: - Server process

/// One running LSP server (the Rust `LspClient`). Synchronous: `request`
/// blocks the calling thread with the same per-method timeouts as lsp.rs.
final class ServerProcess {
  let serverId: String
  let clientKey: String
  let rootUri: String

  private let commandName: String
  private let process: Process
  private let stdinHandle: FileHandle
  private let stdoutHandle: FileHandle
  private let stderrHandle: FileHandle
  private let onEvent: (LSPEvent) -> Void

  private let writeLock = NSLock()
  private let stateLock = NSLock()
  private var pending: [Int64: PendingRequest] = [:]
  private var nextId: Int64 = 1
  private var exited = false
  private var serverCapabilitiesStorage: [String: Any]?
  private var changeSyncKindStorage: Int?

  /// Server capabilities from the initialize response (JSON object), if the
  /// handshake succeeded.
  var serverCapabilities: [String: Any]? {
    stateLock.lock()
    defer { stateLock.unlock() }
    return serverCapabilitiesStorage
  }

  /// TextDocumentSyncKind advertised by the server (1 = full,
  /// 2 = incremental), if any.
  var changeSyncKind: Int? {
    stateLock.lock()
    defer { stateLock.unlock() }
    return changeSyncKindStorage
  }

  /// Port of `lsp_request_timeout` — same durations as lsp.rs.
  static func requestTimeout(for method: String) -> TimeInterval {
    switch method {
    case "textDocument/completion", "textDocument/hover", "textDocument/signatureHelp":
      return 5
    case "textDocument/definition", "textDocument/declaration",
      "textDocument/typeDefinition", "textDocument/implementation",
      "textDocument/references":
      return 15
    case "textDocument/rename", "textDocument/prepareRename":
      return 15
    case "textDocument/codeAction":
      return 10
    case "initialize":
      return 30
    case "shutdown":
      return 5
    default:
      return 15
    }
  }

  // MARK: Lifecycle

  /// Port of `LspClient::start`: spawn, wire up reader/stderr/exit handling,
  /// then run the initialize handshake (blocking, 30s timeout). On handshake
  /// failure the child is not killed (matching Rust); its eventual exit still
  /// produces a `serverExited` event.
  static func start(
    command: String,
    args: [String],
    rootUri: String,
    serverId: String,
    clientKey: String,
    initializationOptions: Any?,
    onEvent: @escaping (LSPEvent) -> Void
  ) -> Result<ServerProcess, String> {
    lspLog(
      "LSP: starting server '\(command)' with args \(args) for server_id '\(serverId)', root_uri=\(rootUri), key=\(clientKey)"
    )

    let client = ServerProcess(
      command: command, args: args, rootUri: rootUri, serverId: serverId,
      clientKey: clientKey, onEvent: onEvent)
    if let error = client.launch() {
      return .failure("Failed to start LSP server '\(command)': \(error)")
    }

    if case .failure(let error) = client.initialize(initializationOptions: initializationOptions) {
      return .failure(error)
    }
    lspLog("LSP: server '\(command)' initialized successfully for key=\(clientKey)")
    return .success(client)
  }

  private init(
    command: String,
    args: [String],
    rootUri: String,
    serverId: String,
    clientKey: String,
    onEvent: @escaping (LSPEvent) -> Void
  ) {
    self.commandName = command
    self.rootUri = rootUri
    self.serverId = serverId
    self.clientKey = clientKey
    self.onEvent = onEvent

    let process = Process()
    process.executableURL = URL(fileURLWithPath: command)
    process.arguments = args
    // Servers start with `#!/usr/bin/env node` and run other tools: give
    // them the login shell's PATH, not launchd's.
    process.environment = ManagedServers.toolEnvironment()
    let stdinPipe = Pipe()
    let stdoutPipe = Pipe()
    let stderrPipe = Pipe()
    process.standardInput = stdinPipe
    process.standardOutput = stdoutPipe
    process.standardError = stderrPipe
    self.process = process
    self.stdinHandle = stdinPipe.fileHandleForWriting
    self.stdoutHandle = stdoutPipe.fileHandleForReading
    self.stderrHandle = stderrPipe.fileHandleForReading
  }

  /// Returns an error description on spawn failure, nil on success.
  private func launch() -> String? {
    // The termination handler retains self, keeping the client (and its
    // pending-request drain) alive until the child exits — the equivalent of
    // the detached tokio tasks in Rust. Set before run() so an
    // immediately-exiting child is never missed.
    process.terminationHandler = { [self] proc in
      lspLog(
        "LSP server '\(commandName)' exited with status: \(proc.terminationStatus) (key=\(clientKey))"
      )
      handleExit()
      proc.terminationHandler = nil
    }

    do {
      try process.run()
    } catch {
      process.terminationHandler = nil
      return error.localizedDescription
    }

    // Writes to a dead server must fail with EPIPE, not kill the app.
    _ = fcntl(stdinHandle.fileDescriptor, F_SETNOSIGPIPE, 1)

    Thread.detachNewThread { [self] in
      readLoop()
    }
    Thread.detachNewThread { [self] in
      stderrLoop()
    }
    return nil
  }

  private func handleExit() {
    stateLock.lock()
    if exited {
      stateLock.unlock()
      return
    }
    exited = true
    let drained = pending
    pending.removeAll()
    stateLock.unlock()

    // Drain all pending requests so waiting callers get an error instead of
    // hanging.
    if !drained.isEmpty {
      lspLog(
        "Draining \(drained.count) pending LSP request(s) for crashed server '\(commandName)'")
      for (_, waiter) in drained {
        waiter.complete(.failure("LSP server exited unexpectedly"))
      }
    }
    onEvent(.serverExited(clientKey: clientKey, serverId: serverId))
  }

  // MARK: Reading

  private func readLoop() {
    let parser = FrameParser()
    while true {
      let data = stdoutHandle.availableData
      if data.isEmpty { return }  // EOF
      for body in parser.feed(data) {
        handleMessage(body)
      }
      if parser.failed { return }
    }
  }

  private func stderrLoop() {
    var bytesRead: UInt64 = 0
    let logLimit: UInt64 = 10 * 1024 * 1024
    var lineBuffer: [UInt8] = []
    while true {
      let data = stderrHandle.availableData
      if data.isEmpty { return }
      bytesRead += UInt64(data.count)
      if bytesRead > logLimit {
        // Stop logging but keep draining so the server doesn't block on a
        // full pipe buffer.
        while !stderrHandle.availableData.isEmpty {}
        return
      }
      for byte in data {
        lineBuffer.append(byte)
        if byte == 0x0A {
          logStderrLine(lineBuffer)
          lineBuffer.removeAll(keepingCapacity: true)
        }
      }
    }
  }

  private func logStderrLine(_ bytes: [UInt8]) {
    var line = String(decoding: bytes, as: UTF8.self)
    if line.utf8.count > 8192 {
      line = String(line.prefix(8192))
    }
    let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
    if !trimmed.isEmpty {
      lspLog("LSP stderr [\(commandName):\(clientKey)]: \(trimmed)")
    }
  }

  private func handleMessage(_ body: [UInt8]) {
    guard let raw = try? JSONSerialization.jsonObject(with: Data(body)),
      let message = raw as? [String: Any],
      message["jsonrpc"] is String
    else {
      lspLog("Failed to parse LSP message")
      return
    }
    // serde requires `method` to be a string and `error` to be a well-formed
    // object when present; anything else failed the whole message parse.
    if let rawMethod = message["method"], !(rawMethod is NSNull), !(rawMethod is String) {
      lspLog("Failed to parse LSP message")
      return
    }
    var errorPair: (code: Int64, message: String)?
    if let rawError = message["error"], !(rawError is NSNull) {
      guard let errorObject = rawError as? [String: Any],
        let code = JSONUtil.asInt64(errorObject["code"]),
        let text = errorObject["message"] as? String
      else {
        lspLog("Failed to parse LSP message")
        return
      }
      errorPair = (code, text)
    }

    let method = message["method"] as? String
    let hasId = message["id"] != nil

    if hasId && method == nil {
      // Response to one of our requests.
      if let idNumber = JSONUtil.asInt64(message["id"]) {
        stateLock.lock()
        let waiter = pending.removeValue(forKey: idNumber)
        stateLock.unlock()
        if let waiter {
          if let errorPair {
            waiter.complete(.failure("LSP error \(errorPair.code): \(errorPair.message)"))
          } else {
            waiter.complete(.success(message["result"] ?? NSNull()))
          }
        }
      }
      return
    }

    if let method {
      let params = message["params"]
      if hasId {
        handleServerRequest(method: method, id: message["id"] ?? NSNull(), params: params)
        return
      }
      handleServerNotification(method: method, params: params)
    }
  }

  // MARK: Server requests / notifications

  private func handleServerRequest(method: String, id: Any, params: Any?) {
    switch method {
    case "workspace/configuration":
      let items = (params as? [String: Any])?["items"] as? [Any]
      let result: [Any]
      if let items {
        result = items.map { item -> Any in
          let section = (item as? [String: Any])?["section"] as? String ?? ""
          return workspaceConfigurationForSection(section)
        }
      } else {
        result = []
      }
      sendResult(id: id, result: result)
    case "window/workDoneProgress/create":
      sendResult(id: id, result: NSNull())
    case "workspace/workspaceFolders":
      let folder: [String: Any] = [
        "uri": rootUri,
        "name": FileURI.workspaceFolderName(rootUri),
      ]
      sendResult(id: id, result: [folder])
    case "client/registerCapability", "client/unregisterCapability":
      sendResult(id: id, result: NSNull())
    case "workspace/applyEdit":
      // The app applies it and answers with `respond(id:result:)`.
      let object = params as? [String: Any]
      guard let edit = object?["edit"] else {
        sendError(id: id, code: -32602, message: "Missing edit")
        return
      }
      onEvent(
        .applyEdit(
          clientKey: clientKey, serverId: serverId, id: id, label: object?["label"] as? String, edit: edit))
    case "window/showMessageRequest":
      // No buttons: show it like a message and answer "dismissed".
      if let object = params as? [String: Any], let text = object["message"] as? String {
        onEvent(
          .showMessage(
            serverId: serverId, type: JSONUtil.asInt64(object["type"]) ?? 3, message: text))
      }
      sendResult(id: id, result: NSNull())
    default:
      sendError(id: id, code: -32601, message: "Method not found")
    }
  }

  private func handleServerNotification(method: String, params: Any?) {
    switch method {
    case "textDocument/publishDiagnostics":
      if let event = Self.parseDiagnosticsEvent(params) {
        onEvent(event)
      }
    case "window/showMessage":
      if let object = params as? [String: Any], let text = object["message"] as? String {
        onEvent(
          .showMessage(
            serverId: serverId, type: JSONUtil.asInt64(object["type"]) ?? 3, message: text))
      }
    case "$/progress":
      if let event = Self.parseProgress(params, clientKey: clientKey, serverId: serverId) {
        onEvent(event)
      }
    case "window/logMessage", "$/logTrace":
      // log::debug! in Rust — intentionally quiet here.
      break
    default:
      // log::debug!("Unhandled LSP notification: {}", method)
      break
    }
  }

  /// Strict parse of `PublishDiagnosticsParams` matching serde's
  /// all-or-nothing behavior: any malformed diagnostic drops the whole
  /// event. Diagnostics are capped at 1000 entries.
  static func parseDiagnosticsEvent(_ params: Any?) -> LSPEvent? {
    guard let object = params as? [String: Any],
      let uri = object["uri"] as? String,
      let rawDiagnostics = object["diagnostics"] as? [Any]
    else { return nil }

    var version: Int64?
    if let rawVersion = object["version"], !(rawVersion is NSNull) {
      guard let value = JSONUtil.asInt64(rawVersion),
        value >= Int64(Int32.min), value <= Int64(Int32.max)
      else { return nil }
      version = value
    }

    var diagnostics: [LSPDiagnostic] = []
    diagnostics.reserveCapacity(rawDiagnostics.count)
    for raw in rawDiagnostics {
      guard let diagnostic = raw as? [String: Any],
        let range = diagnostic["range"] as? [String: Any],
        let start = range["start"] as? [String: Any],
        let end = range["end"] as? [String: Any],
        let startLine = JSONUtil.asUInt32(start["line"]),
        let startColumn = JSONUtil.asUInt32(start["character"]),
        let endLine = JSONUtil.asUInt32(end["line"]),
        let endColumn = JSONUtil.asUInt32(end["character"]),
        let message = diagnostic["message"] as? String
      else { return nil }

      var severity: Int64?
      if let rawSeverity = diagnostic["severity"], !(rawSeverity is NSNull) {
        guard let value = JSONUtil.asInt64(rawSeverity) else { return nil }
        severity = value
      }
      var source: String?
      if let rawSource = diagnostic["source"], !(rawSource is NSNull) {
        guard let value = rawSource as? String else { return nil }
        source = value
      }

      diagnostics.append(
        LSPDiagnostic(
          severity: severity, startLine: startLine, startColumn: startColumn,
          endLine: endLine, endColumn: endColumn, message: message, source: source))
    }

    // Cap diagnostics to prevent memory exhaustion from malicious servers.
    let maxDiagnostics = 1000
    if diagnostics.count > maxDiagnostics {
      lspLog("Truncating diagnostics for \(uri) from \(diagnostics.count) to \(maxDiagnostics)")
      diagnostics = Array(diagnostics.prefix(maxDiagnostics))
    }

    return .diagnostics(uri: uri, version: version, diagnostics: diagnostics)
  }

  /// Work-done progress (`begin` / `report` / `end`) as an event.
  static func parseProgress(_ params: Any?, clientKey: String, serverId: String) -> LSPEvent? {
    guard let object = params as? [String: Any],
      let value = object["value"] as? [String: Any],
      let kind = value["kind"] as? String, ["begin", "report", "end"].contains(kind)
    else { return nil }
    let token: String
    if let text = object["token"] as? String {
      token = text
    } else if let number = JSONUtil.asInt64(object["token"]) {
      token = String(number)
    } else {
      return nil
    }
    return .progress(
      clientKey: clientKey, serverId: serverId, token: token, kind: kind,
      title: value["title"] as? String, message: value["message"] as? String,
      percentage: JSONUtil.asInt64(value["percentage"]))
  }

  // MARK: Writing

  /// Answer a request the server sent (`workspace/applyEdit`).
  func respond(id: Any, result: Any) {
    sendResult(id: id, result: result)
  }

  private func sendResult(id: Any, result: Any) {
    _ = send(["jsonrpc": "2.0", "id": id, "result": result])
  }

  private func sendError(id: Any, code: Int, message: String) {
    _ = send([
      "jsonrpc": "2.0",
      "id": id,
      "error": ["code": code, "message": message] as [String: Any],
    ])
  }

  private func send(_ message: [String: Any]) -> Bool {
    guard let body = JSONUtil.encodeData(message) else { return false }
    var frame = Data("Content-Length: \(body.count)\r\n\r\n".utf8)
    frame.append(body)
    writeLock.lock()
    defer { writeLock.unlock() }
    do {
      try stdinHandle.write(contentsOf: frame)
      return true
    } catch {
      return false
    }
  }

  // MARK: Requests / notifications

  /// Blocking JSON-RPC request with the same per-method timeout table as
  /// lsp.rs. Safe to call from any non-main thread.
  func request(method: String, params: Any?) -> Result<Any, String> {
    switch begin(method: method, params: params) {
    case .failure(let error):
      return .failure(error)
    case .success(let (id, waiter)):
      return finish(method: method, id: id, waiter: waiter)
    }
  }

  private func begin(method: String, params: Any?) -> Result<(Int64, PendingRequest), String> {
    stateLock.lock()
    let id = nextId
    nextId += 1
    let waiter = PendingRequest()
    pending[id] = waiter
    stateLock.unlock()

    let message: [String: Any] = [
      "jsonrpc": "2.0",
      "id": id,
      "method": method,
      "params": params ?? NSNull(),
    ]
    guard send(message) else {
      stateLock.lock()
      pending.removeValue(forKey: id)
      stateLock.unlock()
      return .failure("failed to write to LSP server stdin")
    }
    return .success((id, waiter))
  }

  private func finish(method: String, id: Int64, waiter: PendingRequest) -> Result<Any, String> {
    let timeout = Self.requestTimeout(for: method)
    if let result = waiter.wait(timeout: timeout) {
      return result
    }
    // Remove the pending request so a late response is ignored.
    stateLock.lock()
    pending.removeValue(forKey: id)
    stateLock.unlock()
    return .failure("LSP request '\(method)' timed out after \(Int(timeout))s")
  }

  @discardableResult
  func notify(method: String, params: Any?) -> Bool {
    send(["jsonrpc": "2.0", "method": method, "params": params ?? NSNull()])
  }

  /// Port of `did_change_with_changes`: incremental changes are forwarded
  /// only when non-empty and the server advertised incremental sync;
  /// otherwise the full text is sent as a single change.
  func didChangeWithChanges(
    uri: String, version: Int32, fullText: String, changes: [ContentChange]
  ) -> Bool {
    let useIncremental = !changes.isEmpty && changeSyncKind == 2
    let contentChanges: [[String: Any]] =
      useIncremental
      ? changes.map { $0.toJSONObject() }
      : [["text": fullText]]
    return notify(
      method: "textDocument/didChange",
      params: [
        "textDocument": ["uri": uri, "version": version] as [String: Any],
        "contentChanges": contentChanges,
      ])
  }

  /// Port of `LspClient::shutdown`: shutdown request (5s timeout), drain
  /// pending requests, then the exit notification.
  func shutdown() {
    _ = request(method: "shutdown", params: NSNull())
    stateLock.lock()
    let drained = pending
    pending.removeAll()
    stateLock.unlock()
    for (_, waiter) in drained {
      waiter.complete(.failure("LSP server shutting down"))
    }
    _ = notify(method: "exit", params: NSNull())
  }

  // MARK: Initialize handshake

  private func initialize(initializationOptions: Any?) -> Result<Void, String> {
    var params: [String: Any] = [
      // lsp-types serializes processId unconditionally; the Rust client left
      // it at its None default.
      "processId": NSNull(),
      "rootUri": rootUri,
      "workspaceFolders": [
        ["uri": rootUri, "name": FileURI.workspaceFolderName(rootUri)] as [String: Any]
      ],
      "capabilities": Self.clientCapabilities,
      "clientInfo": ["name": "Impulse", "version": lspClientVersion] as [String: Any],
    ]
    if let initializationOptions {
      params["initializationOptions"] = initializationOptions
    }

    let result: Any
    switch request(method: "initialize", params: params) {
    case .failure(let error):
      return .failure(error)
    case .success(let value):
      result = value
    }

    if let object = result as? [String: Any],
      let capabilities = object["capabilities"] as? [String: Any]
    {
      stateLock.lock()
      serverCapabilitiesStorage = capabilities
      changeSyncKindStorage = Self.textDocumentSyncKind(capabilities)
      stateLock.unlock()
    }

    guard notify(method: "initialized", params: [String: Any]()) else {
      return .failure("failed to send initialized notification")
    }

    // Push workspace configuration to the server. Many servers (e.g.
    // typescript-language-server) use the push model (didChangeConfiguration)
    // rather than the pull model (workspace/configuration) for their main
    // settings.
    guard
      notify(
        method: "workspace/didChangeConfiguration",
        params: ["settings": defaultWorkspaceSettings()])
    else {
      return .failure("failed to send workspace/didChangeConfiguration")
    }

    onEvent(.initialized(clientKey: clientKey, serverId: serverId))
    return .success(())
  }

  /// Port of `text_document_sync_kind`.
  /// Whether `capabilities` advertise support for request `method`. Unknown
  /// methods count as supported (the server gets to answer).
  static func supports(method: String, capabilities: [String: Any]?) -> Bool {
    let keys: [String: String] = [
      "textDocument/completion": "completionProvider",
      "textDocument/hover": "hoverProvider",
      "textDocument/definition": "definitionProvider",
      "textDocument/typeDefinition": "typeDefinitionProvider",
      "textDocument/implementation": "implementationProvider",
      "textDocument/references": "referencesProvider",
      "textDocument/documentSymbol": "documentSymbolProvider",
      "textDocument/formatting": "documentFormattingProvider",
      "textDocument/rangeFormatting": "documentRangeFormattingProvider",
      "textDocument/signatureHelp": "signatureHelpProvider",
      "textDocument/codeAction": "codeActionProvider",
      "textDocument/rename": "renameProvider",
      "textDocument/prepareRename": "renameProvider",
      "workspace/symbol": "workspaceSymbolProvider",
      "textDocument/documentHighlight": "documentHighlightProvider",
      "textDocument/inlayHint": "inlayHintProvider",
      "workspace/executeCommand": "executeCommandProvider",
      "codeAction/resolve": "codeActionProvider",
    ]
    guard let key = keys[method] else { return true }
    guard let value = capabilities?[key] else { return false }
    // prepareRename needs RenameOptions with prepareProvider, not just `true`.
    if method == "textDocument/prepareRename" {
      return (value as? [String: Any])?["prepareProvider"] as? Bool ?? false
    }
    if method == "codeAction/resolve" {
      return (value as? [String: Any])?["resolveProvider"] as? Bool ?? false
    }
    if let flag = value as? Bool { return flag }
    return !(value is NSNull)
  }

  static func textDocumentSyncKind(_ capabilities: [String: Any]) -> Int? {
    guard let sync = capabilities["textDocumentSync"] else { return nil }
    if let kind = JSONUtil.asInt64(sync) {
      return Int(kind)
    }
    if let options = sync as? [String: Any], let change = JSONUtil.asInt64(options["change"]) {
      return Int(change)
    }
    return nil
  }

  /// The exact `ClientCapabilities` the Rust client sent (lsp-types
  /// serialization: camelCase, absent optionals skipped).
  static let clientCapabilities: [String: Any] = {
    let markupFormats = ["plaintext", "markdown"]
    return [
      "workspace": [
        "configuration": true,
        "workspaceFolders": true,
        "applyEdit": true,
        "workspaceEdit": [
          "documentChanges": true,
          "resourceOperations": ["create", "rename", "delete"],
          "failureHandling": "abort",
        ] as [String: Any],
        "executeCommand": [String: Any](),
      ] as [String: Any],
      "window": [
        "workDoneProgress": true,
        "showMessage": [String: Any](),
      ] as [String: Any],
      "textDocument": [
        "synchronization": [
          "dynamicRegistration": false,
          "willSave": false,
          "willSaveWaitUntil": false,
          "didSave": true,
        ],
        "completion": [
          "completionItem": [
            "snippetSupport": true,
            "documentationFormat": markupFormats,
            "insertReplaceSupport": true,
          ]
        ],
        "hover": [
          "contentFormat": markupFormats
        ],
        "publishDiagnostics": [
          "relatedInformation": true,
          "versionSupport": true,
        ],
        "definition": ["linkSupport": true],
        "declaration": ["linkSupport": true],
        "typeDefinition": ["linkSupport": true],
        "implementation": ["linkSupport": true],
        "references": [String: Any](),
        "rename": ["prepareSupport": true],
        "signatureHelp": [
          "signatureInformation": [
            "documentationFormat": markupFormats
          ]
        ],
        "codeAction": [
          "codeActionLiteralSupport": [
            "codeActionKind": [
              "valueSet": ["quickfix", "refactor", "source"]
            ]
          ],
          "isPreferredSupport": true,
          "dataSupport": true,
          "resolveSupport": ["properties": ["edit"]],
        ] as [String: Any],
        "documentHighlight": [String: Any](),
        "inlayHint": [String: Any](),
      ] as [String: Any],
    ]
  }()
}
