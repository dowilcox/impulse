// Port of `LspRegistry` from impulse-core/src/lsp.rs plus the FFI facade
// from impulse-ffi/src/lib.rs: per-language server lifecycle (start-failure
// cooldown, concurrent-start dedupe), the per-registry document cache, the
// bounded event queue, and the poll-shaped synchronous call surface the app
// uses (`impulse_lsp_ensure_servers` / `_request` / `_notify` /
// `_did_change` / `_poll_event` / `_shutdown_all`).

import Foundation

public final class LSPRegistry {
  /// Port of `START_RETRY_COOLDOWN`.
  public static let startRetryCooldown: TimeInterval = 15
  /// Port of `LSP_EVENT_CHANNEL_CAPACITY`.
  static let eventQueueCapacity = 10_000
  /// 50 iterations * 120ms sleep = ~6s waiting for a concurrent start.
  static let maxStartingWaitIterations = 50

  private let config: LSPConfig
  private let fallbackRootUri: String

  private let stateLock = NSLock()
  private var clients: [String: ServerProcess] = [:]
  private var failedUntil: [String: Date] = [:]
  private var starting: Set<String> = []
  /// Recent crashes per client, for the restart backoff.
  private var crashes: [String: (count: Int, last: Date)] = [:]
  private var isShutDown = false
  /// Servers start here, so notifications (and the app's LSP queue behind
  /// them) never wait the up to 30 s an initialize handshake can take.
  private let startQueue = DispatchQueue(
    label: "impulse.lsp.start", qos: .userInitiated, attributes: .concurrent)

  private let eventLock = NSLock()
  private var eventQueue: [LSPEvent] = []
  private var eventQueueHead = 0

  /// Called (on a server's reader thread) when an event lands in an empty
  /// queue, so a client can poll on demand instead of on a timer.
  public var onEventsAvailable: (() -> Void)? {
    get {
      eventLock.lock()
      defer { eventLock.unlock() }
      return eventsAvailableHandler
    }
    set {
      eventLock.lock()
      eventsAvailableHandler = newValue
      eventLock.unlock()
    }
  }
  private var eventsAvailableHandler: (() -> Void)?

  /// Open documents, so a server that starts after they were opened (or
  /// restarts after a crash) is told about them. Lock order: documentsLock,
  /// then stateLock.
  private struct TrackedDocument {
    var languageId: String
    var version: Int32
    var text: String
  }
  private let documentsLock = NSLock()
  private var documents: [String: TrackedDocument] = [:]

  public convenience init(rootUri: String) {
    self.init(rootUri: rootUri, config: LSPConfig.load(fallbackRootUri: rootUri))
  }

  /// Internal seam for tests: inject a config instead of loading from disk.
  init(rootUri: String, config: LSPConfig) {
    self.fallbackRootUri = rootUri
    self.config = config
  }

  // MARK: Public facade (mirrors the FFI surface)

  /// Port of `impulse_lsp_ensure_servers`: ensures LSP servers are running
  /// for the given language and file, returning the number of clients
  /// started/found.
  @discardableResult
  public func ensureServers(languageId: String, fileUri: String) -> Int {
    return getClients(languageId: languageId, fileUri: fileUri).count
  }

  /// Port of `impulse_lsp_request`: blocking JSON-RPC request against the
  /// first LSP server for the language. Returns the result JSON, or an
  /// `{"error": "..."}` envelope exactly like the FFI. Safe to call from any
  /// non-main thread.
  public func request(languageId: String, fileUri: String, method: String, paramsJSON: String?) -> String {
    let params = paramsJSON.flatMap { JSONUtil.parse($0) }
    guard let client = client(for: method, params: params, languageId: languageId, fileUri: fileUri) else {
      return "{\"error\":\"no LSP client available\"}"
    }
    return Self.encodeResponse(client.request(method: method, params: params))
  }

  /// Answer a request a server sent (by its client key and request id).
  public func respond(clientKey: String, id: Any, resultJSON: String) {
    stateLock.lock()
    let client = clients[clientKey]
    stateLock.unlock()
    client?.respond(id: id, result: JSONUtil.parse(resultJSON) ?? NSNull())
  }

  /// The first server that can answer: several can serve one language
  /// (typescript + eslint + tailwind), and only some support each request.
  /// A command goes to the server that registered it.
  private func client(for method: String, params: Any?, languageId: String, fileUri: String) -> ServerProcess? {
    let clients = getClients(languageId: languageId, fileUri: fileUri)
    if method == "workspace/executeCommand", let command = (params as? [String: Any])?["command"] as? String,
      let owner = clients.first(where: {
        let provider = $0.serverCapabilities?["executeCommandProvider"] as? [String: Any]
        return (provider?["commands"] as? [String])?.contains(command) ?? false
      })
    {
      return owner
    }
    return clients.first(where: {
      ServerProcess.supports(method: method, capabilities: $0.serverCapabilities)
    }) ?? clients.first
  }

  private static func encodeResponse(_ result: Result<Any, String>) -> String {
    switch result {
    case .success(let value):
      if let json = JSONUtil.encode(value) {
        return json
      }
      lspLog("JSON serialization failed for LSP response")
      return JSONUtil.encode(["error": "serialization failed"])
        ?? "{\"error\":\"serialization failed\"}"
    case .failure(let error):
      return JSONUtil.encode(["error": error]) ?? "{\"error\":\"serialization failed\"}"
    }
  }

  /// Port of `impulse_lsp_notify`: sends a notification to every running
  /// LSP server for the language (each needs didOpen/didClose to answer
  /// about the document), updating the document cache on didOpen/didClose.
  /// Servers that aren't running yet start in the background and get the
  /// open documents when they're ready. Returns true when at least one send
  /// succeeded.
  @discardableResult
  public func notify(languageId: String, fileUri: String, method: String, paramsJSON: String?) -> Bool {
    let params = paramsJSON.flatMap { JSONUtil.parse($0) } ?? NSNull()
    documentsLock.lock()
    defer { documentsLock.unlock() }
    updateDocumentCache(method: method, params: params)
    var ok = false
    for client in runningClients(languageId: languageId, fileUri: fileUri) {
      ok = client.notify(method: method, params: params) || ok
    }
    return ok
  }

  /// Port of `impulse_lsp_did_change`: updates the document cache (full text
  /// or incremental changes applied via `DocumentCache`), then sends
  /// capability-aware didChange to every client for the language. Returns
  /// true when at least one notification succeeded.
  @discardableResult
  public func didChange(
    languageId: String, fileUri: String, version: Int32, fullText: String?,
    changesJSON: String?
  ) -> Bool {
    let changes = changesJSON.flatMap { ContentChange.parseArray($0) } ?? []

    documentsLock.lock()
    defer { documentsLock.unlock() }
    var text = documents[fileUri]?.text ?? ""
    if let fullText {
      text = fullText
    } else {
      DocumentCache.applyContentChanges(to: &text, changes: changes)
    }
    documents[fileUri] = TrackedDocument(
      languageId: documents[fileUri]?.languageId ?? languageId, version: version, text: text)

    var ok = false
    for client in runningClients(languageId: languageId, fileUri: fileUri) {
      ok =
        client.didChangeWithChanges(
          uri: fileUri, version: version, fullText: text, changes: changes) || ok
    }
    return ok
  }

  /// Port of `impulse_lsp_poll_event`: next queued event as the same JSON
  /// envelope the FFI emitted, or nil when the queue is empty.
  public func pollEvent() -> String? {
    eventLock.lock()
    defer { eventLock.unlock() }
    while eventQueueHead < eventQueue.count {
      let event = eventQueue[eventQueueHead]
      eventQueueHead += 1
      if eventQueueHead == eventQueue.count {
        eventQueue.removeAll(keepingCapacity: true)
        eventQueueHead = 0
      } else if eventQueueHead > 1024 {
        eventQueue.removeFirst(eventQueueHead)
        eventQueueHead = 0
      }
      if let json = event.encodeJSON() {
        return json
      }
    }
    return nil
  }

  /// Port of `LspRegistry::shutdown_all` — shuts down every running server.
  public func shutdownAll() {
    let snapshot: [ServerProcess]
    stateLock.lock()
    isShutDown = true
    snapshot = Array(clients.values)
    stateLock.unlock()

    for client in snapshot {
      client.shutdown()
    }
  }

  // MARK: Events

  private func enqueue(_ event: LSPEvent) {
    eventLock.lock()
    if eventQueue.count - eventQueueHead >= Self.eventQueueCapacity {
      eventLock.unlock()
      lspLog("LSP event channel full (\(Self.eventQueueCapacity) capacity), dropping event")
      return
    }
    let wasEmpty = eventQueue.count == eventQueueHead
    eventQueue.append(event)
    let handler = wasEmpty ? eventsAvailableHandler : nil
    eventLock.unlock()
    handler?()
  }

  // MARK: Document cache

  /// Port of `update_lsp_document_cache_for_notify` (documentsLock held).
  private func updateDocumentCache(method: String, params: Any) {
    switch method {
    case "textDocument/didOpen":
      guard let object = params as? [String: Any],
        let document = object["textDocument"] as? [String: Any],
        let uri = document["uri"] as? String,
        let text = document["text"] as? String
      else { return }
      documents[uri] = TrackedDocument(
        languageId: document["languageId"] as? String ?? "",
        version: (document["version"] as? NSNumber)?.int32Value ?? 1, text: text)
    case "textDocument/didClose":
      guard let object = params as? [String: Any],
        let document = object["textDocument"] as? [String: Any],
        let uri = document["uri"] as? String
      else { return }
      documents.removeValue(forKey: uri)
    default:
      break
    }
  }

  // MARK: Client lifecycle

  private func resolveServerIds(languageId: String) -> [String] {
    if let ids = config.languageServers[languageId] {
      return ids
    }
    if config.servers[languageId] != nil {
      return [languageId]
    }
    return []
  }

  private func detectRootUri(fileUri: String) -> String {
    LSPConfig.detectProjectRoot(fileUri: fileUri, markers: config.rootMarkers)
      ?? fallbackRootUri
  }

  static func clientKey(serverId: String, rootUri: String) -> String {
    "\(serverId)@\(rootUri)"
  }

  /// The servers for a file that are up now; the others start in the
  /// background (see `startInBackground`).
  private func runningClients(languageId: String, fileUri: String) -> [ServerProcess] {
    let serverIds = resolveServerIds(languageId: languageId)
    if serverIds.isEmpty {
      return []
    }
    let rootUri = detectRootUri(fileUri: fileUri)
    var out: [ServerProcess] = []
    for serverId in serverIds {
      stateLock.lock()
      let client = clients[Self.clientKey(serverId: serverId, rootUri: rootUri)]
      stateLock.unlock()
      if let client {
        out.append(client)
      } else {
        startInBackground(serverId: serverId, rootUri: rootUri)
      }
    }
    return out
  }

  /// Start a server off the caller's thread (unless it's running, starting,
  /// or in its retry cooldown). It gets the open documents once it's up.
  private func startInBackground(serverId: String, rootUri: String) {
    let key = Self.clientKey(serverId: serverId, rootUri: rootUri)
    stateLock.lock()
    if isShutDown || clients[key] != nil || starting.contains(key)
      || failedUntil[key].map({ Date() < $0 }) == true
    {
      stateLock.unlock()
      return
    }
    failedUntil.removeValue(forKey: key)
    starting.insert(key)
    stateLock.unlock()
    startQueue.async { [weak self] in
      guard let self else { return }
      _ = self.startServer(serverId: serverId, rootUri: rootUri, clientKey: key)
      self.stateLock.lock()
      self.starting.remove(key)
      self.stateLock.unlock()
    }
  }

  /// A server process ended. If it was serving (not shutting down, not a
  /// failed start), drop it and bring it back after a growing delay for the
  /// documents it was serving.
  private func serverExited(clientKey: String, serverId: String, rootUri: String) {
    stateLock.lock()
    guard !isShutDown, let client = clients[clientKey], client.hasExited else {
      stateLock.unlock()
      return
    }
    clients.removeValue(forKey: clientKey)
    var crash = crashes[clientKey] ?? (count: 0, last: .distantPast)
    if Date().timeIntervalSince(crash.last) > 300 { crash.count = 0 }
    crash.count += 1
    crash.last = Date()
    crashes[clientKey] = crash
    let delay: TimeInterval = [2, 10, 30, 120][min(crash.count - 1, 3)]
    failedUntil[clientKey] = Date().addingTimeInterval(delay)
    stateLock.unlock()

    lspLog("LSP server '\(serverId)' (key=\(clientKey)) exited; restarting in \(Int(delay))s")
    enqueue(
      .serverError(
        clientKey: clientKey, serverId: serverId,
        message: "The \(serverId) language server stopped unexpectedly. Impulse restarts it."))
    documentsLock.lock()
    let serving = documents.contains { uri, document in
      resolveServerIds(languageId: document.languageId).contains(serverId)
        && detectRootUri(fileUri: uri) == rootUri
    }
    documentsLock.unlock()
    if serving {
      startQueue.asyncAfter(deadline: .now() + delay + 0.1) { [weak self] in
        self?.startInBackground(serverId: serverId, rootUri: rootUri)
      }
    }
  }

  /// Port of `LspRegistry::get_clients`. Starts missing servers and waits
  /// for them (requests, which run off the app's LSP queue).
  func getClients(languageId: String, fileUri: String) -> [ServerProcess] {
    let serverIds = resolveServerIds(languageId: languageId)
    if serverIds.isEmpty {
      return []
    }
    let rootUri = detectRootUri(fileUri: fileUri)
    var out: [ServerProcess] = []
    for serverId in serverIds {
      if let client = getOrStartClient(serverId: serverId, rootUri: rootUri) {
        out.append(client)
      }
    }
    return out
  }

  /// Port of `get_or_start_client`: reuse a running client, honor the
  /// start-failure cooldown, and dedupe concurrent starts (waiting up to
  /// ~6s for another thread's start attempt).
  private func getOrStartClient(serverId: String, rootUri: String) -> ServerProcess? {
    let key = Self.clientKey(serverId: serverId, rootUri: rootUri)
    var waitIterations = 0

    while true {
      stateLock.lock()
      if let client = clients[key] {
        stateLock.unlock()
        return client
      }
      if let until = failedUntil[key] {
        if Date() < until {
          stateLock.unlock()
          return nil
        }
        failedUntil.removeValue(forKey: key)
      }
      if starting.contains(key) {
        stateLock.unlock()
        waitIterations += 1
        if waitIterations >= Self.maxStartingWaitIterations {
          lspLog(
            "LSP server '\(serverId)' at '\(rootUri)' still starting after \(waitIterations) iterations (~\(UInt64(waitIterations) * 120 / 1000)s), giving up"
          )
          return nil
        }
        Thread.sleep(forTimeInterval: 0.12)
        continue
      }
      starting.insert(key)
      stateLock.unlock()
      break
    }

    let result = startServer(serverId: serverId, rootUri: rootUri, clientKey: key)
    stateLock.lock()
    starting.remove(key)
    stateLock.unlock()
    return result
  }

  /// Port of `start_server`: resolve the command (PATH + managed bin dir),
  /// start + initialize the client, and on failure record the 15s cooldown
  /// and emit a serverError event.
  private func startServer(serverId: String, rootUri: String, clientKey: String) -> ServerProcess? {
    guard let serverConfig = config.servers[serverId] else {
      lspLog("No LSP server configured for server id: \(serverId)")
      return nil
    }

    guard let resolvedCommand = ManagedServers.resolveLspCommandPath(serverConfig.command) else {
      let message = ManagedServers.missingCommandMessage(
        serverId: serverId, command: serverConfig.command)
      lspLog("LSP startup skipped for \(serverId): \(message)")
      stateLock.lock()
      failedUntil[clientKey] = Date().addingTimeInterval(Self.startRetryCooldown)
      stateLock.unlock()
      enqueue(.serverError(clientKey: clientKey, serverId: serverId, message: message))
      return nil
    }

    let initOptions =
      serverConfig.initializationOptions
      ?? LSPConfig.defaultInitOptions(serverId: serverId)

    switch ServerProcess.start(
      command: resolvedCommand,
      args: serverConfig.args,
      rootUri: rootUri,
      serverId: serverId,
      clientKey: clientKey,
      initializationOptions: initOptions,
      onEvent: { [weak self] event in
        if case .serverExited(let key, let id) = event {
          self?.serverExited(clientKey: key, serverId: id, rootUri: rootUri)
        }
        self?.enqueue(event)
      })
    {
    case .success(let client):
      // Documents opened before it was up (or before it crashed) first,
      // then it takes notifications like the others: holding the documents
      // lock throughout means no change slips in between.
      documentsLock.lock()
      for (uri, document) in documents
      where resolveServerIds(languageId: document.languageId).contains(serverId)
        && detectRootUri(fileUri: uri) == rootUri
      {
        client.notify(
          method: "textDocument/didOpen",
          params: [
            "textDocument": [
              "uri": uri, "languageId": document.languageId, "version": document.version,
              "text": document.text,
            ] as [String: Any]
          ])
      }
      stateLock.lock()
      clients[clientKey] = client
      stateLock.unlock()
      documentsLock.unlock()
      return client
    case .failure(let error):
      lspLog("Failed to start LSP server for '\(serverId)' (key='\(clientKey)'): \(error)")
      stateLock.lock()
      failedUntil[clientKey] = Date().addingTimeInterval(Self.startRetryCooldown)
      stateLock.unlock()
      enqueue(.serverError(clientKey: clientKey, serverId: serverId, message: error))
      return nil
    }
  }
}
