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

  private let documentsLock = NSLock()
  private var documents: [String: String] = [:]

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
    let clients = getClients(languageId: languageId, fileUri: fileUri)
    guard let client = clients.first else {
      return "{\"error\":\"no LSP client available\"}"
    }
    switch client.request(method: method, params: params) {
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

  /// Port of `impulse_lsp_notify`: sends a notification to the first LSP
  /// server for the language, updating the document cache on
  /// didOpen/didClose like the FFI glue. Returns true on success.
  @discardableResult
  public func notify(languageId: String, fileUri: String, method: String, paramsJSON: String?) -> Bool {
    let params = paramsJSON.flatMap { JSONUtil.parse($0) } ?? NSNull()
    updateDocumentCache(method: method, params: params)
    let clients = getClients(languageId: languageId, fileUri: fileUri)
    guard let client = clients.first else { return false }
    return client.notify(method: method, params: params)
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
    let clients = getClients(languageId: languageId, fileUri: fileUri)

    documentsLock.lock()
    defer { documentsLock.unlock() }
    var document = documents[fileUri] ?? ""
    if let fullText {
      document = fullText
    } else {
      DocumentCache.applyContentChanges(to: &document, changes: changes)
    }
    documents[fileUri] = document

    var ok = false
    for client in clients {
      ok =
        client.didChangeWithChanges(
          uri: fileUri, version: version, fullText: document, changes: changes) || ok
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

  /// Port of `update_lsp_document_cache_for_notify`.
  private func updateDocumentCache(method: String, params: Any) {
    switch method {
    case "textDocument/didOpen":
      guard let object = params as? [String: Any],
        let document = object["textDocument"] as? [String: Any],
        let uri = document["uri"] as? String,
        let text = document["text"] as? String
      else { return }
      documentsLock.lock()
      documents[uri] = text
      documentsLock.unlock()
    case "textDocument/didClose":
      guard let object = params as? [String: Any],
        let document = object["textDocument"] as? [String: Any],
        let uri = document["uri"] as? String
      else { return }
      documentsLock.lock()
      documents.removeValue(forKey: uri)
      documentsLock.unlock()
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

  /// Port of `LspRegistry::get_clients`.
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
      onEvent: { [weak self] event in self?.enqueue(event) })
    {
    case .success(let client):
      stateLock.lock()
      clients[clientKey] = client
      stateLock.unlock()
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
