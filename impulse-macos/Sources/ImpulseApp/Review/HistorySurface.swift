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
  /// The tokens the loaded pages were read with (the loader reads this).
  @ObservationIgnored private(set) var appliedQuery = HistoryQuery()
  @ObservationIgnored private var queryReload: DispatchWorkItem?
  var selectedSha: String?
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
  @ObservationIgnored var loader: ((Int) -> Result<[LogEntry], GitOperationError>)?

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
    guard query.server != appliedQuery else { return }
    let work = DispatchWorkItem { [weak self] in
      guard let self, self.query.server != self.appliedQuery else { return }
      self.reload()
    }
    queryReload = work
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.35, execute: work)
  }

  func reload() {
    entries = []
    rows = []
    reachedEnd = false
    appliedQuery = query.server
    loadMore()
    let divergenceLoader = divergenceLoader
    let forkPointLoader = forkPointLoader
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let divergence = divergenceLoader?()
      let fork = forkPointLoader?()
      DispatchQueue.main.async {
        guard let self else { return }
        if let divergence {
          self.outgoing = divergence.outgoing
          self.incoming = divergence.incoming
        }
        self.forkPoint = fork
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
  func reveal(_ sha: String) {
    selectedSha = sha
    loadDetails(sha)
    if entries.contains(where: { $0.sha == sha }) {
      scrollToken += 1
    } else {
      pendingReveal = sha
      loadMore()
    }
  }

  func loadMore() {
    guard !isLoading, !reachedEnd, let loader else { return }
    isLoading = true
    let skip = entries.count
    let applied = appliedQuery
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let result = loader(skip)
      DispatchQueue.main.async {
        guard let self else { return }
        self.isLoading = false
        // The filter changed while this page was loading: start over.
        guard applied == self.appliedQuery else {
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
            if self.entries.contains(where: { $0.sha.hasPrefix(pending) || pending.hasPrefix($0.sha) }) {
              self.pendingReveal = nil
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

enum HistoryAction {
  case checkout, branchHere, cherryPick, revert, resetSoft, resetMixed, resetHard
  case compareWithWorkingTree, selectForCompare, compareWithSelected, copySha, copySubject
}

final class HistorySurface: NSView {
  static let pageSize = 300

  let repository: GitRepositoryState
  let model: HistoryModel
  private weak var host: GitPanelHost?
  private let review: ReviewSurface
  private var listHost: NSView!

  init(repository: GitRepositoryState, path: String?, theme: Theme, host: GitPanelHost?) {
    self.repository = repository
    self.host = host
    self.model = HistoryModel(palette: ChromePalette(theme: theme))
    self.review = ReviewSurface(
      repository: repository, scope: .commit(sha: "HEAD"), focusPath: nil, theme: theme, host: host)
    super.init(frame: NSRect(x: 0, y: 0, width: 1000, height: 700))
    model.path = path
    let root = repository.root
    model.loader = { [weak model] skip in
      GitLog.entries(
        root: root, scope: model?.scope ?? .head, path: model?.path,
        query: model?.appliedQuery ?? HistoryQuery(), skip: skip, limit: Self.pageSize)
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
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

  var title: String {
    let name = (repository.root as NSString).lastPathComponent
    if let path = model.path { return "History · \((path as NSString).lastPathComponent)" }
    return name.isEmpty ? "History" : "History · \(name)"
  }

  private func setup() {
    wantsLayer = true
    let list = WorkbenchHosting.make(HistoryListView(model: model))
    listHost = list
    let divider = NSBox()
    divider.boxType = .custom
    divider.borderWidth = 0
    divider.fillColor = model.palette.nsHairline
    let details = WorkbenchHosting.make(CommitDetailsView(model: model), intrinsicHeight: true)
    for view in [list, divider, details, review] as [NSView] {
      view.translatesAutoresizingMaskIntoConstraints = false
      addSubview(view)
    }
    NSLayoutConstraint.activate([
      list.topAnchor.constraint(equalTo: topAnchor),
      list.bottomAnchor.constraint(equalTo: bottomAnchor),
      list.leadingAnchor.constraint(equalTo: leadingAnchor),
      list.widthAnchor.constraint(equalTo: widthAnchor, multiplier: 0.42),
      divider.topAnchor.constraint(equalTo: topAnchor),
      divider.bottomAnchor.constraint(equalTo: bottomAnchor),
      divider.leadingAnchor.constraint(equalTo: list.trailingAnchor),
      divider.widthAnchor.constraint(equalToConstant: 1),
      details.topAnchor.constraint(equalTo: topAnchor),
      details.leadingAnchor.constraint(equalTo: divider.trailingAnchor),
      details.trailingAnchor.constraint(equalTo: trailingAnchor),
      review.topAnchor.constraint(equalTo: details.bottomAnchor),
      review.bottomAnchor.constraint(equalTo: bottomAnchor),
      review.leadingAnchor.constraint(equalTo: divider.trailingAnchor),
      review.trailingAnchor.constraint(equalTo: trailingAnchor),
    ])
  }

  func refresh() {
    model.reload()
  }

  func applyTheme(_ theme: Theme) {
    model.palette = ChromePalette(theme: theme)
    review.applyTheme(theme)
  }

  func cleanup() {
    review.cleanup()
  }

  func focus() {
    window?.makeFirstResponder(listHost)
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
    }
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

  private var header: some View {
    let chrome = model.palette
    return HStack(spacing: 8) {
      Icon(.history, size: 13).foregroundStyle(chrome.textSecondary)
      if let path = model.path {
        Text(path).font(ChromeFont.mono(11.5)).foregroundStyle(chrome.text).lineLimit(1)
          .truncationMode(.middle)
      } else {
        Picker(
          "",
          selection: Binding(
            get: { model.scope == .all ? 1 : 0 },
            set: { value in
              model.scope = value == 1 ? .all : .head
              model.reload()
            })
        ) {
          Text("Current branch").tag(0)
          Text("All branches").tag(1)
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        .frame(width: 210)
      }
      Spacer(minLength: 4)
      TextField(
        "Filter or author:…", text: Binding(get: { model.filter }, set: { model.filter = $0 })
      )
      .textFieldStyle(.roundedBorder)
      .frame(maxWidth: 260)
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
      ForEach(entry.refs, id: \.self) { ref in
        RefChip(ref: ref)
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

  @ViewBuilder
  private var menu: some View {
    Button("Check Out (Detached)") { model.onAction?(.checkout, entry) }
    Button("Create Branch Here…") { model.onAction?(.branchHere, entry) }
    Divider()
    Button("Cherry-Pick") { model.onAction?(.cherryPick, entry) }
    Button("Revert") { model.onAction?(.revert, entry) }
    Menu("Reset Current Branch Here") {
      Button("Soft (keep changes staged)") { model.onAction?(.resetSoft, entry) }
      Button("Mixed (keep changes)") { model.onAction?(.resetMixed, entry) }
      Button("Hard (discard changes)…") { model.onAction?(.resetHard, entry) }
    }
    Divider()
    Button("Compare with Working Tree") { model.onAction?(.compareWithWorkingTree, entry) }
    if let base = model.compareBase, base.sha != entry.sha {
      Button("Compare with \(base.shortSha) · \(base.subject)") { model.onAction?(.compareWithSelected, entry) }
    }
    Button(model.compareBase?.sha == entry.sha ? "Clear Compare Selection" : "Select for Compare") {
      model.onAction?(.selectForCompare, entry)
    }
    Button("Copy SHA") { model.onAction?(.copySha, entry) }
    Button("Copy Subject") { model.onAction?(.copySubject, entry) }
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
  let ref: String

  var body: some View {
    let isHead = ref.hasPrefix("HEAD")
    let isTag = ref.hasPrefix("tag: ")
    let isRemote = !isHead && !isTag && ref.contains("/")
    let label = isTag ? String(ref.dropFirst(5)) : ref.replacingOccurrences(of: "HEAD -> ", with: "")
    HStack(spacing: 3) {
      Icon(isTag ? .tag : (isRemote ? .globe : .gitBranch), size: 9)
      Text(label).font(ChromeFont.ui(10.5, weight: isHead ? .semibold : .medium)).lineLimit(1)
    }
    .foregroundStyle(isTag ? chrome.warning : isRemote ? chrome.textSecondary : chrome.accent)
    .padding(.horizontal, 5)
    .frame(height: 17)
    .background(
      RoundedRectangle(cornerRadius: 4).fill(
        (isTag ? chrome.warning : isRemote ? chrome.textTertiary : chrome.accent).opacity(0.14)))
  }
}
