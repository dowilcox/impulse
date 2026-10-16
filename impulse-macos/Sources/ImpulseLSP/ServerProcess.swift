// Port of `LspClient` from impulse-core/src/lsp.rs: child process with piped
// stdio, LSP base-protocol framing (`Content-Length` headers), request/id
// correlation with per-method timeouts, the initialize handshake, server
// request/notification handling, stderr logging, and exit detection. Nothing
// here waits on a server: requests finish from its reader thread (or a
// timer), and messages go out on a queue of its own.

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

// MARK: - Requests

/// A request on its way to a language server. It finishes once: with the
/// server's answer, a failure, or a timeout. `cancel()` withdraws it (the
/// server is told with `$/cancelRequest`), and its completion isn't called.
public final class LSPRequest {
  private let lock = NSLock()
  private var completion: ((Result<Any, String>) -> Void)?
  /// Takes it back from the server it was sent to.
  private var withdraw: (() -> Void)?
  private var done = false

  init(completion: @escaping (Result<Any, String>) -> Void) {
    self.completion = completion
  }

  public var isFinished: Bool {
    lock.withLock { done }
  }

  public func cancel() {
    giveUp(nil)
  }

  /// It was sent: from now on, giving up on it takes it back from the server.
  func sent(withdraw: @escaping () -> Void) {
    lock.lock()
    let late = done
    if !late { self.withdraw = withdraw }
    lock.unlock()
    if late { withdraw() }
  }

  /// The answer, or why there is none. Only the first one counts.
  func finish(_ result: Result<Any, String>) {
    lock.lock()
    let completion = done ? nil : self.completion
    done = true
    self.completion = nil
    withdraw = nil
    lock.unlock()
    completion?(result)
  }

  /// Still unanswered after `timeout`: it fails, and the server is told to
  /// stop working on it.
  func expire(after timeout: TimeInterval, method: String) {
    DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + timeout) { [weak self] in
      self?.giveUp(.failure("LSP request '\(method)' timed out after \(Int(timeout))s"))
    }
  }

  private func giveUp(_ result: Result<Any, String>?) {
    lock.lock()
    let completion = done ? nil : self.completion
    let withdraw = done ? nil : self.withdraw
    done = true
    self.completion = nil
    self.withdraw = nil
    lock.unlock()
    withdraw?()
    if let result { completion?(result) }
  }
}

// MARK: - Server process

/// One running LSP server (the Rust `LspClient`).
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

  /// Messages go out here, in order, so a server that stops reading holds
  /// up only its own queue (never the caller, or the reader thread that
  /// answers the server's requests).
  private let writeQueue: DispatchQueue
  /// writeQueue only.
  private var inputClosed = false
  private let stateLock = NSLock()
  private var pending: [Int64: LSPRequest] = [:]
  private var nextId: Int64 = 1
  private var exited = false
  /// Shut down or given up on: nothing it sends reaches the app any more.
  private var retired = false
  private var exitWaiters: [() -> Void] = []
  /// Bytes queued for its input and not written yet.
  private var queuedBytes = 0
  private var serverCapabilitiesStorage: [String: Any]?
  private var changeSyncKindStorage: Int?

  /// A server this far behind on reading its input is taken as hung and
  /// stopped (the registry restarts it).
  static let maxQueuedBytes = 64 * 1024 * 1024
  /// After the exit notification, how long a server gets to end on its own.
  static let exitGracePeriod: TimeInterval = 2

  /// Whether the process has ended.
  var hasExited: Bool {
    stateLock.lock()
    defer { stateLock.unlock() }
    return exited
  }

  /// Whether it has been shut down (or given up on).
  var isRetired: Bool {
    stateLock.lock()
    defer { stateLock.unlock() }
    return retired
  }

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

  /// How long a request may take (waiting for its server to start
  /// included): as long as the editor waits for the same request
  /// (editor.js), so neither side keeps waiting after the other gave up.
  static func requestTimeout(for method: String) -> TimeInterval {
    switch method {
    case "textDocument/completion", "textDocument/hover", "textDocument/signatureHelp",
      "textDocument/definition", "textDocument/prepareRename", "textDocument/documentHighlight",
      "textDocument/inlayHint":
      return 5
    case "textDocument/codeAction", "textDocument/formatting":
      return 10
    case "textDocument/declaration", "textDocument/typeDefinition", "textDocument/implementation",
      "textDocument/references", "textDocument/rename":
      return 15
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
  /// then run the initialize handshake (30s timeout), calling `completion`
  /// once it's done. On handshake failure the child is stopped.
  static func start(
    command: String,
    args: [String],
    rootUri: String,
    serverId: String,
    clientKey: String,
    initializationOptions: Any?,
    onEvent: @escaping (LSPEvent) -> Void,
    completion: @escaping (Result<ServerProcess, String>) -> Void
  ) {
    lspLog(
      "LSP: starting server '\(command)' with args \(args) for server_id '\(serverId)', root_uri=\(rootUri), key=\(clientKey)"
    )

    let client = ServerProcess(
      command: command, args: args, rootUri: rootUri, serverId: serverId,
      clientKey: clientKey, onEvent: onEvent)
    if let error = client.launch() {
      completion(.failure("Failed to start LSP server '\(command)': \(error)"))
      return
    }

    client.initialize(initializationOptions: initializationOptions) { result in
      switch result {
      case .failure(let error):
        // Don't leave it running: the next attempt would start another.
        client.retire()
        client.terminate()
        completion(.failure(error))
      case .success:
        lspLog("LSP: server '\(command)' initialized successfully for key=\(clientKey)")
        completion(.success(client))
      }
    }
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
    self.writeQueue = DispatchQueue(label: "impulse.lsp.write", qos: .userInitiated)
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
    let waiters = exitWaiters
    exitWaiters.removeAll()
    stateLock.unlock()

    // Drain all pending requests so waiting callers get an error instead of
    // hanging.
    if !drained.isEmpty {
      lspLog(
        "Draining \(drained.count) pending LSP request(s) for crashed server '\(commandName)'")
      for (_, waiter) in drained {
        waiter.finish(.failure("LSP server exited unexpectedly"))
      }
    }
    emit(.serverExited(clientKey: clientKey, serverId: serverId))
    for waiter in waiters { waiter() }
  }

  /// Events reach the registry only while the server serves.
  private func emit(_ event: LSPEvent) {
    guard !isRetired else { return }
    onEvent(event)
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
            waiter.finish(.failure("LSP error \(errorPair.code): \(errorPair.message)"))
          } else {
            waiter.finish(.success(message["result"] ?? NSNull()))
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
      guard !isRetired else {
        sendResult(id: id, result: ["applied": false, "failureReason": "The language server is shutting down."])
        return
      }
      emit(
        .applyEdit(
          clientKey: clientKey, serverId: serverId, id: id, label: object?["label"] as? String, edit: edit))
    case "window/showMessageRequest":
      // No buttons: show it like a message and answer "dismissed".
      if let object = params as? [String: Any], let text = object["message"] as? String {
        emit(
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
        emit(event)
      }
    case "window/showMessage":
      if let object = params as? [String: Any], let text = object["message"] as? String {
        emit(
          .showMessage(
            serverId: serverId, type: JSONUtil.asInt64(object["type"]) ?? 3, message: text))
      }
    case "$/progress":
      if let event = Self.parseProgress(params, clientKey: clientKey, serverId: serverId) {
        emit(event)
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

  /// Queue a message for the server (see `writeQueue`). False when it has
  /// ended, or has stopped reading.
  @discardableResult
  private func send(_ message: [String: Any]) -> Bool {
    guard let body = JSONUtil.encodeData(message) else { return false }
    var frame = Data("Content-Length: \(body.count)\r\n\r\n".utf8)
    frame.append(body)
    stateLock.lock()
    if exited {
      stateLock.unlock()
      return false
    }
    queuedBytes += frame.count
    let backlog = queuedBytes
    stateLock.unlock()
    if backlog > Self.maxQueuedBytes {
      lspLog("LSP server '\(commandName)' stopped reading its input; stopping it (key=\(clientKey))")
      terminate()
      return false
    }
    writeQueue.async { [self] in
      if !inputClosed {
        // A failed write means it ended: the exit handler deals with that.
        try? stdinHandle.write(contentsOf: frame)
      }
      stateLock.lock()
      queuedBytes -= frame.count
      stateLock.unlock()
    }
    return true
  }

  // MARK: Requests / notifications

  /// Send `method` as `request`, which the server's answer finishes (see
  /// `LSPRequest`). Giving up on it forgets it here and tells the server.
  func send(method: String, params: Any?, as request: LSPRequest) {
    stateLock.lock()
    if exited {
      stateLock.unlock()
      request.finish(.failure("LSP server exited unexpectedly"))
      return
    }
    let id = nextId
    nextId += 1
    pending[id] = request
    stateLock.unlock()
    request.sent { [weak self] in self?.withdraw(id) }

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
      request.finish(.failure("failed to write to LSP server stdin"))
      return
    }
  }

  /// A request nobody waits for any more: a late answer is ignored, and the
  /// server can stop working on it.
  private func withdraw(_ id: Int64) {
    stateLock.lock()
    let wasPending = pending.removeValue(forKey: id) != nil
    stateLock.unlock()
    if wasPending { notify(method: "$/cancelRequest", params: ["id": id]) }
  }

  @discardableResult
  func notify(method: String, params: Any?) -> Bool {
    send(["jsonrpc": "2.0", "method": method, "params": params ?? NSNull()])
  }

  /// Port of `did_change_with_changes`: incremental changes are forwarded
  /// only when non-empty and the server advertised incremental sync;
  /// otherwise the full text is sent as a single change (asked for only
  /// then).
  func didChange(uri: String, version: Int32, changes: [ContentChange], fullText: () -> String) -> Bool {
    let useIncremental = !changes.isEmpty && changeSyncKind == 2
    let contentChanges: [[String: Any]] =
      useIncremental
      ? changes.map { $0.toJSONObject() }
      : [["text": fullText()]]
    return notify(
      method: "textDocument/didChange",
      params: [
        "textDocument": ["uri": uri, "version": version] as [String: Any],
        "contentChanges": contentChanges,
      ])
  }

  /// From now on, nothing it sends reaches the app.
  func retire() {
    stateLock.lock()
    retired = true
    stateLock.unlock()
  }

  /// Stop the process: close its input, SIGTERM, and SIGKILL if it's still
  /// there two seconds later.
  func terminate() {
    closeInput()
    guard process.isRunning else { return }
    process.terminate()
    let pid = process.processIdentifier
    DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 2) { [process] in
      if process.isRunning { kill(pid, SIGKILL) }
    }
  }

  /// Close its input after what's queued for it.
  private func closeInput() {
    writeQueue.async { [self] in
      guard !inputClosed else { return }
      inputClosed = true
      try? stdinHandle.close()
    }
  }

  /// Port of `LspClient::shutdown`, without waiting for it: retire it, send
  /// the shutdown request, and once it's answered (or not, within 5 s)
  /// fail the requests still pending, send exit and close its input. A
  /// server still running `exitGracePeriod` later is stopped (SIGTERM, then
  /// SIGKILL). `exited` runs when the process is gone.
  func shutdown(exited: (() -> Void)? = nil) {
    stateLock.lock()
    retired = true
    let gone = self.exited
    if let exited, !gone { exitWaiters.append(exited) }
    stateLock.unlock()
    if gone {
      exited?()
      return
    }
    let request = LSPRequest { [self] _ in
      stateLock.lock()
      let drained = pending
      pending.removeAll()
      stateLock.unlock()
      for (_, waiter) in drained {
        waiter.finish(.failure("LSP server shutting down"))
      }
      notify(method: "exit", params: NSNull())
      closeInput()
      DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + Self.exitGracePeriod) { [self] in
        terminate()
      }
    }
    send(method: "shutdown", params: NSNull(), as: request)
    request.expire(after: Self.requestTimeout(for: "shutdown"), method: "shutdown")
  }

  // MARK: Initialize handshake

  private func initialize(
    initializationOptions: Any?, completion: @escaping (Result<Void, String>) -> Void
  ) {
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

    let request = LSPRequest { [self] response in
      switch response {
      case .failure(let error):
        completion(.failure(error))
      case .success(let result):
        completion(initialized(result))
      }
    }
    send(method: "initialize", params: params, as: request)
    request.expire(after: Self.requestTimeout(for: "initialize"), method: "initialize")
  }

  /// The rest of the handshake, once the server answered `initialize`.
  private func initialized(_ result: Any) -> Result<Void, String> {
    if let object = result as? [String: Any],
      let capabilities = object["capabilities"] as? [String: Any]
    {
      stateLock.lock()
      serverCapabilitiesStorage = capabilities
      changeSyncKindStorage = Self.textDocumentSyncKind(capabilities)
      stateLock.unlock()
      // Positions here are UTF-16, the only encoding offered; a server that
      // picks another anyway puts its columns off on non-ASCII lines.
      if let encoding = capabilities["positionEncoding"] as? String, encoding != "utf-16" {
        lspLog("LSP server '\(commandName)' chose position encoding '\(encoding)', not the offered utf-16")
      }
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

    emit(.initialized(clientKey: clientKey, serverId: serverId))
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
      // Monaco's columns and the document copies count UTF-16 code units.
      "general": ["positionEncodings": ["utf-16"]],
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
          ],
          // What `LSPCompletion.parse` fills in from a list's defaults.
          "completionList": ["itemDefaults": ["editRange", "insertTextFormat"]],
        ] as [String: Any],
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
