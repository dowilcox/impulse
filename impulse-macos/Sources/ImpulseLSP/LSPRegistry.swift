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

  /// A fixed configuration (tests); otherwise it comes from the lsp.json
  /// files, per project, re-read when they change (see `config(forFileUri:)`).
  private let fixedConfig: LSPConfig?
  private let globalConfigPath: String?
  private let fallbackRootUri: String

  /// The configuration files are looked at again at most this often.
  var configRecheckInterval: TimeInterval = 2
  /// Leaf lock: nothing else is locked while it's held.
  private let configLock = NSLock()
  private var baseConfig: (stamp: String?, config: LSPConfig)?
  /// By project folder: the stamps of everything it was built from.
  private var projectConfigs: [String: (stamps: [String?], config: LSPConfig)] = [:]
  /// By a file's folder: its configuration and when that was worked out.
  private var resolvedConfigs: [String: (at: Date, config: LSPConfig)] = [:]

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
  /// By open document: the servers (client keys) it has been opened in, so
  /// servers that stop serving it (a project's lsp.json sent its language
  /// elsewhere) close it. documentsLock.
  private var openIn: [String: Set<String>] = [:]
  /// By document: servers that closed it that way, whose late diagnostics
  /// for it are dropped. Leaf lock (the servers' reader threads read it).
  private let detachedLock = NSLock()
  private var detached: [String: Set<String>] = [:]

  /// Whether servers may run for a file (by URI): some run a project's own
  /// code, so the app allows only trusted folders. Unset: every file.
  public var isAllowed: ((String) -> Bool)?

  /// Whether any server is configured for a language (in a file's project
  /// when given, else by the global configuration).
  public func hasServers(languageId: String, fileUri: String? = nil) -> Bool {
    let config = fileUri.map { self.config(forFileUri: $0) } ?? currentBaseConfig().config
    return !Self.serverIds(languageId: languageId, config: config).isEmpty
  }

  /// Shut down the servers whose project root (a path) matches, e.g. those
  /// in a folder that isn't trusted any more.
  public func shutdownServers(where matches: @escaping (String) -> Bool) {
    stateLock.lock()
    let stopping = clients.filter { _, client in FileURI.toPath(client.rootUri).map(matches) ?? false }
    for key in stopping.keys { clients.removeValue(forKey: key) }
    stateLock.unlock()
    for client in stopping.values {
      startQueue.async { client.shutdown() }
    }
  }

  /// Servers for files without a project root of their own work in
  /// `rootUri`. The configuration comes from `~/.config/impulse/lsp.json`
  /// and each project's `.impulse/lsp.json`.
  public convenience init(rootUri: String) {
    self.init(rootUri: rootUri, globalConfigPath: LSPConfig.globalLspConfigPath())
  }

  /// Internal seam for tests: a global config file of their own.
  init(rootUri: String, globalConfigPath: String?) {
    self.fallbackRootUri = rootUri
    self.fixedConfig = nil
    self.globalConfigPath = globalConfigPath
  }

  /// Internal seam for tests: inject a config instead of loading from disk.
  init(rootUri: String, config: LSPConfig) {
    self.fallbackRootUri = rootUri
    self.fixedConfig = config
    self.globalConfigPath = nil
  }

  // MARK: Configuration

  /// The configuration for a file: defaults, the global config, then the
  /// project config that applies to it (see
  /// `LSPConfig.projectConfigFolder`). Worked out again, from the files,
  /// once `configRecheckInterval` has passed (or now, with `recheck`), so
  /// edits apply without a restart. When that sends a language to other
  /// servers, the open documents move to them (see `syncOpenDocuments`).
  func config(forFileUri fileUri: String, recheck: Bool = false) -> LSPConfig {
    if let fixedConfig { return fixedConfig }
    let directory = FileURI.toPath(fileUri).map { ($0 as NSString).deletingLastPathComponent } ?? ""
    configLock.lock()
    let previous = resolvedConfigs[directory]
    if !recheck, let previous, Date().timeIntervalSince(previous.at) < configRecheckInterval {
      configLock.unlock()
      return previous.config
    }
    configLock.unlock()

    let base = currentBaseConfig()
    var config = base.config
    if !directory.isEmpty, let folder = LSPConfig.projectConfigFolder(forDirectory: directory) {
      let stamps = [base.stamp] + LSPConfig.projectConfigPaths(in: folder).map(LSPConfig.fileStamp)
      configLock.lock()
      let cached = projectConfigs[folder]
      configLock.unlock()
      if let cached, cached.stamps == stamps {
        config = cached.config
      } else {
        config.applyProjectConfig(in: folder)
        configLock.lock()
        projectConfigs[folder] = (stamps, config)
        configLock.unlock()
      }
    }
    configLock.lock()
    resolvedConfigs[directory] = (Date(), config)
    configLock.unlock()
    // Callers may hold documentsLock: the documents move on another thread.
    if !recheck, let previous,
      previous.config.languageServers != config.languageServers || previous.config.rootMarkers != config.rootMarkers
    {
      startQueue.async { [weak self] in self?.syncOpenDocuments() }
    }
    return config
  }

  /// Defaults plus the global config, re-read when that file changes.
  /// Servers whose command, arguments or options changed are stopped, so
  /// they start again with the new ones.
  private func currentBaseConfig() -> (stamp: String?, config: LSPConfig) {
    if let fixedConfig { return (nil, fixedConfig) }
    let stamp = globalConfigPath.flatMap(LSPConfig.fileStamp)
    configLock.lock()
    if let baseConfig, baseConfig.stamp == stamp {
      configLock.unlock()
      return baseConfig
    }
    configLock.unlock()
    let config = LSPConfig.load(globalConfigPath: globalConfigPath, projectFolder: nil)
    configLock.lock()
    let previous = baseConfig?.config
    baseConfig = (stamp, config)
    configLock.unlock()
    if let previous {
      let changed = Set(previous.servers.keys).union(config.servers.keys).filter { id in
        !Self.sameServer(previous.servers[id], config.servers[id])
      }
      if !changed.isEmpty { restartServers(ids: changed) }
    }
    return (stamp, config)
  }

  private static func sameServer(_ a: LSPServerConfig?, _ b: LSPServerConfig?) -> Bool {
    guard let a, let b else { return a == nil && b == nil }
    guard a.command == b.command, a.args == b.args else { return false }
    switch (a.initializationOptions, b.initializationOptions) {
    case (nil, nil): return true
    case let (x?, y?): return (x as? NSObject)?.isEqual(y) ?? false
    default: return false
    }
  }

  /// Stop the running servers with these ids (and forget start failures);
  /// they start again, with the current configuration, when next needed.
  private func restartServers(ids: Set<String>) {
    stateLock.lock()
    let stopping = clients.filter { ids.contains($0.value.serverId) }
    for key in stopping.keys { clients.removeValue(forKey: key) }
    failedUntil = failedUntil.filter { key, _ in !ids.contains { key.hasPrefix("\($0)@") } }
    stateLock.unlock()
    for client in stopping.values {
      lspLog("LSP server '\(client.serverId)' configuration changed; restarting it")
      startQueue.async { client.shutdown() }
    }
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
    var clients = runningClients(languageId: languageId, fileUri: fileUri)
    switch method {
    case "textDocument/didOpen":
      let keys = Set(clients.map(\.clientKey))
      openIn[fileUri] = keys
      reattach(fileUri, to: keys)
    case "textDocument/didClose":
      // Servers it was opened in that don't serve it any more close it too.
      let serving = Set(clients.map(\.clientKey))
      clients += (openIn[fileUri] ?? []).subtracting(serving).compactMap(runningClient)
      openIn.removeValue(forKey: fileUri)
      detachedLock.lock()
      detached.removeValue(forKey: fileUri)
      detachedLock.unlock()
    default:
      syncDocument(fileUri)
    }
    var ok = false
    for client in clients {
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
    // Before the change: a server that gets the document now gets the text
    // the change applies to.
    syncDocument(fileUri)
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

  /// Makes the servers an open document is open in the ones that serve it
  /// now (documentsLock held). Servers that no longer do (the project's
  /// lsp.json sent its language to others, or moved its root) close it,
  /// and their diagnostics for it are cleared; running servers that serve
  /// it but don't have it get it.
  private func syncDocument(_ uri: String, recheck: Bool = false) {
    guard let document = documents[uri] else { return }
    let serving = servingKeys(languageId: document.languageId, fileUri: uri, recheck: recheck)
    let opened = openIn[uri] ?? []
    let stale = opened.subtracting(serving)
    let opening = serving.subtracting(opened).compactMap(runningClient)
    guard !stale.isEmpty || !opening.isEmpty else { return }
    openIn[uri] = opened.intersection(serving).union(opening.map(\.clientKey))
    if !stale.isEmpty {
      detachedLock.lock()
      detached[uri, default: []].formUnion(stale)
      detachedLock.unlock()
      for client in stale.compactMap(runningClient) {
        client.notify(method: "textDocument/didClose", params: ["textDocument": ["uri": uri]])
      }
      // The servers that serve it now publish their own.
      enqueue(.diagnostics(uri: uri, version: nil, diagnostics: []))
    }
    reattach(uri, to: Set(opening.map(\.clientKey)))
    for client in opening {
      client.notify(method: "textDocument/didOpen", params: Self.didOpenParams(uri: uri, document: document))
    }
  }

  /// A project's configuration changed: every open document goes to the
  /// servers that serve it now.
  private func syncOpenDocuments() {
    stateLock.lock()
    let shutDown = isShutDown
    stateLock.unlock()
    guard !shutDown else { return }
    documentsLock.lock()
    defer { documentsLock.unlock() }
    for uri in Array(documents.keys) {
      syncDocument(uri, recheck: true)
    }
  }

  /// The servers (client keys) that serve a file now.
  private func servingKeys(languageId: String, fileUri: String, recheck: Bool) -> Set<String> {
    guard isAllowed?(fileUri) != false else { return [] }
    let config = self.config(forFileUri: fileUri, recheck: recheck)
    let rootUri = LSPConfig.detectProjectRoot(fileUri: fileUri, markers: config.rootMarkers) ?? fallbackRootUri
    return Set(Self.serverIds(languageId: languageId, config: config).map { Self.clientKey(serverId: $0, rootUri: rootUri) })
  }

  private func runningClient(_ key: String) -> ServerProcess? {
    stateLock.lock()
    defer { stateLock.unlock() }
    return clients[key]
  }

  /// These servers have the document open (again): their diagnostics count.
  private func reattach(_ uri: String, to keys: Set<String>) {
    guard !keys.isEmpty else { return }
    detachedLock.lock()
    if let remaining = detached[uri]?.subtracting(keys) {
      detached[uri] = remaining.isEmpty ? nil : remaining
    }
    detachedLock.unlock()
  }

  private func isDetached(_ uri: String, from key: String) -> Bool {
    detachedLock.lock()
    defer { detachedLock.unlock() }
    return detached[uri]?.contains(key) ?? false
  }

  private static func didOpenParams(uri: String, document: TrackedDocument) -> [String: Any] {
    [
      "textDocument": [
        "uri": uri, "languageId": document.languageId, "version": document.version, "text": document.text,
      ] as [String: Any]
    ]
  }

  // MARK: Client lifecycle

  private func resolveServerIds(languageId: String, fileUri: String) -> [String] {
    Self.serverIds(languageId: languageId, config: config(forFileUri: fileUri))
  }

  private static func serverIds(languageId: String, config: LSPConfig) -> [String] {
    if let ids = config.languageServers[languageId] {
      return ids
    }
    if config.servers[languageId] != nil {
      return [languageId]
    }
    return []
  }

  private func detectRootUri(fileUri: String) -> String {
    LSPConfig.detectProjectRoot(fileUri: fileUri, markers: config(forFileUri: fileUri).rootMarkers)
      ?? fallbackRootUri
  }

  static func clientKey(serverId: String, rootUri: String) -> String {
    "\(serverId)@\(rootUri)"
  }

  /// The servers for a file that are up now; the others start in the
  /// background (see `startInBackground`).
  private func runningClients(languageId: String, fileUri: String) -> [ServerProcess] {
    let serverIds = resolveServerIds(languageId: languageId, fileUri: fileUri)
    if serverIds.isEmpty || isAllowed?(fileUri) == false {
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
      resolveServerIds(languageId: document.languageId, fileUri: uri).contains(serverId)
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
    let serverIds = resolveServerIds(languageId: languageId, fileUri: fileUri)
    if serverIds.isEmpty || isAllowed?(fileUri) == false {
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
    // Server commands come only from the global config.
    guard let serverConfig = currentBaseConfig().config.servers[serverId] else {
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
        // A document this server closed when it stopped serving it.
        if case .diagnostics(let uri, _, _) = event, self?.isDetached(uri, from: clientKey) == true { return }
        self?.enqueue(event)
      })
    {
    case .success(let client):
      // Documents opened before it was up (or before it crashed) first,
      // then it takes notifications like the others: holding the documents
      // lock throughout means no change slips in between.
      documentsLock.lock()
      // A new process: nothing is open in it yet.
      openIn = openIn.mapValues { $0.subtracting([clientKey]) }
      for (uri, document) in documents
      where isAllowed?(uri) != false
        && resolveServerIds(languageId: document.languageId, fileUri: uri).contains(serverId)
        && detectRootUri(fileUri: uri) == rootUri
      {
        openIn[uri, default: []].insert(clientKey)
        reattach(uri, to: [clientKey])
        client.notify(method: "textDocument/didOpen", params: Self.didOpenParams(uri: uri, document: document))
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
