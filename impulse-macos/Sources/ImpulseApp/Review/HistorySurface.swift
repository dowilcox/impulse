import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI

/// The repository's history: a commit list with its graph on the left, the
/// selected commit's changes (the review renderer) on the right. Commits can
/// be checked out, branched from, cherry-picked, reverted, reset to, or
/// compared with the working tree.
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
  var filter = ""
  var selectedSha: String?
  var palette: ChromePalette

  @ObservationIgnored var onSelect: ((LogEntry) -> Void)?
  @ObservationIgnored var onAction: ((HistoryAction, LogEntry) -> Void)?
  @ObservationIgnored var loader: ((Int) -> Result<[LogEntry], GitOperationError>)?

  init(palette: ChromePalette) {
    self.palette = palette
  }

  /// Rows the list shows (all, or those matching the filter).
  var visible: [(entry: LogEntry, row: GraphRow?)] {
    let query = filter.trimmingCharacters(in: .whitespaces).lowercased()
    guard !query.isEmpty else {
      return zip(entries, rows).map { ($0, Optional($1)) }
    }
    return entries.filter {
      $0.subject.lowercased().contains(query) || $0.author.lowercased().contains(query)
        || $0.sha.hasPrefix(query) || $0.refs.contains { $0.lowercased().contains(query) }
    }.map { ($0, nil) }
  }

  var maxLanes: Int { min(rows.map(\.width).max() ?? 1, 8) }

  func reload() {
    entries = []
    rows = []
    reachedEnd = false
    loadMore()
  }

  func loadMore() {
    guard !isLoading, !reachedEnd, let loader else { return }
    isLoading = true
    let skip = entries.count
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let result = loader(skip)
      DispatchQueue.main.async {
        guard let self else { return }
        self.isLoading = false
        switch result {
        case .success(let page):
          self.error = nil
          self.reachedEnd = page.count < HistorySurface.pageSize
          self.entries += page
          self.rows = CommitGraph.layout(self.entries.map { GraphCommit(sha: $0.sha, parents: $0.parents) })
          if self.selectedSha == nil, let first = self.entries.first {
            self.selectedSha = first.sha
            self.onSelect?(first)
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
  case compareWithWorkingTree, copySha, copySubject
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
        root: root, scope: model?.scope ?? .head, path: model?.path, skip: skip, limit: Self.pageSize)
    }
    model.onSelect = { [weak self] entry in
      self?.review.show(scope: .commit(sha: entry.sha), focusPath: path)
    }
    model.onAction = { [weak self] action, entry in self?.perform(action, on: entry) }
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
    for view in [list, divider, review] as [NSView] {
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
      review.topAnchor.constraint(equalTo: topAnchor),
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
              let lanes = model.filter.isEmpty ? model.maxLanes : 0
              ForEach(rows, id: \.entry.sha) { item in
                HistoryRowView(
                  entry: item.entry, row: item.row, lanes: lanes,
                  selected: item.entry.sha == model.selectedSha, model: model)
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
        "Filter", text: Binding(get: { model.filter }, set: { model.filter = $0 })
      )
      .textFieldStyle(.roundedBorder)
      .frame(maxWidth: 170)
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
      ForEach(entry.refs, id: \.self) { ref in
        RefChip(ref: ref)
      }
      Text(entry.subject)
        .font(ChromeFont.ui(12, weight: entry.refs.contains { $0.hasPrefix("HEAD") } ? .semibold : .regular))
        .foregroundStyle(chrome.text)
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
    .contentShape(Rectangle())
    .onTapGesture { model.select(entry) }
    .onHover { hovering = $0 }
    .contextMenu { menu }
    .help("\(entry.subject)\n\(entry.author) <\(entry.email)>\n\(entry.sha)")
    .accessibilityElement(children: .combine)
    .accessibilityLabel("\(entry.subject), \(entry.author), \(entry.shortSha)")
  }

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
    Button("Copy SHA") { model.onAction?(.copySha, entry) }
    Button("Copy Subject") { model.onAction?(.copySubject, entry) }
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
