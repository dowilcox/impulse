import AppKit
import ImpulseGit
import ImpulseKit
import Observation

/// Live state of one repository, shared by every window and view that shows
/// it (titlebar breadcrumb, status bar, Changes panel, review, file tree).
/// Refreshes itself from a RepoWatcher; git actions run off the main thread
/// and refresh it afterwards.
@Observable
final class GitRepositoryState {
  let root: String
  private(set) var snapshot: RepoSnapshot?
  private(set) var isRefreshing = false
  /// Long-running remote operation in progress ("Pushing…") with its latest
  /// progress line.
  private(set) var activity: String?
  private(set) var activityDetail: String?
  /// Incremented on every snapshot change (cheap change token for observers).
  private(set) var revision = 0
  /// When this repository last fetched (any fetch or pull from Impulse).
  @ObservationIgnored var lastFetch: Date?
  @ObservationIgnored private var refreshCallbacks: [(RepoSnapshot?) -> Void] = []
  @ObservationIgnored private var watcher: RepoWatcher?
  @ObservationIgnored private var refreshQueued = false
  @ObservationIgnored private var refreshInFlight = false
  @ObservationIgnored private let queue: DispatchQueue
  /// Background fetches: a slow or unreachable remote mustn't hold up
  /// status refreshes or the user's own git actions on `queue`.
  @ObservationIgnored private let fetchQueue: DispatchQueue
  @ObservationIgnored private var quietFetchInFlight = false
  /// Listeners that want to know about working-tree changes even when the
  /// snapshot is identical (e.g. an open review re-diffing a modified file).
  @ObservationIgnored private var changeListeners: [UUID: (RepoWatcher.Change) -> Void] = [:]

  init(root: String) {
    self.root = root
    queue = DispatchQueue(label: "impulse.git.\(root.hashValue)", qos: .userInitiated)
    fetchQueue = DispatchQueue(label: "impulse.git-fetch.\(root.hashValue)", qos: .utility)
  }

  deinit {
    watcher?.stop()
  }

  // MARK: - Refresh

  func start() {
    refresh()
  }

  /// Re-read the snapshot (coalesces bursts; never blocks the main thread).
  func refresh() {
    if refreshInFlight {
      refreshQueued = true
      return
    }
    refreshInFlight = true
    isRefreshing = true
    let root = self.root
    queue.async { [weak self] in
      let snapshot = GitClient.snapshot(forPath: root)
      // A merge, cherry-pick or revert just started (an agent's, say): note
      // the work from before it, so Abort can put exactly that back.
      if let operation = snapshot?.operation, OperationAbort.applies(to: operation) {
        OperationAbort.recordStartIfNeeded(root: root)
      }
      DispatchQueue.main.async {
        guard let self else { return }
        self.refreshInFlight = false
        self.isRefreshing = false
        if snapshot != self.snapshot {
          self.snapshot = snapshot
          self.revision += 1
        }
        self.startWatchingIfNeeded()
        if self.refreshQueued {
          self.refreshQueued = false
          self.refresh()
        } else if !self.refreshCallbacks.isEmpty {
          let callbacks = self.refreshCallbacks
          self.refreshCallbacks = []
          for callback in callbacks { callback(self.snapshot) }
        }
      }
    }
  }

  /// Run `callback` with the snapshot once the refresh in progress (or the
  /// next one) finishes.
  func afterNextRefresh(_ callback: @escaping (RepoSnapshot?) -> Void) {
    refreshCallbacks.append(callback)
    if !refreshInFlight { refresh() }
  }

  /// Fetch without telling anyone (background fetch): no activity bar, no
  /// errors shown, no credential prompts. Skipped while another operation
  /// runs or when there's no remote.
  func fetchQuietly(timeout: TimeInterval = 120) {
    guard activity == nil, !quietFetchInFlight, snapshot?.upstream != nil else { return }
    quietFetchInFlight = true
    lastFetch = Date()
    let root = self.root
    fetchQueue.async { [weak self] in
      let result = GitOperations.fetch(root: root, timeout: timeout)
      DispatchQueue.main.async {
        guard let self else { return }
        self.quietFetchInFlight = false
        switch result {
        case .success:
          self.refresh()
          NotificationCenter.default.post(
            name: .gitRepositoryDidChange, object: self, userInfo: ["root": root])
        case .failure(let error):
          NSLog("Background fetch of %@ failed: %@", root, error.message)
        }
      }
    }
  }

  private func startWatchingIfNeeded() {
    guard watcher == nil, let snapshot else { return }
    let watcher = RepoWatcher(
      root: snapshot.root, gitDir: snapshot.gitDir, commonDir: snapshot.commonDir
    ) { [weak self] change in
      guard let self else { return }
      self.refresh()
      for listener in self.changeListeners.values { listener(change) }
    }
    watcher.start()
    self.watcher = watcher
  }

  /// Observe raw change events (main queue). Returns a token for removal.
  @discardableResult
  func addChangeListener(_ listener: @escaping (RepoWatcher.Change) -> Void) -> UUID {
    let id = UUID()
    changeListeners[id] = listener
    return id
  }

  func removeChangeListener(_ id: UUID) {
    changeListeners[id] = nil
  }

  // MARK: - Running operations

  /// Run a git operation off the main thread, optionally recording a safety
  /// snapshot first, then refresh. `completion` gets the result on main.
  ///
  /// `requireSnapshot`: the operation throws work away, so it doesn't run at
  /// all when the snapshot can't be made (an unreadable file, an LFS filter
  /// that isn't installed…) — the promised Undo would be missing.
  func run(
    _ label: String? = nil, snapshotReason: String? = nil, requireSnapshot: Bool = false,
    _ operation: @escaping (String) -> GitResult,
    completion: ((GitResult, SafetySnapshot?) -> Void)? = nil
  ) {
    if let label { activity = label }
    let root = self.root
    queue.async { [weak self] in
      var safety: SafetySnapshot?
      var snapshotFailure: GitOperationError?
      if let snapshotReason {
        switch SafetySnapshots.create(reason: snapshotReason, root: root) {
        case .success(let created): safety = created
        case .failure(let error): snapshotFailure = error
        }
      }
      let result: GitResult
      if requireSnapshot, let snapshotFailure {
        result = .failure(
          .invalid(
            "Nothing was changed: Impulse couldn't save a safety snapshot to undo it with. \(snapshotFailure.message)"))
      } else {
        result = operation(root)
      }
      DispatchQueue.main.async {
        guard let self else { return }
        if label != nil {
          self.activity = nil
          self.activityDetail = nil
        }
        self.refresh()
        NotificationCenter.default.post(
          name: .gitRepositoryDidChange, object: self, userInfo: ["root": root])
        completion?(result, safety)
      }
      // After the operation, not before it: older than two weeks, or beyond
      // the newest 200, go. Still on the queue, so it never races the next
      // operation for the refs.
      if safety != nil { SafetySnapshots.prune(root: root) }
    }
  }

  /// Progress line for the current activity (called from git's stderr).
  func reportProgress(_ line: String) {
    DispatchQueue.main.async { [weak self] in
      self?.activityDetail = line
    }
  }
}

// MARK: - Store

/// One `GitRepositoryState` per repository root, shared app-wide.
final class GitRepositoryStore {
  static let shared = GitRepositoryStore()

  private var states: [String: GitRepositoryState] = [:]
  /// Directory → repository root (nil = not a repository), resolved off main.
  private var rootCache: [String: String?] = [:]

  /// The state for a repository root, created (and started) on first use.
  func state(forRoot root: String) -> GitRepositoryState {
    if let existing = states[root] { return existing }
    let state = GitRepositoryState(root: root)
    states[root] = state
    state.start()
    return state
  }

  /// Resolve the repository containing `directory` off the main thread and
  /// hand back its state (nil outside a repository) on main.
  func resolve(directory: String, completion: @escaping (GitRepositoryState?) -> Void) {
    if let cached = rootCache[directory] {
      completion(cached.map { state(forRoot: $0) })
      return
    }
    DispatchQueue.global(qos: .userInitiated).async {
      let root = GitClient.repoRoot(forPath: directory)
      DispatchQueue.main.async {
        if self.rootCache.count > 256 { self.rootCache.removeAll() }
        self.rootCache[directory] = .some(root)
        completion(root.map { self.state(forRoot: $0) })
      }
    }
  }

  /// Refresh every known repository (e.g. when the app becomes active).
  func refreshAll() {
    for state in states.values { state.refresh() }
  }
}

extension Notification.Name {
  /// Posted after Impulse itself changed a repository (object: state).
  static let gitRepositoryDidChange = Notification.Name("impulse.gitRepositoryDidChange")
}
