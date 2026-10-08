import AppKit
import ImpulseGit
import ImpulseKit
import Observation
import SwiftUI
import os.log

/// Observable header state for the review surface.
@Observable
final class ReviewSurfaceModel {
  var scope: DiffScope
  var options = ReviewOptions()
  var fileCount = 0
  var viewedCount = 0
  /// Comments on the files this scope shows (what Send and Copy take).
  var commentCount = 0
  /// Every comment in the repository (what Delete All removes).
  var repositoryCommentCount = 0
  var totalAdded = 0
  var totalRemoved = 0
  var isLoading = false
  /// Branch to compare against for "vs base" (auto-detected).
  var baseBranch: String?
  /// The most recent agent turn in this repository, if any.
  var agentTurn: (name: String, scope: DiffScope)?
  /// The last time the user finished reviewing (sent comments, or marked
  /// every file viewed): "Since my last review" diffs from there.
  var lastReview: SafetySnapshot?
  var palette: ChromePalette

  @ObservationIgnored var onSelectScope: ((DiffScope) -> Void)?
  @ObservationIgnored var onSetLayout: ((String) -> Void)?
  @ObservationIgnored var onToggleWhitespace: (() -> Void)?
  @ObservationIgnored var onSetContextLines: ((Int) -> Void)?
  /// `review_context_lines`: what the context menu offers besides its
  /// fixed choices.
  var defaultContextLines = 3
  @ObservationIgnored var onCopyPrompt: (() -> Void)?
  @ObservationIgnored var onListAgents: (() -> [AgentSummary])?
  @ObservationIgnored var onSendToAgent: ((UUID) -> Void)?
  @ObservationIgnored var onClearComments: (() -> Void)?
  @ObservationIgnored var onRefresh: (() -> Void)?
  @ObservationIgnored var onShowChanges: (() -> Void)?

  init(scope: DiffScope, palette: ChromePalette) {
    self.scope = scope
    self.palette = palette
  }
}

/// Multi-file change review, all native: a header (scope, layout,
/// whitespace, progress, comments), a file navigator, and a diff list with
/// syntax-colored hunks, hunk/line staging and reverting, viewed marks and
/// inline comments. Follows the repository live.
final class ReviewSurface: NSView, ReviewDiffHandler {
  let repository: GitRepositoryState
  var repoRoot: String { repository.root }
  private weak var host: GitPanelHost?

  private let model: ReviewSurfaceModel
  private let navigator: ReviewNavigatorModel
  private let diffContext: ReviewDiffContext
  private let diffList: ReviewDiffController
  private var emptyHost: NSView!

  /// What the review is showing (saved with the session).
  var scope: DiffScope { model.scope }
  private var theme: Theme
  private var pendingFocus: String?
  private var generation = 0
  /// Files in display order, and by path.
  private var files: [ReviewFile] = []
  private var filesByPath: [String: ReviewFile] = [:]
  private var emptyMessage = ""
  private var changeListener: UUID?
  private var checkpointObserver: NSObjectProtocol?
  private var refreshWork: DispatchWorkItem?
  private var rowsScheduled = false
  private let comments: ReviewCommentStore
  private let queue = DispatchQueue(label: "impulse.review", qos: .userInitiated)

  private static let log = OSLog(subsystem: "dev.impulse.Impulse", category: "Review")
  /// Stored for a file marked viewed before its diff loaded.
  private static let unknownHash = "unknown"

  init(
    repository: GitRepositoryState, scope: DiffScope?, focusPath: String?, theme: Theme,
    host: GitPanelHost?
  ) {
    self.repository = repository
    self.theme = theme
    self.host = host
    self.comments = ReviewCommentStore.forRepository(repository.root)
    let palette = ChromePalette(theme: theme)
    self.model = ReviewSurfaceModel(scope: scope ?? Self.defaultScope(repository.snapshot), palette: palette)
    self.navigator = ReviewNavigatorModel(palette: palette)
    let settings = SettingsStore.shared.settings
    self.diffContext = ReviewDiffContext(theme: theme, metrics: ReviewMetrics(settings: settings))
    self.diffList = ReviewDiffController(context: diffContext)
    self.pendingFocus = focusPath
    super.init(frame: NSRect(x: 0, y: 0, width: 800, height: 600))
    model.options.contextLines = settings.reviewContextLines
    model.defaultContextLines = settings.reviewContextLines
    wantsLayer = true
    layer?.backgroundColor = NSColor(hex: theme.bg).cgColor
    diffContext.handler = self
    diffContext.capabilities = capabilities
    setupViews()
    wireModel()
    refreshFiles()
    changeListener = repository.addChangeListener { [weak self] _ in
      self?.scheduleRefresh()
    }
    let root = repository.root
    queue.async { [weak self] in
      let base = GitClient.defaultBaseBranch(repoPath: root)
      DispatchQueue.main.async { self?.model.baseBranch = base }
    }
    updateAgentTurn()
    queue.async { [weak self] in
      let last = SafetySnapshots.list(root: root, prefix: SafetySnapshots.reviewPrefix).first
      DispatchQueue.main.async { self?.model.lastReview = last }
    }
    checkpointObserver = NotificationCenter.default.addObserver(
      forName: .agentCheckpointsChanged, object: nil, queue: .main
    ) { [weak self] _ in
      self?.updateAgentTurn()
    }
    settingsObserver = NotificationCenter.default.addObserver(
      forName: .impulseSettingsDidChange, object: nil, queue: .main
    ) { [weak self] _ in
      self?.applySettings()
    }
  }

  private var settingsObserver: NSObjectProtocol?
  /// The context menu picked this review's context; the setting no longer
  /// moves it.
  private var contextPicked = false

  /// The code font follows the editor's (family and size); the context
  /// follows `review_context_lines` until the header picks one.
  private func applySettings() {
    let settings = SettingsStore.shared.settings
    let metrics = ReviewMetrics(settings: settings)
    if metrics.codeFont != diffContext.metrics.codeFont {
      diffContext.metrics = metrics
      scheduleRows()
    }
    model.defaultContextLines = settings.reviewContextLines
    if !contextPicked { setContextLines(settings.reviewContextLines) }
  }

  /// Show `lines` of unchanged context around each change: every diff is
  /// read again (hunks merge or split, so selections and focus go).
  private func setContextLines(_ lines: Int) {
    guard lines != model.options.contextLines else { return }
    model.options.contextLines = lines
    diffContext.focus = nil
    for file in files {
      file.diff = nil
      file.syntax = nil
      file.selection = nil
      file.stale = true
    }
    scheduleRows()
    refreshFiles()
  }

  private func updateAgentTurn() {
    let turn = AgentCheckpoints.shared.lastTurn(inRepo: repository.root)
    model.agentTurn = turn.map { ($0.agentName, $0.scope) }
    // Following the latest turn: when it finishes, show the finished diff.
    if case .snapshot(let from, _) = model.scope, let turn, turn.start.ref == from,
      model.scope != turn.scope
    {
      setScope(turn.scope)
    }
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

  /// Unstaged when there is unstaged work (stage/revert from there), else
  /// staged, else everything uncommitted.
  static func defaultScope(_ snapshot: RepoSnapshot?) -> DiffScope {
    guard let snapshot else { return .uncommitted }
    if !snapshot.unstaged.isEmpty || !snapshot.untracked.isEmpty { return .unstaged }
    if !snapshot.staged.isEmpty { return .staged }
    return .uncommitted
  }

  // MARK: - Setup

  private static let navigatorWidth: CGFloat = 250

  private func setupViews() {
    let header = WorkbenchHosting.make(ReviewHeaderBar(model: model), intrinsicHeight: true)
    let nav = WorkbenchHosting.make(ReviewNavigatorView(model: navigator))
    let divider = NSBox()
    divider.boxType = .custom
    divider.borderWidth = 0
    divider.fillColor = ChromePalette(theme: theme).nsHairline
    navigatorDivider = divider
    let list = diffList.scrollView
    let empty = WorkbenchHosting.make(ReviewEmptyView(message: ""))
    emptyHost = empty
    empty.isHidden = true
    for view in [header, nav, divider, list, empty] as [NSView] {
      view.translatesAutoresizingMaskIntoConstraints = false
      addSubview(view)
    }
    let navWidth = nav.widthAnchor.constraint(equalToConstant: Self.navigatorWidth)
    navigatorWidth = navWidth
    NSLayoutConstraint.activate([
      header.topAnchor.constraint(equalTo: topAnchor),
      header.leadingAnchor.constraint(equalTo: leadingAnchor),
      header.trailingAnchor.constraint(equalTo: trailingAnchor),
      nav.topAnchor.constraint(equalTo: header.bottomAnchor),
      nav.bottomAnchor.constraint(equalTo: bottomAnchor),
      nav.leadingAnchor.constraint(equalTo: leadingAnchor),
      navWidth,
      divider.topAnchor.constraint(equalTo: header.bottomAnchor),
      divider.bottomAnchor.constraint(equalTo: bottomAnchor),
      divider.leadingAnchor.constraint(equalTo: nav.trailingAnchor),
      divider.widthAnchor.constraint(equalToConstant: 1),
      list.topAnchor.constraint(equalTo: header.bottomAnchor),
      list.bottomAnchor.constraint(equalTo: bottomAnchor),
      list.leadingAnchor.constraint(equalTo: divider.trailingAnchor),
      list.trailingAnchor.constraint(equalTo: trailingAnchor),
      empty.topAnchor.constraint(equalTo: list.topAnchor),
      empty.bottomAnchor.constraint(equalTo: list.bottomAnchor),
      empty.leadingAnchor.constraint(equalTo: leadingAnchor),
      empty.trailingAnchor.constraint(equalTo: list.trailingAnchor),
    ])
  }

  private var navigatorDivider: NSBox?
  private var navigatorWidth: NSLayoutConstraint?

  override func layout() {
    super.layout()
    // Narrow (a split pane, History's lower half): give the diff the room.
    let narrow = bounds.width < 620
    navigatorWidth?.constant = narrow ? 0 : Self.navigatorWidth
    navigatorDivider?.isHidden = narrow
  }

  private func wireModel() {
    model.onSelectScope = { [weak self] scope in self?.setScope(scope) }
    model.onSetLayout = { [weak self] layout in self?.applyLayout(layout) }
    model.onToggleWhitespace = { [weak self] in
      guard let self else { return }
      self.model.options.ignoreWhitespace.toggle()
      for file in self.files {
        file.diff = nil
        file.syntax = nil
        file.stale = true
      }
      self.scheduleRows()
      self.refreshFiles()
    }
    model.onSetContextLines = { [weak self] lines in
      self?.contextPicked = true
      self?.setContextLines(lines)
    }
    model.onCopyPrompt = { [weak self] in self?.copyCommentsAsPrompt() }
    model.onListAgents = { [weak self] in self?.host?.agentTargets ?? [] }
    model.onSendToAgent = { [weak self] id in self?.sendCommentsToAgent(id) }
    model.onClearComments = { [weak self] in self?.clearComments() }
    model.onRefresh = { [weak self] in self?.refresh() }
    model.onShowChanges = { [weak self] in
      (self?.host as? MainWindowController)?.showChangesPanel()
    }
    navigator.onSelect = { [weak self] path in self?.revealFile(path) }
    navigator.onToggleViewed = { [weak self] path in
      guard let self, let file = self.filesByPath[path] else { return }
      self.reviewSetViewed(path, viewed: !file.viewed)
    }
  }

  func cleanup() {
    if let changeListener { repository.removeChangeListener(changeListener) }
    changeListener = nil
    if let checkpointObserver { NotificationCenter.default.removeObserver(checkpointObserver) }
    checkpointObserver = nil
    if let settingsObserver { NotificationCenter.default.removeObserver(settingsObserver) }
    settingsObserver = nil
    refreshWork?.cancel()
  }

  func focus() {
    window?.makeFirstResponder(diffList.tableView)
  }

  /// Switch between "unified" and "split" rows.
  func setLayout(_ layout: String) {
    model.onSetLayout?(layout)
  }

  private func applyLayout(_ layout: String) {
    model.options.layout = layout
    diffContext.layout = ReviewLayout(rawValue: layout) ?? .unified
    scheduleRows()
  }

  /// Re-read the file list (and loaded diffs) now.
  func refresh() {
    refreshFiles()
  }

  /// Switch scope (and optionally scroll to a file), e.g. from the Changes panel.
  func show(scope: DiffScope?, focusPath: String?) {
    if let scope, scope != model.scope {
      pendingFocus = focusPath
      setScope(scope)
    } else if let focusPath {
      revealFile(focusPath)
    }
  }

  func applyTheme(_ theme: Theme) {
    self.theme = theme
    let palette = ChromePalette(theme: theme)
    model.palette = palette
    navigator.palette = palette
    navigatorDivider?.fillColor = palette.nsHairline
    layer?.backgroundColor = NSColor(hex: theme.bg).cgColor
    diffContext.colors = ReviewColors(theme: theme)
    diffList.applyColors()
    scheduleRows()
  }

  // MARK: - Scope

  private var scopeKey: String {
    switch model.scope {
    case .unstaged: return "unstaged"
    case .staged: return "staged"
    case .uncommitted: return "uncommitted"
    case .branch(let base): return "branch:\(base)"
    case .commit(let sha): return "commit:\(sha)"
    case .range(let a, let b): return "range:\(a)..\(b)"
    case .stash(let i): return "stash:\(i)"
    case .snapshot(let a, let b): return "snapshot:\(a)..\(b ?? "wt")"
    }
  }

  private func setScope(_ scope: DiffScope) {
    model.scope = scope
    files = []
    filesByPath = [:]
    diffContext.files = [:]
    diffContext.focus = nil
    diffContext.capabilities = capabilities
    scheduleRows()
    refreshFiles()
  }

  private var capabilities: ReviewCapabilities {
    switch model.scope {
    case .unstaged: return ReviewCapabilities(stage: true, unstage: false, revert: true)
    case .staged: return ReviewCapabilities(stage: false, unstage: true, revert: false)
    default: return ReviewCapabilities(stage: false, unstage: false, revert: false)
    }
  }

  // MARK: - Data

  private var diffOptions: DiffOptions {
    DiffOptions(
      contextLines: model.options.contextLines, ignoreWhitespace: model.options.ignoreWhitespace)
  }

  private func scheduleRefresh() {
    refreshWork?.cancel()
    let work = DispatchWorkItem { [weak self] in self?.refreshFiles() }
    refreshWork = work
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.3, execute: work)
  }

  private func refreshFiles() {
    generation += 1
    let generation = self.generation
    let root = repoRoot
    let scope = model.scope
    let options = diffOptions
    let viewed = ReviewViewedStore.viewed(root: root, scope: scopeKey)
    model.isLoading = true
    queue.async { [weak self] in
      let result = Result { try GitClient.changedFiles(repoPath: root, scope: scope) }
      // Read viewed files' diffs now, so ones that changed come back unviewed.
      var viewedDiffs: [String: FileDiff] = [:]
      if case .success(let changes) = result {
        for change in changes where viewed[change.path] != nil {
          if let diff = try? GitClient.fileDiff(
            repoPath: root, path: change.path, oldPath: change.oldPath, scope: scope, options: options)
          {
            viewedDiffs[change.path] = diff
          }
        }
      }
      DispatchQueue.main.async {
        guard let self, generation == self.generation else { return }
        self.model.isLoading = false
        switch result {
        case .success(let changes):
          self.merge(changes, viewedDiffs: viewedDiffs, viewed: viewed)
        case .failure(let error):
          self.files = []
          self.filesByPath = [:]
          self.diffContext.files = [:]
          self.emptyMessage = "Couldn't read changes: \(error)"
          self.scheduleRows()
        }
      }
    }
  }

  /// Bring the file list in line with `changes`, keeping each file's view
  /// state and showing its previous diff until the new one arrives.
  private func merge(_ changes: [FileChange], viewedDiffs: [String: FileDiff], viewed: [String: String]) {
    emptyMessage = Self.emptyMessage(for: model.scope)
    var next: [ReviewFile] = []
    var byPath: [String: ReviewFile] = [:]
    for change in changes {
      let file = filesByPath[change.path] ?? ReviewFile(change: change)
      let isNew = filesByPath[change.path] == nil
      file.change = change
      file.stale = true
      if let diff = viewedDiffs[change.path] { apply(diff, to: file) }
      if let hash = viewed[change.path] {
        let current = file.diff?.contentHash
        // Marked viewed before its diff had loaded: what's here now is
        // what was viewed, so adopt its hash rather than calling it changed.
        if hash == Self.unknownHash, let current {
          ReviewViewedStore.set(path: change.path, hash: current, root: repoRoot, scope: scopeKey)
        }
        let changed = hash != Self.unknownHash && current != nil && current != hash
        file.viewed = !changed
        file.changedSinceViewed = changed
      } else {
        file.viewed = false
      }
      if isNew { file.expanded = !file.viewed }
      file.comments = comments.comments(for: change.path)
      updateOutdated(file)
      next.append(file)
      byPath[change.path] = file
    }
    files = next
    filesByPath = byPath
    diffContext.files = byPath
    // Diffs already on screen are read again; the rest load when shown.
    for file in files where file.diff != nil && file.expanded {
      loadDiff(file.path)
    }
    scheduleRows(background: true)
    if let focus = pendingFocus, byPath[focus] != nil {
      pendingFocus = nil
      DispatchQueue.main.async { [weak self] in self?.revealFile(focus) }
    }
  }

  private static func emptyMessage(for scope: DiffScope) -> String {
    switch scope {
    case .unstaged: return "No unstaged changes."
    case .staged: return "Nothing is staged."
    case .uncommitted: return "No uncommitted changes."
    case .branch(let base): return "No changes compared to \(base)."
    default: return "No changes."
    }
  }

  private func loadDiff(_ path: String) {
    guard let file = filesByPath[path], !file.loading else { return }
    file.loading = true
    let change = file.change
    let root = repoRoot
    let scope = model.scope
    let options = diffOptions
    let generation = self.generation
    queue.async { [weak self] in
      let result = Result {
        try GitClient.fileDiff(
          repoPath: root, path: change.path, oldPath: change.oldPath, scope: scope, options: options)
      }
      DispatchQueue.main.async {
        guard let self, let file = self.filesByPath[path] else { return }
        file.loading = false
        guard generation == self.generation || file.diff == nil else {
          // A newer list arrived meanwhile: read it again for that one.
          self.loadDiff(path)
          return
        }
        switch result {
        case .success(let diff):
          self.apply(diff, to: file)
          // A viewed file whose diff changed is unviewed again.
          let viewed = ReviewViewedStore.viewed(root: root, scope: self.scopeKey)
          if viewed[path] == Self.unknownHash {
            ReviewViewedStore.set(path: path, hash: diff.contentHash, root: root, scope: self.scopeKey)
          } else if let hash = viewed[path], hash != diff.contentHash, file.viewed {
            file.viewed = false
            file.changedSinceViewed = true
            file.expanded = true
          }
        case .failure(let error):
          file.error = "\(error)"
          file.stale = false
        }
        self.scheduleRows(background: true)
      }
    }
  }

  private func apply(_ diff: FileDiff, to file: ReviewFile) {
    let previous = file.diff?.contentHash
    file.diff = diff
    file.error = nil
    file.stale = false
    if previous != diff.contentHash {
      file.selection = nil
      file.syntax = nil
      highlight(file)
    } else if file.syntax == nil {
      highlight(file)
    }
    updateOutdated(file)
  }

  /// Color the file's lines in the background, old and new sides apart so
  /// strings and comments don't run across them.
  private func highlight(_ file: ReviewFile) {
    guard let diff = file.diff, !diff.isBinary, !diff.tooLarge,
      let language = SyntaxHighlighter.hljsLanguage(diff.language, path: diff.path)
    else { return }
    let hash = diff.contentHash
    let path = file.path
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      var oldLines: [String] = []
      var newLines: [String] = []
      var positions: [[(old: Int?, new: Int?)]] = []
      for hunk in diff.hunks {
        positions.append(
          hunk.lines.map { line in
            var old: Int?
            var new: Int?
            if line.kind != .added {
              old = oldLines.count
              oldLines.append(line.content)
            }
            if line.kind != .removed {
              new = newLines.count
              newLines.append(line.content)
            }
            return (old, new)
          })
      }
      let highlighter = SyntaxHighlighter.shared
      guard let oldSpans = highlighter.highlight(lines: oldLines, language: language),
        let newSpans = highlighter.highlight(lines: newLines, language: language)
      else { return }
      let syntax: [[[SyntaxHighlighter.Span]]] = zip(diff.hunks, positions).map { hunk, places in
        zip(hunk.lines, places).map { line, place in
          if line.kind == .removed { return place.old.flatMap { oldSpans[safe: $0] } ?? [] }
          return place.new.flatMap { newSpans[safe: $0] } ?? []
        }
      }
      DispatchQueue.main.async {
        guard let self, let file = self.filesByPath[path], file.diff?.contentHash == hash else { return }
        file.syntax = syntax
        self.diffList.redrawVisibleRows()
      }
    }
  }

  private func updateOutdated(_ file: ReviewFile) {
    guard let diff = file.diff else {
      file.outdated = []
      return
    }
    var oldLines: [Int: String] = [:]
    var newLines: [Int: String] = [:]
    for hunk in diff.hunks {
      for line in hunk.lines {
        if let old = line.oldLineno, line.kind != .added { oldLines[Int(old)] = line.content }
        if let new = line.newLineno, line.kind != .removed { newLines[Int(new)] = line.content }
      }
    }
    file.outdated = Set(
      file.comments.filter {
        ReviewCommentAnchoring.isOutdated($0, lines: $0.side == .old ? oldLines : newLines)
      }.map(\.id))
  }

  private func reloadComments(_ paths: Set<String>? = nil) {
    for file in files where paths?.contains(file.path) ?? true {
      file.comments = comments.comments(for: file.path)
      updateOutdated(file)
    }
    scheduleRows()
  }

  // MARK: - Rows

  /// A comment is being written or edited.
  private var isComposing: Bool {
    files.contains { $0.composer != nil || $0.editingComment != nil }
  }
  /// A background refresh waited for the comment to be finished.
  private var deferredRows = false

  /// Rebuild the list on the next turn of the run loop (changes batch up).
  /// `background` refreshes (files changing on disk, diffs arriving) wait
  /// while a comment is being typed, so its row isn't rebuilt under it.
  private func scheduleRows(background: Bool = false) {
    if background && isComposing {
      deferredRows = true
      return
    }
    deferredRows = false
    guard !rowsScheduled else { return }
    rowsScheduled = true
    DispatchQueue.main.async { [weak self] in
      guard let self else { return }
      self.rowsScheduled = false
      self.rebuildRows()
    }
  }

  private func rebuildRows() {
    diffList.reload(ReviewRowBuilder.rows(files, layout: diffContext.layout))
    navigator.items = files.map { file in
      ReviewNavigatorModel.Item(
        path: file.path, status: file.change.status, viewed: file.viewed,
        changedSinceViewed: file.changedSinceViewed, commentCount: file.comments.count,
        added: file.diff?.added ?? file.change.added, removed: file.diff?.removed ?? file.change.removed,
        binary: file.diff?.isBinary ?? file.change.isBinary)
    }
    model.fileCount = files.count
    model.viewedCount = files.filter(\.viewed).count
    model.commentCount = scopedComments.count
    model.repositoryCommentCount = comments.comments.count
    model.totalAdded = files.compactMap { $0.change.added }.reduce(0, +)
    model.totalRemoved = files.compactMap { $0.change.removed }.reduce(0, +)
    let showEmpty = files.isEmpty && !model.isLoading && !emptyMessage.isEmpty
    emptyHost.isHidden = !showEmpty
    if showEmpty, let hosting = emptyHost as? NSHostingView<ReviewEmptyView> {
      hosting.rootView = ReviewEmptyView(message: emptyMessage, palette: model.palette)
    }
  }

  /// Scroll to a file (expanding it) and make it the keyboard focus.
  private func revealFile(_ path: String) {
    guard let file = filesByPath[path] else {
      pendingFocus = path
      return
    }
    if !file.expanded {
      file.expanded = true
      rebuildRows()
    }
    diffContext.focus = (path, 0)
    diffList.reveal(path: path)
    navigator.current = path
    focus()
  }

  // MARK: - ReviewDiffHandler

  func reviewToggleExpanded(_ path: String) {
    guard let file = filesByPath[path] else { return }
    file.expanded.toggle()
    scheduleRows()
  }

  func reviewSetViewed(_ path: String, viewed: Bool) {
    guard let file = filesByPath[path] else { return }
    let hash = viewed ? (file.diff?.contentHash ?? Self.unknownHash) : nil
    ReviewViewedStore.set(path: path, hash: hash, root: repoRoot, scope: scopeKey)
    let wasComplete = !files.isEmpty && files.allSatisfy(\.viewed)
    file.viewed = viewed
    file.changedSinceViewed = false
    // Viewing collapses; un-viewing expands.
    file.expanded = !viewed
    scheduleRows()
    if viewed, !wasComplete, files.allSatisfy(\.viewed) {
      markReviewed()
    }
  }

  func reviewFileAction(_ action: ReviewAction, path: String) {
    guard let file = filesByPath[path], let host else { return }
    let actions = GitActions(repository: repository, host: host)
    switch action {
    case .stage: actions.stage([file.change])
    case .unstage: actions.unstage([file.change])
    case .revert: actions.discard([file.change])
    }
  }

  func reviewHunkAction(_ action: ReviewAction, path: String, hunk: Int) {
    guard let file = filesByPath[path], !file.busy, let diff = file.diff, diff.hunks.indices.contains(hunk),
      let host
    else { return }
    switch action {
    case .stage: guard capabilities.stage else { return }
    case .unstage: guard capabilities.unstage else { return }
    case .revert: guard capabilities.revert else { return }
    }
    if diff.truncated {
      host.toasts.show(Toast(kind: .info, message: GitOperations.truncatedDiffMessage))
      return
    }
    let lines = file.selection.flatMap { $0.hunk == hunk ? $0.lines.sorted() : nil }
    file.selection = nil
    let target: PatchTarget = action == .stage ? .stage : action == .unstage ? .unstage : .discard
    let selection: PatchSelection = lines.map { .lines(Set($0), inHunk: hunk) } ?? .wholeHunks([hunk])
    file.busy = true
    diffList.redrawVisibleRows()
    GitActions(repository: repository, host: host).apply(
      target, selection: selection, change: file.change, expectedHunkIds: [hunk: diff.hunkIds[hunk]],
      options: diffOptions
    ) { [weak self] _ in
      file.busy = false
      self?.refreshFiles()
    }
  }

  func reviewToggleLine(path: String, hunk: Int, line: Int, extend: Bool) {
    guard let file = filesByPath[path], let lines = file.diff?.hunks[safe: hunk]?.lines else { return }
    var selection =
      file.selection.flatMap { $0.hunk == hunk ? $0 : nil }
      ?? ReviewFile.LineSelection(hunk: hunk, lines: [], anchor: line)
    if extend {
      for index in min(selection.anchor, line)...max(selection.anchor, line) where lines[index].kind != .context {
        selection.lines.insert(index)
      }
    } else {
      if selection.lines.contains(line) { selection.lines.remove(line) } else { selection.lines.insert(line) }
      selection.anchor = line
    }
    file.selection = selection.lines.isEmpty ? nil : selection
    diffContext.focus = (path, hunk)
    diffList.redrawVisibleRows()
  }

  func reviewOpenComposer(path: String, hunk: Int, line lineIndex: Int?) {
    guard let file = filesByPath[path], let lines = file.diff?.hunks[safe: hunk]?.lines, !lines.isEmpty else {
      return
    }
    let selected = file.selection.flatMap { $0.hunk == hunk ? $0.lines : nil } ?? []
    // For the hunk: the selection's last line, else its last changed line.
    let target =
      lineIndex
      ?? selected.max()
      ?? lines.lastIndex { $0.kind != .context }
      ?? lines.count - 1
    // A selection including the line widens the comment to the range.
    let indices = selected.contains(target) ? selected.sorted() : [target]
    let anchor = lines[target]
    let side: ReviewComment.Side = anchor.kind == .removed ? .old : .new
    let sideLines = indices.map { lines[$0] }.filter { side == .old ? $0.kind == .removed : $0.kind != .removed }
    let numbers = sideLines.compactMap { Int((side == .old ? $0.oldLineno : $0.newLineno) ?? 0) }.filter { $0 > 0 }
    let first = numbers.min() ?? Int((side == .old ? anchor.oldLineno : anchor.newLineno) ?? 1)
    let last = numbers.max() ?? first
    // Every line of the range on that side, context included: the comment
    // is outdated when line `first + k` no longer reads like snippet line k.
    let spanned = lines.filter {
      let number = Int((side == .old ? $0.oldLineno : $0.newLineno) ?? 0)
      return number >= first && number <= last
    }
    file.composer = ReviewFile.Composer(
      hunk: hunk, lineIndex: indices.last ?? target, side: side, line: first, endLine: last,
      snippet: spanned.map(\.content).joined(separator: "\n"))
    file.editingComment = nil
    rebuildRows()
  }

  func reviewSaveComposer(path: String, text: String) {
    guard let file = filesByPath[path], let composer = file.composer else { return }
    comments.add(
      ReviewComment(
        path: path, side: composer.side, line: composer.line, endLine: composer.endLine,
        snippet: composer.snippet, text: text))
    file.composer = nil
    file.selection = nil
    reloadComments([path])
    focus()
  }

  func reviewDraftChanged(path: String, text: String, editing: Bool) {
    guard let file = filesByPath[path] else { return }
    if editing {
      file.editingDraft = text
    } else {
      file.composer?.draft = text
    }
  }

  func reviewCancelComposer(path: String) {
    filesByPath[path]?.composer = nil
    scheduleRows()
    focus()
  }

  func reviewEditComment(_ id: String?, path: String) {
    filesByPath[path]?.editingComment = id
    filesByPath[path]?.editingDraft = nil
    scheduleRows()
    if id == nil { focus() }
  }

  func reviewSaveComment(id: String, text: String) {
    comments.update(id: id, text: text)
    let path = comments.comments.first { $0.id == id }?.path
    if let path {
      filesByPath[path]?.editingComment = nil
      filesByPath[path]?.editingDraft = nil
    }
    reloadComments(path.map { [$0] })
    focus()
  }

  func reviewDeleteComment(id: String) {
    let path = comments.comments.first { $0.id == id }?.path
    comments.remove(id: id)
    reloadComments(path.map { [$0] })
  }

  func reviewOpenFile(path: String, line: Int?, diff: Bool) {
    let absolute = (repoRoot as NSString).appendingPathComponent(path)
    if diff {
      host?.gitOpenDiffEditor(absolute)
    } else if let controller = host as? MainWindowController {
      controller.paletteOpenFile(absolute, line: line.map(UInt32.init), column: nil)
    } else {
      host?.gitOpenFile(absolute)
    }
  }

  func reviewCopyPath(_ path: String) {
    NSPasteboard.general.clearContents()
    NSPasteboard.general.setString(path, forType: .string)
  }

  func reviewNeedsDiff(_ path: String) {
    guard let file = filesByPath[path], file.expanded, file.diff == nil || file.stale else { return }
    loadDiff(path)
  }

  func reviewFocusHunk(path: String, hunk: Int) {
    diffContext.focus = (path, hunk)
    diffList.redrawVisibleRows()
    focus()
  }

  func reviewTopFileChanged(_ path: String?) {
    if navigator.current != path { navigator.current = path }
  }

  func reviewCopy() {
    for file in files {
      guard let selection = file.selection, let lines = file.diff?.hunks[safe: selection.hunk]?.lines else {
        continue
      }
      let text = selection.lines.sorted().compactMap { lines[safe: $0]?.content }.joined(separator: "\n")
      NSPasteboard.general.clearContents()
      NSPasteboard.general.setString(text, forType: .string)
      return
    }
    NSSound.beep()
  }

  // MARK: Keyboard

  /// j/k hunks, n/p files (N: next unviewed), s/u/x stage/unstage/revert
  /// (also ⌘Y, ⇧⌘Y, ⌥⌘Z), v viewed, c comment, o open, ⏎/space expand,
  /// Esc clear the selection, t or / filter.
  func reviewKey(_ event: NSEvent) -> Bool {
    let flags = event.modifierFlags.intersection([.command, .control, .option, .shift])
    let key = event.charactersIgnoringModifiers ?? ""
    let focused = diffContext.focus.flatMap { focus in filesByPath[focus.path].map { ($0, focus.hunk) } }
    if flags.contains(.command) {
      guard let (file, hunk) = focused else { return false }
      if key.lowercased() == "y" {
        reviewHunkAction(flags.contains(.shift) ? .unstage : .stage, path: file.path, hunk: hunk)
        return true
      }
      if flags.contains(.option), event.keyCode == 6 {  // Z
        reviewHunkAction(.revert, path: file.path, hunk: hunk)
        return true
      }
      return false
    }
    if flags.contains(.control) || flags.contains(.option) { return false }
    switch key {
    case "j": moveHunk(1)
    case "k": moveHunk(-1)
    case "n": moveFile(1, unviewedOnly: false)
    case "p": moveFile(-1, unviewedOnly: false)
    case "N": moveFile(1, unviewedOnly: true)
    case "s", "u", "x":
      guard let (file, hunk) = focused else { return true }
      reviewHunkAction(key == "s" ? .stage : key == "u" ? .unstage : .revert, path: file.path, hunk: hunk)
    case "v":
      if let path = focused?.0.path ?? navigator.current, let file = filesByPath[path] {
        reviewSetViewed(path, viewed: !file.viewed)
      }
    case "c":
      if let (file, hunk) = focused, file.diff != nil { reviewOpenComposer(path: file.path, hunk: hunk, line: nil) }
    case "o":
      if let path = focused?.0.path ?? navigator.current, let file = filesByPath[path] {
        let hunkIndex = focused?.1 ?? 0
        let line = file.diff?.hunks[safe: hunkIndex].flatMap { hunk in
          (hunk.lines.first { $0.kind != .context } ?? hunk.lines.first).map {
            Int($0.newLineno ?? $0.oldLineno ?? 1)
          }
        }
        reviewOpenFile(path: path, line: line, diff: false)
      }
    case "\r", " ":
      if let path = focused?.0.path ?? navigator.current { reviewToggleExpanded(path) }
    case "\u{1B}":
      for file in files { file.selection = nil }
      diffList.redrawVisibleRows()
    case "t", "T", "/":
      if bounds.width >= 620 { navigator.filterFocusRequest += 1 }
    default:
      return false
    }
    return true
  }

  private var visiblePaths: [String] {
    navigator.visibleItems.map(\.path)
  }

  private func moveHunk(_ delta: Int) {
    let paths = visiblePaths
    guard !paths.isEmpty else { return }
    var (path, hunk) = diffContext.focus.map { ($0.path, $0.hunk) } ?? (navigator.current ?? paths[0], -1)
    var index = paths.firstIndex(of: path) ?? 0
    for _ in 0..<(paths.count * 2 + 2) {
      let file = filesByPath[paths[index]]
      let count = file.flatMap { $0.expanded ? $0.diff?.hunks.count : 0 } ?? 0
      let next = hunk + delta
      if next >= 0, next < count {
        diffContext.focus = (paths[index], next)
        diffList.reveal(path: paths[index], hunk: next, onlyIfNeeded: true)
        diffList.redrawVisibleRows()
        return
      }
      index += delta
      guard paths.indices.contains(index) else { return }
      path = paths[index]
      let nextFile = filesByPath[path]
      if nextFile?.diff == nil || nextFile?.expanded == false {
        // Not loaded (or collapsed): go to the file; it loads when shown.
        revealFile(path)
        return
      }
      hunk = delta > 0 ? -1 : nextFile?.diff?.hunks.count ?? 0
    }
  }

  private func moveFile(_ delta: Int, unviewedOnly: Bool) {
    let paths = visiblePaths
    guard !paths.isEmpty else { return }
    var index = paths.firstIndex(of: diffContext.focus?.path ?? navigator.current ?? "") ?? (delta > 0 ? -1 : paths.count)
    for _ in paths.indices {
      index += delta
      guard paths.indices.contains(index) else { return }
      if !unviewedOnly || filesByPath[paths[index]]?.viewed == false {
        revealFile(paths[index])
        return
      }
    }
  }

  // MARK: - Snapshot checks

  /// `keys=<chars>` types review keys; `select=<path>:<hunk>:<from>-<to>`
  /// selects lines; `comment=<text>` comments on the focused hunk;
  /// `reveal=<path>` scrolls to a file; `scroll=<y>` to a point; `composer`
  /// opens one; `edit` edits the first comment.
  func debugAction(_ action: String) {
    if action.hasPrefix("keys=") {
      for char in action.dropFirst(5) {
        let text = String(char)
        guard
          let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: char.isUppercase ? .shift : [], timestamp: 0,
            windowNumber: window?.windowNumber ?? 0, context: nil, characters: text,
            charactersIgnoringModifiers: text, isARepeat: false, keyCode: 0)
        else { continue }
        _ = reviewKey(event)
      }
    } else if action.hasPrefix("select=") {
      let parts = action.dropFirst(7).split(separator: ":")
      guard parts.count == 3, let hunk = Int(parts[1]) else { return }
      let range = parts[2].split(separator: "-").compactMap { Int($0) }
      guard let from = range.first, let to = range.last else { return }
      reviewToggleLine(path: String(parts[0]), hunk: hunk, line: from, extend: false)
      reviewToggleLine(path: String(parts[0]), hunk: hunk, line: to, extend: true)
    } else if action.hasPrefix("comment="), let focus = diffContext.focus {
      reviewOpenComposer(path: focus.path, hunk: focus.hunk, line: nil)
      reviewSaveComposer(path: focus.path, text: String(action.dropFirst(8)))
    } else if action.hasPrefix("scroll="), let y = Double(action.dropFirst(7)) {
      diffList.scroll(toY: y)
    } else if action.hasPrefix("reveal=") {
      revealFile(String(action.dropFirst(7)))
    } else if action == "edit", let comment = files.lazy.flatMap(\.comments).first {
      reviewEditComment(comment.id, path: comment.path)
    } else if action == "scope=uncommitted" {
      setScope(.uncommitted)
    } else if action.hasPrefix("scope=branch:") {
      setScope(.branch(base: String(action.dropFirst(13))))
    } else if action == "composer", let focus = diffContext.focus {
      reviewOpenComposer(path: focus.path, hunk: focus.hunk, line: nil)
    }
  }

  // MARK: - Comments

  /// The comments on files this scope shows. Comments on other files stay
  /// in the repository's store, out of this review's count and prompt.
  private var scopedComments: [ReviewComment] {
    comments.comments.filter { filesByPath[$0.path] != nil }
  }

  private func copyCommentsAsPrompt() {
    let scoped = scopedComments
    let prompt = ReviewCommentAnchoring.prompt(for: scoped)
    guard !prompt.isEmpty else {
      host?.toasts.show(Toast(kind: .info, message: "There are no review comments in this view yet."))
      return
    }
    NSPasteboard.general.clearContents()
    NSPasteboard.general.setString(prompt, forType: .string)
    host?.toasts.show(
      Toast(
        kind: .success,
        message: "Copied \(scoped.count) comment\(scoped.count == 1 ? "" : "s") as a prompt"))
  }

  private func sendCommentsToAgent(_ terminalID: UUID) {
    let prompt = ReviewCommentAnchoring.prompt(for: scopedComments)
    guard !prompt.isEmpty else {
      host?.toasts.show(Toast(kind: .info, message: "There are no review comments in this view yet."))
      return
    }
    host?.sendToAgent(prompt, terminalID: terminalID)
    markReviewed()
  }

  /// Remember "reviewed up to here" (at most once every 10 s).
  private func markReviewed() {
    if let last = model.lastReview, Date().timeIntervalSince(last.date) < 10 { return }
    let root = repoRoot
    queue.async { [weak self] in
      guard
        case .success(let snapshot) = SafetySnapshots.create(
          reason: "reviewed", root: root, prefix: SafetySnapshots.reviewPrefix)
      else { return }
      SafetySnapshots.prune(root: root, prefix: SafetySnapshots.reviewPrefix, keep: 20)
      DispatchQueue.main.async { self?.model.lastReview = snapshot }
    }
  }

  private func clearComments() {
    let total = comments.comments.count
    guard total > 0 else { return }
    let elsewhere = total - scopedComments.count
    host?.gitConfirm(
      title: "Delete every review comment in this repository?",
      message: "\(total) comment\(total == 1 ? "" : "s") will be removed"
        + (elsewhere > 0 ? ", including \(elsewhere) on files this view doesn't show." : "."),
      confirmTitle: "Delete", destructive: true
    ) { [weak self] proceed in
      guard proceed, let self else { return }
      self.comments.removeAll()
      self.reloadComments()
    }
  }
}

/// Shown over the diff list when there's nothing to review.
struct ReviewEmptyView: View {
  let message: String
  var palette: ChromePalette? = nil

  var body: some View {
    Text(message)
      .font(ChromeFont.ui(13))
      .foregroundStyle(palette?.textTertiary ?? .secondary)
      .frame(maxWidth: .infinity, maxHeight: .infinity)
      .background(palette?.content ?? .clear)
  }
}

extension Array {
  fileprivate subscript(safe index: Int) -> Element? { indices.contains(index) ? self[index] : nil }
}

// MARK: - Header

/// Native header: scope, layout, whitespace, context, progress, comment
/// actions.
struct ReviewHeaderBar: View {
  var model: ReviewSurfaceModel

  /// "3 Lines of Context", "Whole File" (the current one is disabled).
  static func contextTitle(_ lines: Int) -> String {
    switch lines {
    case ReviewOptions.wholeFile...: return "Whole File"
    case 0: return "Changed Lines Only"
    case 1: return "1 Line of Context"
    default: return "\(lines) Lines of Context"
    }
  }

  var body: some View {
    let chrome = model.palette
    HStack(spacing: 8) {
      ChromeMenuButton(help: "Choose what to compare") {
        var items = [
          ChromeMenuItem("Unstaged changes") { model.onSelectScope?(.unstaged) },
          ChromeMenuItem("Staged changes") { model.onSelectScope?(.staged) },
          ChromeMenuItem("All uncommitted changes") { model.onSelectScope?(.uncommitted) },
        ]
        if let base = model.baseBranch {
          items.append(.separator)
          items.append(ChromeMenuItem("Compare with \(base)") { model.onSelectScope?(.branch(base: base)) })
        }
        items.append(.separator)
        items.append(ChromeMenuItem("Last commit") { model.onSelectScope?(.commit(sha: "HEAD")) })
        if let turn = model.agentTurn {
          items.append(ChromeMenuItem("Last agent turn (\(turn.name))") { model.onSelectScope?(turn.scope) })
        }
        if let review = model.lastReview {
          let scope = DiffScope.snapshot(from: review.ref, to: nil)
          items.append(ChromeMenuItem(scope.title) { model.onSelectScope?(scope) })
        }
        return items
      } label: {
        HStack(spacing: 5) {
          Icon(.fileDiff, size: 13)
          Text(model.scope.title).font(ChromeFont.ui(12, weight: .semibold))
          Icon(.chevronDown, size: 10)
        }
        .foregroundStyle(chrome.text)
      }

      if model.fileCount > 0 {
        Text("\(model.fileCount) file\(model.fileCount == 1 ? "" : "s")")
          .font(ChromeFont.ui(11.5))
          .foregroundStyle(chrome.textSecondary)
        DiffStat(added: model.totalAdded, removed: model.totalRemoved)
      }
      if model.isLoading {
        ProgressRing(progress: nil, color: chrome.textTertiary, size: 11, lineWidth: 1.5)
      }

      Spacer(minLength: 8)

      if model.fileCount > 0 {
        Text("\(model.viewedCount)/\(model.fileCount) viewed")
          .font(ChromeFont.mono(11))
          .foregroundStyle(
            model.viewedCount == model.fileCount ? chrome.success : chrome.textSecondary)
      }

      ChromeSegmented(
        options: [("unified", "Unified"), ("split", "Split")],
        selection: Binding(get: { model.options.layout }, set: { model.onSetLayout?($0) }))

      ChromeIconButton(
        icon: .space,
        help: model.options.ignoreWhitespace ? "Show whitespace changes" : "Hide whitespace changes",
        isActive: model.options.ignoreWhitespace
      ) { model.onToggleWhitespace?() }

      ChromeMenuButton(help: "Context: \(Self.contextTitle(model.options.contextLines))") {
        Set([model.defaultContextLines, 3, 10, 25, ReviewOptions.wholeFile]).sorted().map { lines in
          ChromeMenuItem(Self.contextTitle(lines), isEnabled: lines != model.options.contextLines) {
            model.onSetContextLines?(lines)
          }
        }
      } label: {
        Icon(.chevronsUpDown, size: 13)
          .foregroundStyle(
            model.options.contextLines > model.defaultContextLines ? chrome.accent : chrome.textSecondary)
      }

      ChromeMenuButton(help: "Review comments") {
        var items: [ChromeMenuItem] = []
        let agents = model.onListAgents?() ?? []
        for agent in agents {
          items.append(
            ChromeMenuItem("Send Comments to \(agent.agentName) · \(agent.tabTitle)", isEnabled: model.commentCount > 0) {
              model.onSendToAgent?(agent.id)
            })
        }
        if !agents.isEmpty { items.append(.separator) }
        return items + [
          ChromeMenuItem("Copy Comments as Prompt", isEnabled: model.commentCount > 0) {
            model.onCopyPrompt?()
          },
          .separator,
          ChromeMenuItem("Delete All Comments in Repository…", isEnabled: model.repositoryCommentCount > 0) {
            model.onClearComments?()
          },
        ]
      } label: {
        HStack(spacing: 4) {
          Icon(.messageSquare, size: 13)
          if model.commentCount > 0 {
            Text("\(model.commentCount)").font(ChromeFont.mono(11))
          }
        }
        .foregroundStyle(model.commentCount > 0 ? chrome.accent : chrome.textSecondary)
      }

      ChromeIconButton(icon: .gitBranch, help: "Show Changes panel to commit (⌃⇧G)") {
        model.onShowChanges?()
      }
      ChromeIconButton(icon: .refreshCw, help: "Refresh") { model.onRefresh?() }
    }
    .padding(.horizontal, 10)
    .frame(height: 36)
    .background(chrome.panel)
    .overlay(alignment: .bottom) { Hairline() }
    .environment(\.chrome, chrome)
  }
}
