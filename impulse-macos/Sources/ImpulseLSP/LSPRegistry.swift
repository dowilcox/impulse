// Port of `LspRegistry` from impulse-core/src/lsp.rs plus the FFI facade
// from impulse-ffi/src/lib.rs: per-language server lifecycle (start-failure
// cooldown, concurrent-start dedupe), the per-registry document cache, the
// bounded event queue, and the poll-shaped call surface the app uses
// (`impulse_lsp_ensure_servers` / `_request` / `_notify` / `_did_change` /
// `_poll_event` / `_shutdown_all`). Requests don't block: they complete from
// the server's reader thread, or when they time out.

import Foundation
import ImpulseKit

public final class LSPRegistry {
  /// Port of `START_RETRY_COOLDOWN`.
  public static let startRetryCooldown: TimeInterval = 15
  /// Port of `LSP_EVENT_CHANNEL_CAPACITY`.
  static let eventQueueCapacity = 10_000
  /// More changes than this in one edit (Replace All) go to servers as the
  /// whole text: applying thousands of edits one at a time costs a server
  /// more than reading the text once.
  static let maxIncrementalChanges = 64
  /// Request completions run here (short work: nothing waits on it).
  private static let callbackQueue = DispatchQueue.global(qos: .userInitiated)

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

  /// A file's project root is looked for again at most this often.
  var rootRecheckInterval: TimeInterval = 30
  /// Leaf lock.
  private let rootsLock = NSLock()
  /// By file: its project root, and the root markers and time it was found
  /// with (so an edit doesn't walk up the folders every time).
  private var roots: [String: (markers: [String], at: Date, root: String)] = [:]

  private let stateLock = NSLock()
  private var clients: [String: ServerProcess] = [:]
  private var failedUntil: [String: Date] = [:]
  private var starting: Set<String> = []
  /// By client key: what waits for a start in progress.
  private var startWaiters: [String: [(ServerProcess?) -> Void]] = [:]
  /// Recent crashes per client, for the restart backoff.
  private var crashes: [String: (count: Int, last: Date)] = [:]
  private var isShutDown = false
  /// Servers start here, so notifications (and the app's LSP queue behind
  /// them) never wait for a process launch.
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
    var text: DocumentText
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

  /// The trusted folder a file (by URI) is in, when trust applies: its
  /// servers' project root stays inside it, so trusting a folder never
  /// makes an untrusted one above it a server's root. Nil: no limit.
  public var trustedFolder: ((String) -> String?)?

  /// Whether any server is configured for a language (in a file's project
  /// when given, else by the global configuration).
  public func hasServers(languageId: String, fileUri: String? = nil) -> Bool {
    let config = fileUri.map { self.config(forFileUri: $0) } ?? currentBaseConfig().config
    return !Self.serverIds(languageId: languageId, config: config).isEmpty
  }

  /// Shut down the servers whose project root (a path) matches, e.g. those
  /// in a folder that isn't trusted any more. Project roots are worked out
  /// again, since what's trusted bounds them.
  public func shutdownServers(where matches: @escaping (String) -> Bool) {
    rootsLock.withLock { roots.removeAll() }
    stateLock.lock()
    let stopping = clients.filter { _, client in FileURI.toPath(client.rootUri).map(matches) ?? false }
    for key in stopping.keys { clients.removeValue(forKey: key) }
    stateLock.unlock()
    for client in stopping.values {
      client.shutdown()
    }
  }

  /// Whether the server with this client key is still running, and still
  /// allowed where it works: only then may what it asks for (its edits)
  /// be carried out.
  public func isServing(clientKey: String) -> Bool {
    stateLock.lock()
    let client = clients[clientKey]
    stateLock.unlock()
    guard let client, !client.isRetired, !client.hasExited else { return false }
    return isAllowed?(client.rootUri) != false
  }

  /// The project root (a path) of the server with this client key.
  public func rootPath(clientKey: String) -> String? {
    stateLock.lock()
    defer { stateLock.unlock() }
    return clients[clientKey].flatMap { FileURI.toPath($0.rootUri) }
  }

  /// The project root (a path) a file's servers work in.
  public func rootPath(forFileUri fileUri: String) -> String? {
    FileURI.toPath(projectRoot(forFileUri: fileUri))
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
      client.shutdown()
    }
  }

  /// The project root (URI) a file's servers work in (see
  /// `LSPConfig.detectProjectRoot`), inside the trusted folder the file is
  /// in. Kept per file for `rootRecheckInterval` (or until its root
  /// markers change, or `recheck`).
  func projectRoot(forFileUri fileUri: String, config: LSPConfig? = nil, recheck: Bool = false) -> String {
    let markers = (config ?? self.config(forFileUri: fileUri)).rootMarkers
    rootsLock.lock()
    if !recheck, let cached = roots[fileUri], cached.markers == markers,
      Date().timeIntervalSince(cached.at) < rootRecheckInterval
    {
      rootsLock.unlock()
      return cached.root
    }
    rootsLock.unlock()
    let ceiling = trustedFolder?(fileUri)
    let root =
      LSPConfig.detectProjectRoot(fileUri: fileUri, markers: markers, within: ceiling)
      ?? ceiling.flatMap(FileURI.fromPath) ?? fallbackRootUri
    rootsLock.lock()
    // Files that were only asked about (never opened) don't pile up.
    if roots.count >= 2048 { roots.removeAll() }
    roots[fileUri] = (markers, Date(), root)
    rootsLock.unlock()
    return root
  }

  // MARK: Public facade (mirrors the FFI surface)

  /// Port of `impulse_lsp_ensure_servers`: ensures LSP servers are running
  /// for the given language and file, returning the number of clients
  /// started/found. Blocks until their starts finish (tests).
  @discardableResult
  func ensureServers(languageId: String, fileUri: String) -> Int {
    let done = DispatchSemaphore(value: 0)
    var count = 0
    withClients(languageId: languageId, fileUri: fileUri) { clients in
      count = clients.count
      done.signal()
    }
    done.wait()
    return count
  }

  /// Port of `impulse_lsp_request`: a JSON-RPC request to the first server
  /// for the language that supports it, started first if it isn't running.
  /// `completion` gets (on a background thread) the result JSON, or an
  /// `{"error": "..."}` envelope exactly like the FFI. Nothing waits in the
  /// meantime; the request gives up after its method's timeout (see
  /// `ServerProcess.requestTimeout`), and `cancel()` on what's returned
  /// withdraws it (completion isn't called then).
  @discardableResult
  public func request(
    languageId: String, fileUri: String, method: String, paramsJSON: String?,
    completion: @escaping (String) -> Void
  ) -> LSPRequest {
    let params = paramsJSON.flatMap { JSONUtil.parse($0) }
    let request = LSPRequest { result in
      Self.callbackQueue.async { completion(Self.encodeResponse(result)) }
    }
    request.expire(after: ServerProcess.requestTimeout(for: method), method: method)
    withClients(languageId: languageId, fileUri: fileUri) { clients in
      guard !request.isFinished else { return }
      guard let client = Self.client(for: method, params: params, among: clients) else {
        request.finish(.failure("no LSP client available"))
        return
      }
      client.send(method: method, params: params, as: request)
    }
    return request
  }

  /// `request`, waiting for the answer (tests).
  func request(languageId: String, fileUri: String, method: String, paramsJSON: String?) -> String {
    let done = DispatchSemaphore(value: 0)
    var response = ""
    request(languageId: languageId, fileUri: fileUri, method: method, paramsJSON: paramsJSON) {
      response = $0
      done.signal()
    }
    done.wait()
    return response
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
  private static func client(for method: String, params: Any?, among clients: [ServerProcess]) -> ServerProcess? {
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
  /// open documents when they're ready. A document no server is configured
  /// for (or allowed to see) isn't kept. Returns true when at least one send
  /// succeeded.
  @discardableResult
  public func notify(languageId: String, fileUri: String, method: String, paramsJSON: String?) -> Bool {
    let params = paramsJSON.flatMap { JSONUtil.parse($0) } ?? NSNull()
    documentsLock.lock()
    defer { documentsLock.unlock() }
    if method == "textDocument/didOpen",
      resolveServerIds(languageId: languageId, fileUri: fileUri).isEmpty || isAllowed?(fileUri) == false
    {
      return false
    }
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
      rootsLock.withLock { _ = roots.removeValue(forKey: fileUri) }
    default:
      syncDocument(fileUri)
    }
    var ok = false
    for client in clients {
      ok = client.notify(method: method, params: params) || ok
    }
    return ok
  }

  /// Port of `impulse_lsp_did_change`: updates the copy of an open document
  /// (to `fullText`, or with the incremental `changes`), then sends
  /// capability-aware didChange to every client for the language: the
  /// changes to servers that take them (unless there are more than
  /// `maxIncrementalChanges`), the whole text to the others. Returns true
  /// when at least one notification succeeded; false for a document that
  /// isn't open.
  @discardableResult
  public func didChange(
    languageId: String, fileUri: String, version: Int32, fullText: String?, changes: [ContentChange]
  ) -> Bool {
    documentsLock.lock()
    defer { documentsLock.unlock() }
    guard documents[fileUri] != nil else { return false }
    // Before the change: a server that gets the document now gets the text
    // the change applies to.
    syncDocument(fileUri)
    // In place: the text isn't copied.
    if let fullText {
      documents[fileUri]?.text = DocumentText(fullText)
    } else {
      documents[fileUri]?.text.apply(changes)
    }
    documents[fileUri]?.version = version

    let incremental = fullText == nil && changes.count <= Self.maxIncrementalChanges ? changes : []
    var whole: String?
    let wholeText = { () -> String in
      if let whole { return whole }
      let text = self.documents[fileUri]?.text.text ?? ""
      whole = text
      return text
    }
    var ok = false
    for client in runningClients(languageId: languageId, fileUri: fileUri) {
      ok = client.didChange(uri: fileUri, version: version, changes: incremental, fullText: wholeText) || ok
    }
    return ok
  }

  /// `didChange` with the changes as a JSON array of
  /// `TextDocumentContentChangeEvent` (none when it doesn't parse).
  @discardableResult
  public func didChange(
    languageId: String, fileUri: String, version: Int32, fullText: String?, changesJSON: String?
  ) -> Bool {
    didChange(
      languageId: languageId, fileUri: fileUri, version: version, fullText: fullText,
      changes: changesJSON.flatMap { ContentChange.parseArray($0) } ?? [])
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

  /// Port of `LspRegistry::shutdown_all`: shuts every running server down,
  /// all at once, and waits up to `deadline` for them to go; any still
  /// running then are stopped (SIGTERM), without waiting further.
  public func shutdownAll(deadline: TimeInterval = 2) {
    stateLock.lock()
    isShutDown = true
    let snapshot = Array(clients.values)
    stateLock.unlock()

    let group = DispatchGroup()
    for client in snapshot {
      group.enter()
      client.shutdown(exited: { group.leave() })
    }
    if group.wait(timeout: .now() + deadline) == .timedOut {
      for client in snapshot where !client.hasExited {
        client.terminate()
      }
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
        version: (document["version"] as? NSNumber)?.int32Value ?? 1, text: DocumentText(text))
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
    guard let languageId = documents[uri]?.languageId else { return }
    let serving = servingKeys(languageId: languageId, fileUri: uri, recheck: recheck)
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
    guard !opening.isEmpty, let document = documents[uri] else { return }
    let params = Self.didOpenParams(uri: uri, document: document)
    for client in opening {
      client.notify(method: "textDocument/didOpen", params: params)
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
    let rootUri = projectRoot(forFileUri: fileUri, config: config, recheck: recheck)
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
        "uri": uri, "languageId": document.languageId, "version": document.version, "text": document.text.text,
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

  static func clientKey(serverId: String, rootUri: String) -> String {
    "\(serverId)@\(rootUri)"
  }

  /// The servers for a file that are up now; the others start in the
  /// background (see `availability`).
  private func runningClients(languageId: String, fileUri: String) -> [ServerProcess] {
    let serverIds = resolveServerIds(languageId: languageId, fileUri: fileUri)
    if serverIds.isEmpty || isAllowed?(fileUri) == false {
      return []
    }
    let rootUri = projectRoot(forFileUri: fileUri)
    return serverIds.compactMap { serverId in
      if case .running(let client) = availability(serverId: serverId, rootUri: rootUri, onStart: nil) {
        return client
      }
      return nil
    }
  }

  /// The servers for a file, once each has started (or failed to): `body`
  /// runs right away when none is starting, else (on a background thread)
  /// when the last start finishes. Nothing waits in the meantime.
  private func withClients(languageId: String, fileUri: String, _ body: @escaping ([ServerProcess]) -> Void) {
    let serverIds = resolveServerIds(languageId: languageId, fileUri: fileUri)
    guard !serverIds.isEmpty, isAllowed?(fileUri) != false else { return body([]) }
    let rootUri = projectRoot(forFileUri: fileUri)
    let lock = NSLock()
    var found = [ServerProcess?](repeating: nil, count: serverIds.count)
    let group = DispatchGroup()
    var waiting = false
    for (index, serverId) in serverIds.enumerated() {
      group.enter()
      let now = availability(serverId: serverId, rootUri: rootUri) { client in
        lock.withLock { found[index] = client }
        group.leave()
      }
      switch now {
      case .running(let client):
        lock.withLock { found[index] = client }
        group.leave()
      case .unavailable:
        group.leave()
      case .starting:
        waiting = true
      }
    }
    guard waiting else { return body(found.compactMap { $0 }) }
    group.notify(queue: Self.callbackQueue) {
      body(lock.withLock { found }.compactMap { $0 })
    }
  }

  private enum Availability {
    case running(ServerProcess)
    /// Shut down, or in its retry cooldown.
    case unavailable
    case starting
  }

  /// A server now: running, unavailable, or starting (started here if
  /// nobody has), in which case `onStart` gets it (or nil, when the start
  /// fails) once the start finishes.
  private func availability(
    serverId: String, rootUri: String, onStart: ((ServerProcess?) -> Void)?
  ) -> Availability {
    let key = Self.clientKey(serverId: serverId, rootUri: rootUri)
    stateLock.lock()
    if let client = clients[key] {
      stateLock.unlock()
      return .running(client)
    }
    if isShutDown || failedUntil[key].map({ Date() < $0 }) == true {
      stateLock.unlock()
      return .unavailable
    }
    if let onStart { startWaiters[key, default: []].append(onStart) }
    let starts = starting.insert(key).inserted
    if starts { failedUntil.removeValue(forKey: key) }
    stateLock.unlock()
    if starts {
      startQueue.async { [weak self] in
        self?.startServer(serverId: serverId, rootUri: rootUri, clientKey: key) { client in
          guard let self else { return }
          self.stateLock.lock()
          self.starting.remove(key)
          let waiters = self.startWaiters.removeValue(forKey: key) ?? []
          self.stateLock.unlock()
          for waiter in waiters { waiter(client) }
        }
      }
    }
    return .starting
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
        && projectRoot(forFileUri: uri) == rootUri
    }
    documentsLock.unlock()
    if serving {
      startQueue.asyncAfter(deadline: .now() + delay + 0.1) { [weak self] in
        _ = self?.availability(serverId: serverId, rootUri: rootUri, onStart: nil)
      }
    }
  }

  /// Port of `start_server`: resolve the command (PATH + managed bin dir),
  /// start the client and run its handshake, and on failure record the 15s
  /// cooldown and emit a serverError event. `completion` gets the client
  /// (registered, with the open documents sent to it), or nil.
  private func startServer(
    serverId: String, rootUri: String, clientKey: String, completion: @escaping (ServerProcess?) -> Void
  ) {
    // Server commands come only from the global config.
    guard let serverConfig = currentBaseConfig().config.servers[serverId] else {
      lspLog("No LSP server configured for server id: \(serverId)")
      completion(nil)
      return
    }

    guard let resolvedCommand = ManagedServers.resolveLspCommandPath(serverConfig.command) else {
      let message = ManagedServers.missingCommandMessage(
        serverId: serverId, command: serverConfig.command)
      lspLog("LSP startup skipped for \(serverId): \(message)")
      stateLock.lock()
      failedUntil[clientKey] = Date().addingTimeInterval(Self.startRetryCooldown)
      stateLock.unlock()
      enqueue(.serverError(clientKey: clientKey, serverId: serverId, message: message))
      completion(nil)
      return
    }

    let initOptions =
      serverConfig.initializationOptions
      ?? LSPConfig.defaultInitOptions(serverId: serverId)

    ServerProcess.start(
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
      }
    ) { [weak self] result in
      guard let self else {
        if case .success(let client) = result { client.shutdown() }
        completion(nil)
        return
      }
      switch result {
      case .success(let client):
        // Documents opened before it was up (or before it crashed) first,
        // then it takes notifications like the others: holding the documents
        // lock throughout means no change slips in between.
        self.documentsLock.lock()
        self.stateLock.lock()
        let shutDown = self.isShutDown
        self.stateLock.unlock()
        if shutDown {
          self.documentsLock.unlock()
          client.shutdown()
          completion(nil)
          return
        }
        // A new process: nothing is open in it yet.
        self.openIn = self.openIn.mapValues { $0.subtracting([clientKey]) }
        for (uri, document) in self.documents
        where self.isAllowed?(uri) != false
          && self.resolveServerIds(languageId: document.languageId, fileUri: uri).contains(serverId)
          && self.projectRoot(forFileUri: uri) == rootUri
        {
          self.openIn[uri, default: []].insert(clientKey)
          self.reattach(uri, to: [clientKey])
          client.notify(method: "textDocument/didOpen", params: Self.didOpenParams(uri: uri, document: document))
        }
        self.stateLock.lock()
        self.clients[clientKey] = client
        self.stateLock.unlock()
        self.documentsLock.unlock()
        completion(client)
      case .failure(let error):
        lspLog("Failed to start LSP server for '\(serverId)' (key='\(clientKey)'): \(error)")
        self.stateLock.lock()
        self.failedUntil[clientKey] = Date().addingTimeInterval(Self.startRetryCooldown)
        self.stateLock.unlock()
        self.enqueue(.serverError(clientKey: clientKey, serverId: serverId, message: error))
        completion(nil)
      }
    }
  }
}
