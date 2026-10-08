import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI

/// The repository's history: a commit list with its graph on the left, the
/// selected commit's changes (the review renderer) on the right. Commits can
/// be checked out, branched from, cherry-picked, reverted, reset to, or
/// compared with the working tree or with each other.
@Observable
final class HistoryModel {
  var scope: GitLog.Scope = .head
  /// History of one file or folder (relative to the repository root).
  var path: String?
  private(set) var entries: [LogEntry] = []
  private(set) var rows: [GraphRow] = []
  private(set) var isLoading = false
  private(set) var reachedEnd = false
  var error: String?
  /// The filter field: free text plus `author:` `path:` `since:` `until:`.
  var filter = "" {
    didSet {
      query = HistoryQuery.parse(filter)
      scheduleQueryReload()
    }
  }
  private(set) var query = HistoryQuery()
  /// What the loaded pages were read with: a page that comes back for
  /// anything else (the scope or filter changed meanwhile) is dropped.
  struct Request: Equatable {
    var scope: GitLog.Scope
    var path: String?
    var query: HistoryQuery
  }
  @ObservationIgnored private(set) var applied = Request(scope: .head, path: nil, query: HistoryQuery())
  @ObservationIgnored private var queryReload: DispatchWorkItem?
  var selectedSha: String?
  /// Bumped to give the commit list keyboard focus (↑/↓ move the selection).
  var focusRequest = 0
  var palette: ChromePalette
  /// Commits not on the upstream yet, and upstream commits not here yet.
  private(set) var outgoing: Set<String> = []
  private(set) var incoming: Set<String> = []
  /// Where HEAD left the default branch; older commits are dimmed.
  private(set) var forkPoint: GitLog.ForkPoint?
  @ObservationIgnored var forkPointLoader: (() -> GitLog.ForkPoint?)?
  @ObservationIgnored var userNameLoader: (() -> String?)?
  /// The commit picked with "Select for Compare".
  var compareBase: LogEntry?
  /// A commit to select once paging reaches it.
  @ObservationIgnored var pendingReveal: String?
  /// Bumped when the list should scroll to the selection again.
  private(set) var scrollToken = 0
  @ObservationIgnored var divergenceLoader: (() -> (outgoing: Set<String>, incoming: Set<String>))?
  /// The selected commit's full details (message body, committer, parents).
  private(set) var details: CommitDetails?
  @ObservationIgnored var detailsLoader: ((String) -> CommitDetails?)?

  /// Load the details for `sha` (shown above its diff).
  func loadDetails(_ sha: String) {
    guard let detailsLoader, details?.sha != sha else { return }
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let loaded = detailsLoader(sha)
      DispatchQueue.main.async {
        guard let self, self.selectedSha.map({ loaded?.sha.hasPrefix($0) ?? false }) ?? false else { return }
        self.details = loaded
      }
    }
  }

  @ObservationIgnored var onSelect: ((LogEntry) -> Void)?
  /// Show a commit's changes by SHA (it may not be loaded in the list yet).
  @ObservationIgnored var onShowCommit: ((String) -> Void)?
  @ObservationIgnored var onAction: ((HistoryAction, LogEntry) -> Void)?
  /// A page of history: (skip, limit).
  /// Reads a page (off the main thread) for a request taken on main.
  @ObservationIgnored var loader: ((Request, Int, Int) -> Result<[LogEntry], GitOperationError>)?

  /// What the menus need: the checked-out branch, the remotes (to tell
  /// `origin/x` from a local `feature/x`) and where tags go.
  struct Context: Equatable {
    var branch: String?
    var remotes: [String] = []
    /// Where tags are pushed to and deleted from (`GitOperations.defaultRemote`).
    var tagRemote: String?
  }
  private(set) var context = Context()
  @ObservationIgnored var contextLoader: (() -> Context)?

  init(palette: ChromePalette) {
    self.palette = palette
  }

  /// Rows the list shows (all, or those matching the filter's text).
  var visible: [(entry: LogEntry, row: GraphRow?)] {
    let text = query.text.trimmingCharacters(in: .whitespaces).lowercased()
    guard !text.isEmpty else {
      return zip(entries, rows).map { ($0, Optional($1)) }
    }
    return entries.filter {
      $0.subject.lowercased().contains(text) || $0.author.lowercased().contains(text)
        || $0.sha.hasPrefix(text) || $0.refs.contains { $0.lowercased().contains(text) }
    }.map { ($0, nil) }
  }

  var maxLanes: Int { min(rows.map(\.width).max() ?? 1, 8) }

  /// The graph only makes sense for unfiltered history.
  var showsGraph: Bool { query == HistoryQuery() }

  /// On HEAD's history, from before the branch forked off the default branch.
  func isBeforeFork(_ sha: String) -> Bool {
    guard scope == .head, let forkPoint else { return false }
    return !forkPoint.branchOnly.contains(sha)
  }

  /// Re-read history when the filter's tokens change (typing settles first).
  private func scheduleQueryReload() {
    queryReload?.cancel()
    guard query.server != applied.query else { return }
    let work = DispatchWorkItem { [weak self] in
      guard let self, self.query.server != self.applied.query else { return }
      self.reload()
    }
    queryReload = work
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.35, execute: work)
  }

  func reload() {
    entries = []
    rows = []
    reachedEnd = false
    applied = Request(scope: scope, path: path, query: query.server)
    loadMore()
    loadSurroundings()
  }

  /// Re-read the commits already loaded (a ref moved: a new tag, a fetch,
  /// a commit) keeping the selection and scroll position.
  func refreshInPlace() {
    guard !isLoading, let loader else { return }
    isLoading = true
    let count = max(entries.count, HistorySurface.pageSize)
    let request = applied
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let result = loader(request, 0, count)
      DispatchQueue.main.async {
        guard let self else { return }
        self.isLoading = false
        guard request == self.applied else { return self.reload() }
        if case .success(let page) = result, page != self.entries {
          self.entries = page
          self.reachedEnd = page.count < count
          self.rows = CommitGraph.layout(page.map { GraphCommit(sha: $0.sha, parents: $0.parents) })
        }
      }
    }
    loadSurroundings()
  }

  /// Divergence from the upstream, the fork point, and the menu context.
  private func loadSurroundings() {
    let divergenceLoader = divergenceLoader
    let forkPointLoader = forkPointLoader
    let contextLoader = contextLoader
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let divergence = divergenceLoader?()
      let fork = forkPointLoader?()
      let context = contextLoader?()
      DispatchQueue.main.async {
        guard let self else { return }
        if let divergence {
          self.outgoing = divergence.outgoing
          self.incoming = divergence.incoming
        }
        self.forkPoint = fork
        if let context, context != self.context { self.context = context }
      }
    }
  }

  /// Select and show a commit by SHA.
  func show(_ sha: String) {
    reveal(sha)
    onShowCommit?(sha)
  }

  /// Select `sha`, paging further into history until it shows up (the
  /// caller shows the commit itself right away).
  /// `sha` may be abbreviated (a hash clicked in terminal output); the
  /// selection takes the full one once the commit is in the list.
  func reveal(_ sha: String) {
    if let entry = entries.first(where: { $0.sha.hasPrefix(sha) }) {
      selectedSha = entry.sha
      loadDetails(entry.sha)
      scrollToken += 1
    } else {
      selectedSha = sha
      loadDetails(sha)
      pendingReveal = sha
      loadMore()
    }
  }

  func loadMore() {
    guard !isLoading, !reachedEnd, let loader else { return }
    isLoading = true
    let skip = entries.count
    let request = applied
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let result = loader(request, skip, HistorySurface.pageSize)
      DispatchQueue.main.async {
        guard let self else { return }
        self.isLoading = false
        // The scope or filter changed while this page loaded: start over.
        guard request == self.applied else {
          self.reload()
          return
        }
        switch result {
        case .success(let page):
          self.error = nil
          self.reachedEnd = page.count < HistorySurface.pageSize
          self.entries += page
          self.rows = CommitGraph.layout(self.entries.map { GraphCommit(sha: $0.sha, parents: $0.parents) })
          if let pending = self.pendingReveal {
            if let match = self.entries.first(where: { $0.sha.hasPrefix(pending) || pending.hasPrefix($0.sha) }) {
              self.pendingReveal = nil
              if self.selectedSha == pending { self.selectedSha = match.sha }
              self.scrollToken += 1
            } else if !self.reachedEnd, self.entries.count < 20_000 {
              self.loadMore()
            } else {
              self.pendingReveal = nil
            }
          } else if self.selectedSha == nil, let first = self.entries.first {
            self.select(first)
          }
        case .failure(let failure):
          self.error = failure.message
          self.reachedEnd = true
        }
      }
    }
  }

  func select(_ entry: LogEntry) {
    selectedSha = entry.sha
    onSelect?(entry)
    loadDetails(entry.sha)
  }

  /// ↑/↓ through the visible rows.
  func moveSelection(_ delta: Int) {
    let list = visible.map(\.entry)
    guard !list.isEmpty else { return }
    let index = list.firstIndex { $0.sha == selectedSha } ?? -1
    let next = max(0, min(list.count - 1, index + delta))
    select(list[next])
    if next >= list.count - 20 { loadMore() }
  }
}

enum HistoryAction: Equatable {
  case checkout, branchHere, tagHere, cherryPick, revert, resetSoft, resetMixed, resetHard
  /// Merge this commit (or the branch on it) into the current branch.
  case mergeIntoCurrent
  /// Rebase the current branch onto this commit (or the branch on it).
  case rebaseCurrentOnto
  case compareWithWorkingTree, selectForCompare, compareWithSelected, copySha, copySubject
  // A branch or tag shown on the commit.
  case switchToBranch(String), mergeRef(String), rebaseOntoRef(String), deleteBranch(String)
  case pushTag(String), deleteTag(String), deleteRemoteTag(String), copyName(String)
}

final class HistorySurface: NSView {
  static let pageSize = 300

  let repository: GitRepositoryState
  let model: HistoryModel
  private weak var host: GitPanelHost?
  private let review: ReviewSurface
  private var listHost: NSView!
  /// Focus asked for before the view was in a window (a new History tab).
  private var focusWhenInWindow = false

  init(
    repository: GitRepositoryState, path: String?, scope: GitLog.Scope = .head, theme: Theme, host: GitPanelHost?
  ) {
    self.repository = repository
    self.host = host
    self.model = HistoryModel(palette: ChromePalette(theme: theme))
    self.review = ReviewSurface(
      repository: repository, scope: .commit(sha: "HEAD"), focusPath: nil, theme: theme, host: host)
    super.init(frame: NSRect(x: 0, y: 0, width: 1000, height: 700))
    model.path = path
    model.scope = scope
    let root = repository.root
    model.loader = { request, skip, limit in
      GitLog.entries(
        root: root, scope: request.scope, path: request.path, query: request.query, skip: skip, limit: limit)
    }
    model.contextLoader = {
      HistoryModel.Context(
        branch: GitOperations.currentBranch(root: root), remotes: GitOperations.remotes(root: root),
        tagRemote: GitOperations.defaultRemote(root: root))
    }
    model.onSelect = { [weak self] entry in
      self?.review.show(scope: .commit(sha: entry.sha), focusPath: path)
    }
    model.onAction = { [weak self] action, entry in self?.perform(action, on: entry) }
    model.onShowCommit = { [weak self] sha in self?.review.show(scope: .commit(sha: sha), focusPath: path) }
    model.divergenceLoader = { GitLog.divergence(root: root) }
    model.forkPointLoader = { GitLog.forkPoint(root: root) }
    model.userNameLoader = { GitLog.userName(root: root) }
    model.detailsLoader = { GitLog.details(root: root, sha: $0) }
    setup()
    model.reload()
    // Refs moved (a tag, a commit, a fetch): refresh what's loaded.
    refsListener = repository.addChangeListener { [weak self] change in
      guard change.contains(.refs) || change.contains(.operation) else { return }
      self?.scheduleRefresh()
    }
  }

  private var refsListener: UUID?
  private var refreshWork: DispatchWorkItem?

  private func scheduleRefresh() {
    refreshWork?.cancel()
    let work = DispatchWorkItem { [weak self] in self?.model.refreshInPlace() }
    refreshWork = work
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.4, execute: work)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

  var title: String {
    let name = (repository.root as NSString).lastPathComponent
    if let path = model.path { return "History · \((path as NSString).lastPathComponent)" }
    return name.isEmpty ? "History" : "History · \(name)"
  }

  /// The commit graph on top, the selected commit's details and changes
  /// below, split by a divider you can drag.
  private func setup() {
    wantsLayer = true
    let list = WorkbenchHosting.make(HistoryListView(model: model))
    listHost = list
    let details = WorkbenchHosting.make(CommitDetailsView(model: model), intrinsicHeight: true)
    let changes = NSView()
    for view in [details, review] as [NSView] {
      view.translatesAutoresizingMaskIntoConstraints = false
      changes.addSubview(view)
    }
    NSLayoutConstraint.activate([
      details.topAnchor.constraint(equalTo: changes.topAnchor),
      details.leadingAnchor.constraint(equalTo: changes.leadingAnchor),
      details.trailingAnchor.constraint(equalTo: changes.trailingAnchor),
      review.topAnchor.constraint(equalTo: details.bottomAnchor),
      review.bottomAnchor.constraint(equalTo: changes.bottomAnchor),
      review.leadingAnchor.constraint(equalTo: changes.leadingAnchor),
      review.trailingAnchor.constraint(equalTo: changes.trailingAnchor),
    ])

    split.isVertical = false
    split.dividerStyle = .thin
    split.dividerTint = model.palette.nsHairline
    split.delegate = self
    split.addArrangedSubview(list)
    split.addArrangedSubview(changes)
    // A taller window gives its room to the changes.
    split.setHoldingPriority(.defaultHigh, forSubviewAt: 0)
    split.setHoldingPriority(.defaultLow, forSubviewAt: 1)
    split.translatesAutoresizingMaskIntoConstraints = false
    addSubview(split)
    NSLayoutConstraint.activate([
      split.topAnchor.constraint(equalTo: topAnchor),
      split.bottomAnchor.constraint(equalTo: bottomAnchor),
      split.leadingAnchor.constraint(equalTo: leadingAnchor),
      split.trailingAnchor.constraint(equalTo: trailingAnchor),
    ])
  }

  private let split = HistorySplitView()
  private var placedDivider = false
  private static let dividerKey = "historyGraphHeight"

  override func layout() {
    super.layout()
    // First layout: the graph gets the height you last gave it, or 40%.
    guard !placedDivider, bounds.height > 0 else { return }
    placedDivider = true
    let saved = UserDefaults.standard.double(forKey: Self.dividerKey)
    let height = saved > 0 ? min(saved, bounds.height - 200) : (bounds.height * 0.4).rounded()
    split.setPosition(max(140, height), ofDividerAt: 0)
  }

  func refresh() {
    model.reload()
  }

  /// Show another branch's history (or HEAD's, or all), from its newest
  /// commit.
  func show(scope: GitLog.Scope) {
    guard scope != model.scope else { return refresh() }
    model.scope = scope
    model.selectedSha = nil
    model.reload()
  }

  func applyTheme(_ theme: Theme) {
    model.palette = ChromePalette(theme: theme)
    split.dividerTint = model.palette.nsHairline
    split.needsDisplay = true
    review.applyTheme(theme)
  }

  func cleanup() {
    review.cleanup()
    if let refsListener { repository.removeChangeListener(refsListener) }
    refsListener = nil
    refreshWork?.cancel()
  }

  /// The commit list takes the keyboard, so ↑/↓ move through commits
  /// right away (the top one is selected when History opens).
  func focus() {
    guard let window else {
      focusWhenInWindow = true
      return
    }
    window.makeFirstResponder(listHost)
    model.focusRequest &+= 1
  }

  override func viewDidMoveToWindow() {
    super.viewDidMoveToWindow()
    guard window != nil, focusWhenInWindow else { return }
    focusWhenInWindow = false
    // After the hosting views are laid out in the window.
    DispatchQueue.main.async { [weak self] in self?.focus() }
  }

  /// Snapshot checks: scroll the selected commit's changes.
  func debugScrollChanges(toY y: Double) {
    review.debugAction("scroll=\(y)")
  }

  /// Select a commit (e.g. from blame) and show its changes.
  func reveal(sha: String) {
    model.show(sha)
  }

  private func perform(_ action: HistoryAction, on entry: LogEntry) {
    let actions = GitActions(repository: repository, host: host)
    switch action {
    case .checkout: actions.checkout(commit: entry.sha)
    case .cherryPick: actions.cherryPick(entry.sha)
    case .revert: actions.revert(entry.sha)
    case .resetSoft: actions.reset(.soft, to: entry.sha)
    case .resetMixed: actions.reset(.mixed, to: entry.sha)
    case .resetHard: actions.reset(.hard, to: entry.sha)
    case .compareWithWorkingTree:
      host?.gitOpenReview(scope: .snapshot(from: entry.sha, to: nil), focusPath: model.path)
    case .selectForCompare:
      model.compareBase = model.compareBase?.sha == entry.sha ? nil : entry
    case .compareWithSelected:
      guard let base = model.compareBase, base.sha != entry.sha else { return }
      // Older → newer, by position in the (newest-first) list.
      let index = { (sha: String) in self.model.entries.firstIndex { $0.sha == sha } ?? 0 }
      let (from, to) = index(base.sha) > index(entry.sha) ? (base, entry) : (entry, base)
      host?.gitOpenReview(scope: .range(from: from.sha, to: to.sha), focusPath: model.path)
    case .copySha, .copySubject:
      NSPasteboard.general.clearContents()
      NSPasteboard.general.setString(action == .copySha ? entry.sha : entry.subject, forType: .string)
    case .branchHere:
      askBranchName(at: entry)
    case .tagHere:
      guard let window else { return }
      GitPrompts.askForTag(
        in: window, root: repository.root, revision: entry.sha, subject: entry.subject, host: host
      ) { tag in
        actions.createTag(tag.name, at: entry.sha, message: tag.message, push: tag.push)
      }
    case .mergeIntoCurrent:
      let (revision, label) = mergeTarget(entry)
      actions.merge(revision, label: label)
    case .rebaseCurrentOnto:
      let (revision, label) = mergeTarget(entry)
      actions.rebase(onto: revision, label: label)
    case .switchToBranch(let name):
      actions.switchBranch(name)
    case .mergeRef(let name):
      actions.merge(name, label: name)
    case .rebaseOntoRef(let name):
      actions.rebase(onto: name, label: name)
    case .deleteBranch(let name):
      actions.deleteBranch(name)
    case .pushTag(let name):
      actions.pushTag(name)
    case .deleteTag(let name):
      actions.deleteTag(name)
    case .deleteRemoteTag(let name):
      actions.deleteRemoteTag(name)
    case .copyName(let name):
      NSPasteboard.general.clearContents()
      NSPasteboard.general.setString(name, forType: .string)
    }
  }

  /// Merging or rebasing onto a commit uses the branch on it, when there is
  /// one (for the merge message), else the commit.
  private func mergeTarget(_ entry: LogEntry) -> (revision: String, label: String) {
    let refs = entry.refs.map { RefDecoration.parse($0, remotes: model.context.remotes) }
    for ref in refs {
      switch ref {
      case .localBranch(let name), .tag(let name): return (name, name)
      case .remoteBranch(let remote, let branch) where branch != "HEAD": return ("\(remote)/\(branch)", "\(remote)/\(branch)")
      default: continue
      }
    }
    return (entry.sha, entry.shortSha)
  }

  private func askBranchName(at entry: LogEntry) {
    guard let window else { return }
    let alert = NSAlert()
    alert.messageText = "New branch at \(entry.shortSha)"
    alert.informativeText = entry.subject
    alert.addButton(withTitle: "Create Branch")
    alert.addButton(withTitle: "Cancel")
    let field = NSTextField(string: "")
    field.placeholderString = "branch-name"
    field.frame = NSRect(x: 0, y: 0, width: 260, height: 24)
    alert.accessoryView = field
    alert.window.initialFirstResponder = field
    alert.beginSheetModal(for: window) { [weak self] response in
      guard let self, response == .alertFirstButtonReturn else { return }
      let name = field.stringValue.trimmingCharacters(in: .whitespaces)
      guard PaletteModel.isValidBranchName(name) else {
        self.host?.toasts.show(Toast(kind: .warning, message: "“\(name)” isn't a valid branch name."))
        return
      }
      GitActions(repository: self.repository, host: self.host).createBranch(name, at: entry.sha)
    }
  }
}

extension HistorySurface: NSSplitViewDelegate {
  func splitView(_ splitView: NSSplitView, constrainMinCoordinate proposed: CGFloat, ofSubviewAt index: Int) -> CGFloat {
    max(proposed, 120)
  }

  func splitView(_ splitView: NSSplitView, constrainMaxCoordinate proposed: CGFloat, ofSubviewAt index: Int) -> CGFloat {
    min(proposed, splitView.bounds.height - 160)
  }

  func splitViewDidResizeSubviews(_ notification: Notification) {
    guard placedDivider, let graph = split.arrangedSubviews.first else { return }
    UserDefaults.standard.set(Double(graph.frame.height), forKey: Self.dividerKey)
  }
}

/// A thin divider in the theme's hairline color.
final class HistorySplitView: NSSplitView {
  var dividerTint: NSColor = .separatorColor
  override var dividerColor: NSColor { dividerTint }
}

// MARK: - List

struct HistoryListView: View {
  var model: HistoryModel
  @FocusState private var focused: Bool

  var body: some View {
    let chrome = model.palette
    VStack(spacing: 0) {
      header
      Hairline()
      if let error = model.error {
        Text(error).font(ChromeFont.ui(12)).foregroundStyle(chrome.danger).padding(16)
        Spacer()
      } else {
        ScrollViewReader { proxy in
          ScrollView {
            LazyVStack(spacing: 0) {
              let rows = model.visible
              let lanes = model.showsGraph ? model.maxLanes : 0
              ForEach(rows, id: \.entry.sha) { item in
                HistoryRowView(
                  entry: item.entry, row: item.row, lanes: lanes,
                  selected: item.entry.sha == model.selectedSha, focused: focused, model: model)
                .id(item.entry.sha)
                .onAppear {
                  if item.entry.sha == rows.last?.entry.sha { model.loadMore() }
                }
              }
              if model.isLoading {
                ProgressRing(progress: nil, color: chrome.textTertiary, size: 12, lineWidth: 1.5)
                  .padding(10)
              }
            }
          }
          .onChange(of: model.selectedSha) { _, sha in
            if let sha { proxy.scrollTo(sha) }
          }
          .onChange(of: model.scrollToken) { _, _ in
            if let sha = model.selectedSha { proxy.scrollTo(sha, anchor: .center) }
          }
        }
      }
    }
    .background(chrome.content)
    .environment(\.chrome, chrome)
    .focusable()
    .focused($focused)
    .focusEffectDisabled()
    .onChange(of: model.focusRequest) { _, _ in focused = true }
    .onAppear { if model.focusRequest > 0 { focused = true } }
    .onKeyPress(.upArrow) {
      model.moveSelection(-1)
      return .handled
    }
    .onKeyPress(.downArrow) {
      model.moveSelection(1)
      return .handled
    }
  }

  private var filterMenu: some View {
    ChromeMenuButton(help: "Filter presets") {
      [
        ChromeMenuItem("My Commits") {
          DispatchQueue.global(qos: .userInitiated).async {
            let name = model.userNameLoader?()
            DispatchQueue.main.async {
              guard let name else { return }
              model.filter = HistoryQuery.setting("author", to: name, in: model.filter)
            }
          }
        },
        .separator,
        ChromeMenuItem("Last 7 Days") { model.filter = HistoryQuery.setting("since", to: "1w", in: model.filter) },
        ChromeMenuItem("Last 30 Days") { model.filter = HistoryQuery.setting("since", to: "30d", in: model.filter) },
        ChromeMenuItem("Last Year") { model.filter = HistoryQuery.setting("since", to: "1y", in: model.filter) },
        .separator,
        ChromeMenuItem("Clear Filter", isEnabled: !model.filter.isEmpty) { model.filter = "" },
      ]
    } label: {
      Icon(.listFilter, size: 13)
        .foregroundStyle(model.query.hasServerFilters ? model.palette.accent : model.palette.textSecondary)
    }
  }

  /// Current branch / All branches, plus the branch shown when History was
  /// opened on another one (Branch Manager ▸ Show History).
  private var scopeOptions: [(value: Int, label: String)] {
    var options = [(value: 0, label: "Current branch"), (value: 1, label: "All branches")]
    if case .branch(let name) = model.scope { options.append((value: 2, label: name)) }
    return options
  }

  private var header: some View {
    let chrome = model.palette
    return HStack(spacing: 8) {
      Icon(.history, size: 13).foregroundStyle(chrome.textSecondary)
      if let path = model.path {
        Text(path).font(ChromeFont.mono(11.5)).foregroundStyle(chrome.text).lineLimit(1)
          .truncationMode(.middle)
      } else {
        ChromeSegmented(
          options: scopeOptions,
          selection: Binding(
            get: {
              switch model.scope {
              case .head: return 0
              case .all: return 1
              case .branch: return 2
              }
            },
            set: { value in
              guard value != 2 else { return }
              model.scope = value == 1 ? .all : .head
              model.reload()
            }))
      }
      Spacer(minLength: 4)
      ChromeTextField(
        placeholder: "Filter, or author: path: since:",
        text: Binding(get: { model.filter }, set: { model.filter = $0 }), icon: .search
      )
      .frame(maxWidth: 300)
      .help("Text matches subjects, authors, SHAs and refs. author:name, path:dir/, since:2w, until:2026-01-01 search all of history.")
      filterMenu
    }
    .padding(.horizontal, 10)
    .frame(height: 36)
    .background(chrome.panel)
  }
}

private struct HistoryRowView: View {
  @Environment(\.chrome) private var chrome
  let entry: LogEntry
  let row: GraphRow?
  let lanes: Int
  let selected: Bool
  /// The list has keyboard focus.
  let focused: Bool
  var model: HistoryModel
  @State private var hovering = false

  private static let relative: RelativeDateTimeFormatter = {
    let formatter = RelativeDateTimeFormatter()
    formatter.unitsStyle = .abbreviated
    return formatter
  }()

  var body: some View {
    HStack(spacing: 6) {
      if lanes > 0 {
        GraphCell(row: row, lanes: lanes, colors: laneColors)
          .frame(width: CGFloat(lanes) * GraphCell.laneWidth + 6, height: 28)
      }
      if model.outgoing.contains(entry.sha) {
        Icon(.arrowUp, size: 10).foregroundStyle(chrome.accent).help("Not pushed yet")
      } else if model.incoming.contains(entry.sha) {
        Icon(.arrowDown, size: 10).foregroundStyle(chrome.info).help("On the upstream, not pulled yet")
      }
      if model.compareBase?.sha == entry.sha {
        Icon(.gitCompare, size: 11).foregroundStyle(chrome.warning).help("Selected for compare")
      }
      ForEach(entry.refs, id: \.self) { text in
        let ref = RefDecoration.parse(text, remotes: model.context.remotes)
        RefChip(ref: ref)
          .contextMenu { RefMenuItems(ref: ref, model: model, entry: entry) }
      }
      if model.scope == .head, let fork = model.forkPoint, fork.sha == entry.sha {
        Icon(.gitFork, size: 10)
          .foregroundStyle(chrome.textSecondary)
          .frame(width: 17, height: 17)
          .overlay(RoundedRectangle(cornerRadius: 4).strokeBorder(chrome.textTertiary.opacity(0.5)))
          .help("Fork point: this branch left \(fork.base) here")
          .accessibilityLabel("Fork point from \(fork.base)")
      }
      Text(entry.subject)
        .font(ChromeFont.ui(12, weight: entry.refs.contains { $0.hasPrefix("HEAD") } ? .semibold : .regular))
        .foregroundStyle(beforeFork ? chrome.textTertiary : chrome.text)
        .lineLimit(1)
        .truncationMode(.tail)
      Spacer(minLength: 6)
      Text(entry.author)
        .font(ChromeFont.ui(11))
        .foregroundStyle(chrome.textTertiary)
        .lineLimit(1)
        .frame(maxWidth: 110, alignment: .trailing)
      Text(Self.relative.localizedString(for: entry.date, relativeTo: Date()))
        .font(ChromeFont.ui(11))
        .foregroundStyle(chrome.textTertiary)
        .frame(width: 58, alignment: .trailing)
      Text(entry.shortSha)
        .font(ChromeFont.mono(10.5))
        .foregroundStyle(chrome.textTertiary)
    }
    .padding(.leading, lanes > 0 ? 2 : 10)
    .padding(.trailing, 10)
    .frame(height: 28)
    .background(selected ? chrome.selection : hovering ? chrome.hover : .clear)
    .overlay {
      if selected && focused {
        Rectangle().strokeBorder(chrome.focusRing, lineWidth: 1.5)
      }
    }
    .contentShape(Rectangle())
    .onTapGesture { model.select(entry) }
    .onHover { hovering = $0 }
    .contextMenu { menu }
    .help("\(entry.subject)\n\(entry.author) <\(entry.email)>\n\(entry.sha)")
    .accessibilityElement(children: .combine)
    .accessibilityLabel("\(entry.subject), \(entry.author), \(entry.shortSha)")
  }

  /// Already on the default branch when this branch forked: greyed out so
  /// the branch's own work stands out.
  private var beforeFork: Bool { model.isBeforeFork(entry.sha) }

  private var laneColors: [Color] {
    [chrome.accent, chrome.success, chrome.warning, chrome.info, chrome.danger, chrome.gitRenamed]
  }

  private var refs: [RefDecoration] {
    entry.refs.map { RefDecoration.parse($0, remotes: model.context.remotes) }
  }

  /// HEAD is on this commit.
  private var isHead: Bool {
    refs.contains { if case .head = $0 { return true }; return $0 == .detachedHead }
  }

  @ViewBuilder
  private var menu: some View {
    let current = model.context.branch ?? "HEAD"
    Button("Check Out (Detached)") { model.onAction?(.checkout, entry) }
    Button("Create Branch Here…") { model.onAction?(.branchHere, entry) }
    Button("Create Tag Here…") { model.onAction?(.tagHere, entry) }
    Divider()
    Button("Merge into \(current)") { model.onAction?(.mergeIntoCurrent, entry) }
      .disabled(isHead)
    Button("Rebase \(current) onto Here") { model.onAction?(.rebaseCurrentOnto, entry) }
      .disabled(isHead || model.context.branch == nil)
    Button("Cherry-Pick") { model.onAction?(.cherryPick, entry) }
    Button("Revert") { model.onAction?(.revert, entry) }
    Menu("Reset \(current) Here") {
      Button("Soft (keep changes staged)") { model.onAction?(.resetSoft, entry) }
      Button("Mixed (keep changes)") { model.onAction?(.resetMixed, entry) }
      Button("Hard (discard changes)…") { model.onAction?(.resetHard, entry) }
    }
    let named = refs.filter { if case .detachedHead = $0 { return false }; return true }
    if !named.isEmpty {
      Divider()
      ForEach(named.indices, id: \.self) { index in
        Menu(Self.menuTitle(named[index])) {
          RefMenuItems(ref: named[index], model: model, entry: entry)
        }
      }
    }
    Divider()
    Button("Compare with Working Tree") { model.onAction?(.compareWithWorkingTree, entry) }
    if let base = model.compareBase, base.sha != entry.sha {
      Button("Compare with \(base.shortSha) · \(base.subject)") { model.onAction?(.compareWithSelected, entry) }
    }
    Button(model.compareBase?.sha == entry.sha ? "Clear Compare Selection" : "Select for Compare") {
      model.onAction?(.selectForCompare, entry)
    }
    Divider()
    Button("Copy SHA") { model.onAction?(.copySha, entry) }
    Button("Copy Subject") { model.onAction?(.copySubject, entry) }
  }

  static func menuTitle(_ ref: RefDecoration) -> String {
    switch ref {
    case .tag(let name): return "Tag \(name)"
    case .remoteBranch: return "Remote Branch \(ref.label)"
    default: return "Branch \(ref.label)"
    }
  }
}

/// The actions for one branch or tag on a commit (its chip's menu, and a
/// submenu of the commit's menu).
private struct RefMenuItems: View {
  let ref: RefDecoration
  var model: HistoryModel
  let entry: LogEntry

  var body: some View {
    let current = model.context.branch ?? "HEAD"
    switch ref {
    case .tag(let name):
      if let remote = model.context.tagRemote {
        Button("Push to \(remote)") { model.onAction?(.pushTag(name), entry) }
      }
      Button("Merge into \(current)") { model.onAction?(.mergeRef(name), entry) }
      Button("Copy Name") { model.onAction?(.copyName(name), entry) }
      Divider()
      Button("Delete Tag") { model.onAction?(.deleteTag(name), entry) }
      if let remote = model.context.tagRemote {
        Button("Delete from \(remote)…") { model.onAction?(.deleteRemoteTag(name), entry) }
      }
    case .localBranch(let name):
      Button("Switch to \(name)") { model.onAction?(.switchToBranch(name), entry) }
      Button("Merge into \(current)") { model.onAction?(.mergeRef(name), entry) }
      Button("Rebase \(current) onto \(name)") { model.onAction?(.rebaseOntoRef(name), entry) }
        .disabled(model.context.branch == nil)
      Button("Copy Name") { model.onAction?(.copyName(name), entry) }
      Divider()
      Button("Delete Branch…") { model.onAction?(.deleteBranch(name), entry) }
    case .head(let name):
      Button("Copy Name") { model.onAction?(.copyName(name), entry) }
    case .remoteBranch(_, let branch):
      let full = ref.label
      if branch != "HEAD" {
        Button("Check Out \(branch)") { model.onAction?(.switchToBranch(branch), entry) }
        Button("Merge into \(current)") { model.onAction?(.mergeRef(full), entry) }
        Button("Rebase \(current) onto \(full)") { model.onAction?(.rebaseOntoRef(full), entry) }
          .disabled(model.context.branch == nil)
      }
      Button("Copy Name") { model.onAction?(.copyName(full), entry) }
    case .detachedHead:
      EmptyView()
    }
  }
}

/// The selected commit: message, who and when, SHA and parents.
private struct CommitDetailsView: View {
  var model: HistoryModel
  @State private var expanded = false

  private static let dates: DateFormatter = {
    let formatter = DateFormatter()
    formatter.dateStyle = .medium
    formatter.timeStyle = .short
    return formatter
  }()

  var body: some View {
    let chrome = model.palette
    VStack(alignment: .leading, spacing: 6) {
      if let details = model.details {
        Text(details.subject)
          .font(ChromeFont.ui(13, weight: .semibold))
          .foregroundStyle(chrome.text)
          .textSelection(.enabled)
          .fixedSize(horizontal: false, vertical: true)
        if !details.body.isEmpty {
          Text(details.body)
            .font(ChromeFont.ui(12))
            .foregroundStyle(chrome.textSecondary)
            .lineLimit(expanded ? nil : 4)
            .textSelection(.enabled)
            .fixedSize(horizontal: false, vertical: true)
          if details.body.components(separatedBy: "\n").count > 4 {
            Button(expanded ? "Show less" : "Show more") { expanded.toggle() }
              .buttonStyle(.plain)
              .font(ChromeFont.ui(11))
              .foregroundStyle(chrome.accent)
          }
        }
        HStack(spacing: 6) {
          Text(details.author).font(ChromeFont.ui(11.5, weight: .medium)).foregroundStyle(chrome.text)
          Text(Self.dates.string(from: details.date)).font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textTertiary)
          if details.committer != details.author {
            Text("· committed by \(details.committer)").font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textTertiary)
          }
          Spacer(minLength: 6)
          shaChip(details.sha, help: "Copy SHA") {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(details.sha, forType: .string)
          }
          ForEach(details.parents, id: \.self) { parent in
            shaChip(parent, help: "Show parent", icon: .arrowDown) { model.show(parent) }
          }
        }
      }
    }
    .padding(.horizontal, 12)
    .padding(.vertical, model.details == nil ? 0 : 10)
    .frame(maxWidth: .infinity, alignment: .leading)
    .background(chrome.panel)
    .overlay(alignment: .bottom) {
      if model.details != nil { Rectangle().fill(chrome.hairline).frame(height: 1) }
    }
    .onChange(of: model.details?.sha) { _, _ in expanded = false }
  }

  private func shaChip(_ sha: String, help: String, icon: LucideIcon = .copy, action: @escaping () -> Void) -> some View {
    let chrome = model.palette
    return Button(action: action) {
      HStack(spacing: 3) {
        Icon(icon, size: 9)
        Text(sha.prefix(7)).font(ChromeFont.mono(10.5))
      }
      .foregroundStyle(chrome.textSecondary)
      .padding(.horizontal, 5)
      .frame(height: 18)
      .background(RoundedRectangle(cornerRadius: 4).fill(chrome.raised))
    }
    .buttonStyle(.plain)
    .help(help)
  }
}

/// One row's slice of the commit graph.
private struct GraphCell: View {
  static let laneWidth: CGFloat = 13
  let row: GraphRow?
  let lanes: Int
  let colors: [Color]

  var body: some View {
    Canvas { context, size in
      guard let row else { return }
      let mid = size.height / 2
      func x(_ lane: Int) -> CGFloat { 9 + CGFloat(lane) * Self.laneWidth }
      func color(_ lane: Int) -> Color { colors[lane % colors.count] }
      for segment in row.top where segment.from < lanes || segment.to < lanes {
        var path = Path()
        path.move(to: CGPoint(x: x(segment.from), y: 0))
        path.addCurve(
          to: CGPoint(x: x(segment.to), y: mid),
          control1: CGPoint(x: x(segment.from), y: mid * 0.6),
          control2: CGPoint(x: x(segment.to), y: mid * 0.4))
        context.stroke(path, with: .color(color(segment.from)), lineWidth: 1.6)
      }
      for segment in row.bottom where segment.from < lanes || segment.to < lanes {
        var path = Path()
        path.move(to: CGPoint(x: x(segment.from), y: mid))
        path.addCurve(
          to: CGPoint(x: x(segment.to), y: size.height),
          control1: CGPoint(x: x(segment.from), y: mid + mid * 0.6),
          control2: CGPoint(x: x(segment.to), y: mid + mid * 0.4))
        context.stroke(path, with: .color(color(segment.to)), lineWidth: 1.6)
      }
      let dot = CGRect(x: x(row.lane) - 4, y: mid - 4, width: 8, height: 8)
      if row.isMerge {
        context.stroke(Path(ellipseIn: dot.insetBy(dx: 0.8, dy: 0.8)), with: .color(color(row.lane)), lineWidth: 1.6)
      } else {
        context.fill(Path(ellipseIn: dot), with: .color(color(row.lane)))
      }
    }
    .accessibilityHidden(true)
  }
}

private struct RefChip: View {
  @Environment(\.chrome) private var chrome
  let ref: RefDecoration

  var body: some View {
    let color: Color = {
      switch ref {
      case .tag: return chrome.warning
      case .remoteBranch: return chrome.textSecondary
      default: return chrome.accent
      }
    }()
    let icon: LucideIcon = {
      switch ref {
      case .tag: return .tag
      case .remoteBranch: return .globe
      default: return .gitBranch
      }
    }()
    HStack(spacing: 3) {
      Icon(icon, size: 9)
      Text(ref.label).font(ChromeFont.ui(10.5, weight: Self.isHead(ref) ? .semibold : .medium)).lineLimit(1)
    }
    .foregroundStyle(color)
    .padding(.horizontal, 5)
    .frame(height: 17)
    .background(
      RoundedRectangle(cornerRadius: 4).fill(
        (ref.isRemote ? chrome.textTertiary : color).opacity(0.14)))
  }

  private static func isHead(_ ref: RefDecoration) -> Bool {
    if case .head = ref { return true }
    return ref == .detachedHead
  }
}

private extension RefDecoration {
  var isRemote: Bool {
    if case .remoteBranch = self { return true }
    return false
  }
}
