import AppKit
import ImpulseGit
import ImpulseKit
import Observation

/// What the palette does with the user's choice. Implemented by the window.
protocol PaletteHost: AnyObject {
  var paletteRoot: String { get }
  var paletteOpenFiles: [String] { get }
  var paletteTabs: [TabDisplayInfo] { get }
  var paletteCurrentBranch: String? { get }
  var paletteHasEditor: Bool { get }
  func paletteOpenFile(_ path: String, line: UInt32?, column: UInt32?)
  func paletteGoToLine(_ line: UInt32, column: UInt32?)
  func paletteSwitchBranch(_ branch: String)
  func paletteCreateBranch(_ name: String)
  func paletteSelectTab(_ index: Int)
  /// The window's workspaces, and the global tab indexes the strip shows.
  var paletteWorkspaces: [WorkspaceInfo] { get }
  var paletteVisibleTabIndices: [Int] { get }
  func paletteSelectWorkspace(_ id: UUID)
  /// Open a folder as a workspace; nil asks for one.
  func paletteOpenWorkspace(folder: String?)
  /// The focused terminal's folder and repository, for history filters.
  var paletteHistoryContext: (cwd: String?, repo: String?) { get }
  /// Put a command from history at the prompt (not run).
  func paletteInsertCommand(_ command: String)
  /// Open pull requests in the repository (nil: gh missing or failed).
  func palettePullRequests(_ completion: @escaping ([PullRequestSummary]?) -> Void)
  /// Check a pull request out into a new task worktree.
  func paletteCheckOutPullRequest(_ pullRequest: PullRequestSummary)
  /// Open the Settings tab at one setting.
  func paletteOpenSetting(_ key: String)
  /// The focused editor's symbols (nil: no editor or no language server).
  func paletteDocumentSymbols(_ completion: @escaping ([OutlineSymbol]?) -> Void)
  /// The project's `.impulse/project.toml` actions, and running one.
  var paletteProjectActions: [ProjectConfig.Action] { get }
  func paletteRunProjectAction(_ action: ProjectConfig.Action)
  func paletteEditProjectConfig()
  /// Project symbols matching `query` from the focused editor's language
  /// server (nil: no editor or no server).
  func paletteWorkspaceSymbols(
    _ query: String, completion: @escaping ([(symbol: OutlineSymbol, path: String)]?) -> Void)
}

/// One result row.
struct PaletteRow: Identifiable {
  enum Glyph {
    case lucide(LucideIcon)
    case image(NSImage)
  }

  let id: String
  let glyph: Glyph?
  let title: String
  /// UTF-16 offsets in `title` to emphasize (fuzzy match positions).
  var highlights: [Int] = []
  var subtitle: String? = nil
  var trailing: String? = nil
  let run: () -> Void
}

/// State and providers for the command palette / quick open.
///
/// The query's prefix picks the mode, so one field covers everything:
///   (none) files · `>` commands · `:` go to line · `%` text in files ·
///   `b:` branches · `t:` tabs · `w:` workspaces · `h:` history · `?` help.
@Observable
final class PaletteModel {
  enum Mode: Equatable {
    case files, commands, goToLine, text, branches, tabs, workspaces, history, pullRequests, settings, symbols
    case workspaceSymbols, actions, help

    var placeholder: String {
      switch self {
      case .files: return "Go to file…  (type > for commands, ? for help)"
      case .commands: return "Run a command…"
      case .goToLine: return "Go to line, e.g. 42 or 42:7"
      case .text: return "Search text in project…"
      case .branches: return "Switch to branch…"
      case .tabs: return "Switch to tab…"
      case .workspaces: return "Switch to workspace or open a folder…"
      case .history: return "Search history…  @here @repo @failed @today"
      case .pullRequests: return "Check out a pull request into a new task…"
      case .settings: return "Find a setting…"
      case .symbols: return "Go to symbol in this file…"
      case .workspaceSymbols: return "Go to symbol in the project…"
      case .actions: return "Run a project action…"
      case .help: return "Palette modes"
      }
    }

    var icon: LucideIcon {
      switch self {
      case .files: return .file
      case .commands: return .command
      case .goToLine: return .cornerDownLeft
      case .text: return .search
      case .branches: return .gitBranch
      case .tabs: return .layers
      case .workspaces: return .folderGit2
      case .history: return .history
      case .pullRequests: return .gitPullRequest
      case .settings: return .settings
      case .symbols: return .code
      case .workspaceSymbols: return .code
      case .actions: return .play
      case .help: return .info
      }
    }
  }

  var query: String = "" {
    // Return in the field writes the same text back to the binding; a
    // refresh then would reset the selection to the top row before the
    // submit runs.
    didSet { if query != oldValue { refresh() } }
  }
  private(set) var rows: [PaletteRow] = []
  var selectedIndex: Int = 0
  private(set) var isBusy = false
  private(set) var mode: Mode = .files
  /// Text shown when there are no rows.
  private(set) var emptyMessage: String = ""

  @ObservationIgnored weak var host: PaletteHost?
  @ObservationIgnored var commands: [AppCommand] = []
  @ObservationIgnored var shortcutOverrides: [String: String] = [:]
  @ObservationIgnored var iconCache: IconCache?
  @ObservationIgnored var onDismiss: (() -> Void)?

  @ObservationIgnored private var fileIndex: [String] = []
  @ObservationIgnored private var fileIndexRoot: String = ""
  @ObservationIgnored private var fileIndexDate: Date = .distantPast
  @ObservationIgnored private var branches: [String]?
  @ObservationIgnored private var pullRequests: [PullRequestSummary]?
  @ObservationIgnored private var symbols: [OutlineSymbol]?
  @ObservationIgnored private var generation = 0
  @ObservationIgnored private var textSearchWork: DispatchWorkItem?

  private static let recentsKey = "paletteRecentCommands"
  private static let worker = DispatchQueue(label: "impulse.palette", qos: .userInitiated)

  /// Prepare for showing: reload the file index if the root changed or it's
  /// stale, forget cached branches, and set the initial query.
  func prepare(prefix: String) {
    branches = nil
    pullRequests = nil
    symbols = nil
    let root = host?.paletteRoot ?? ""
    if root != fileIndexRoot || Date().timeIntervalSince(fileIndexDate) > 20 {
      fileIndexRoot = root
      fileIndex = []
      loadFileIndex(root: root)
    }
    if query == prefix {
      refresh()
    } else {
      query = prefix
    }
  }

  // MARK: - Selection

  func moveSelection(_ delta: Int) {
    guard !rows.isEmpty else { return }
    selectedIndex = (selectedIndex + delta + rows.count) % rows.count
  }

  func runSelected() {
    guard rows.indices.contains(selectedIndex) else {
      if mode == .goToLine { runGoToLine() }
      return
    }
    run(rows[selectedIndex])
  }

  func run(_ row: PaletteRow) {
    onDismiss?()
    // Let the panel close and focus return to the window before acting.
    DispatchQueue.main.async { row.run() }
  }

  // MARK: - Modes

  private func parse() -> (Mode, String) {
    if query.hasPrefix(">") { return (.commands, String(query.dropFirst())) }
    if query.hasPrefix(":") { return (.goToLine, String(query.dropFirst())) }
    if query.hasPrefix("%") { return (.text, String(query.dropFirst())) }
    if query.hasPrefix("b:") { return (.branches, String(query.dropFirst(2))) }
    if query.hasPrefix("t:") { return (.tabs, String(query.dropFirst(2))) }
    if query.hasPrefix("w:") { return (.workspaces, String(query.dropFirst(2))) }
    if query.hasPrefix("h:") { return (.history, String(query.dropFirst(2))) }
    if query.hasPrefix("pr:") { return (.pullRequests, String(query.dropFirst(3))) }
    if query.hasPrefix("set:") { return (.settings, String(query.dropFirst(4))) }
    if query.hasPrefix("@") { return (.symbols, String(query.dropFirst())) }
    if query.hasPrefix("#") { return (.workspaceSymbols, String(query.dropFirst())) }
    if query.hasPrefix("a:") { return (.actions, String(query.dropFirst(2))) }
    if query.hasPrefix("?") { return (.help, "") }
    return (.files, query)
  }

  private func refresh() {
    generation += 1
    textSearchWork?.cancel()
    let (newMode, term) = parse()
    mode = newMode
    selectedIndex = 0
    let trimmed = term.trimmingCharacters(in: .whitespaces)
    switch newMode {
    case .files: refreshFiles(trimmed)
    case .commands: refreshCommands(trimmed)
    case .goToLine: refreshGoToLine(trimmed)
    case .text: refreshText(trimmed)
    case .branches: refreshBranches(trimmed)
    case .tabs: refreshTabs(trimmed)
    case .workspaces: refreshWorkspaces(trimmed)
    case .history: refreshHistory(trimmed)
    case .pullRequests: refreshPullRequests(trimmed)
    case .settings: refreshSettings(trimmed)
    case .symbols: refreshSymbols(trimmed)
    case .workspaceSymbols: refreshWorkspaceSymbols(trimmed)
    case .actions: refreshActions(trimmed)
    case .help: refreshHelp()
    }
  }

  // MARK: Files

  private func loadFileIndex(root: String) {
    guard !root.isEmpty else { return }
    isBusy = true
    Self.worker.async { [weak self] in
      let files = FileIndex.files(root: root)
      DispatchQueue.main.async {
        guard let self, self.fileIndexRoot == root else { return }
        self.fileIndex = files
        self.fileIndexDate = Date()
        self.isBusy = false
        if self.mode == .files { self.refresh() }
      }
    }
  }

  private func refreshFiles(_ term: String) {
    let root = fileIndexRoot
    if term.isEmpty {
      // Open files first, most useful when no query has been typed.
      let open = host?.paletteOpenFiles ?? []
      rows = open.prefix(12).map { path in fileRow(absolute: path, root: root, positions: []) }
      emptyMessage = isBusy ? "Indexing files…" : "Type to search files in \(displayRoot(root))"
      return
    }
    let index = fileIndex
    let generation = self.generation
    Self.worker.async { [weak self] in
      let ranked = FuzzyMatcher.rank(index, query: term, limit: 60, isPath: true) { $0 }
      DispatchQueue.main.async {
        guard let self, self.generation == generation else { return }
        self.rows = ranked.map { entry in
          self.fileRow(
            absolute: (root as NSString).appendingPathComponent(entry.item), root: root,
            positions: entry.match.positions)
        }
        self.emptyMessage = self.isBusy ? "Indexing files…" : "No matching files"
      }
    }
  }

  private func fileRow(absolute path: String, root: String, positions: [Int]) -> PaletteRow {
    let relative =
      path.hasPrefix(root + "/") ? String(path.dropFirst(root.count + 1)) : path
    let name = (relative as NSString).lastPathComponent
    let dir = (relative as NSString).deletingLastPathComponent
    // Map match positions in the relative path onto the file name.
    let nameStart = relative.utf16.count - name.utf16.count
    let highlights = positions.compactMap { $0 >= nameStart ? $0 - nameStart : nil }
    let icon = iconCache?.icon(filename: name, isDirectory: false, expanded: false)
    return PaletteRow(
      id: "file:" + path, glyph: icon.map { .image($0) } ?? .lucide(.file), title: name,
      highlights: highlights, subtitle: dir.isEmpty ? nil : dir
    ) { [weak self] in
      self?.host?.paletteOpenFile(path, line: nil, column: nil)
    }
  }

  private func displayRoot(_ root: String) -> String {
    root.isEmpty ? "the project" : TabManager.abbreviateHomePath(root)
  }

  // MARK: Commands

  private func refreshCommands(_ term: String) {
    let recents = Self.recents()
    let ranked: [(item: AppCommand, match: FuzzyMatcher.Match)]
    if term.isEmpty {
      let byRecent = commands.sorted { a, b in
        let ra = recents.firstIndex(of: a.id) ?? Int.max
        let rb = recents.firstIndex(of: b.id) ?? Int.max
        return ra != rb ? ra < rb : a.title < b.title
      }
      ranked = byRecent.map { ($0, FuzzyMatcher.Match(score: 0, positions: [])) }
    } else {
      // Match titles; fall back to "category title keywords" so e.g. "git"
      // finds Review Changes. Recent commands get a small boost.
      var scored: [(item: AppCommand, match: FuzzyMatcher.Match, score: Int)] = []
      for command in commands {
        let recentBoost = recents.firstIndex(of: command.id).map { max(0, 20 - $0 * 2) } ?? 0
        if let m = FuzzyMatcher.match(term, in: command.title) {
          scored.append((command, m, m.score + recentBoost + 10))
        } else if let m = FuzzyMatcher.match(
          term, in: ([command.category] + command.keywords).joined(separator: " "))
        {
          scored.append((command, FuzzyMatcher.Match(score: m.score, positions: []), m.score / 2))
        }
      }
      scored.sort { $0.score > $1.score }
      ranked = scored.map { ($0.item, $0.match) }
    }
    rows = ranked.map { entry in
      let command = entry.item
      let shortcut =
        command.keybindingId.flatMap {
          Keybindings.symbolDisplay(forId: $0, overrides: shortcutOverrides)
        } ?? command.shortcut
      return PaletteRow(
        id: "cmd:" + command.id, glyph: command.icon.map { .lucide($0) }, title: command.title,
        highlights: entry.match.positions, subtitle: command.category, trailing: shortcut
      ) {
        Self.recordRecent(command.id)
        command.action()
      }
    }
    emptyMessage = "No matching commands"
  }

  private static func recents() -> [String] {
    UserDefaults.standard.stringArray(forKey: recentsKey) ?? []
  }

  private static func recordRecent(_ id: String) {
    var list = recents().filter { $0 != id }
    list.insert(id, at: 0)
    UserDefaults.standard.set(Array(list.prefix(20)), forKey: recentsKey)
  }

  // MARK: Go to line

  private func parseLine(_ term: String) -> (UInt32, UInt32?)? {
    let parts = term.split(separator: ":", omittingEmptySubsequences: false)
    guard let first = parts.first, let line = UInt32(first), line > 0 else { return nil }
    let column = parts.count > 1 ? UInt32(parts[1]) : nil
    return (line, column)
  }

  private func refreshGoToLine(_ term: String) {
    guard host?.paletteHasEditor == true else {
      rows = []
      emptyMessage = "Open a file in the editor to go to a line"
      return
    }
    if let (line, column) = parseLine(term) {
      let label = column.map { "Go to line \(line), column \($0)" } ?? "Go to line \(line)"
      rows = [
        PaletteRow(id: "line", glyph: .lucide(.cornerDownLeft), title: label) { [weak self] in
          self?.host?.paletteGoToLine(line, column: column)
        }
      ]
    } else {
      rows = []
      emptyMessage = "Type a line number, optionally :column"
    }
  }

  private func runGoToLine() {
    let (_, term) = parse()
    guard let (line, column) = parseLine(term) else { return }
    onDismiss?()
    DispatchQueue.main.async { [weak self] in self?.host?.paletteGoToLine(line, column: column) }
  }

  // MARK: Text search

  private func refreshText(_ term: String) {
    rows = []
    guard term.count >= 2 else {
      emptyMessage = "Type at least 2 characters"
      isBusy = false
      return
    }
    let root = fileIndexRoot
    guard !root.isEmpty else {
      emptyMessage = "No project folder"
      return
    }
    isBusy = true
    emptyMessage = "Searching…"
    let generation = self.generation
    let work = DispatchWorkItem { [weak self] in
      let results = FileSearch.searchContents(
        root: root, query: term, limit: 200, caseSensitive: term.contains { $0.isUppercase })
      DispatchQueue.main.async {
        guard let self, self.generation == generation else { return }
        self.isBusy = false
        self.rows = results.map { result in
          let relative =
            result.path.hasPrefix(root + "/")
            ? String(result.path.dropFirst(root.count + 1)) : result.path
          let line = result.lineNumber
          let column = result.columnStart
          let snippet = (result.lineContent ?? "").trimmingCharacters(in: .whitespaces)
          return PaletteRow(
            id: "text:\(result.path):\(line ?? 0):\(column ?? 0)", glyph: .lucide(.fileCode),
            title: snippet.isEmpty ? (relative as NSString).lastPathComponent : snippet,
            subtitle: "\(relative):\(line ?? 0)"
          ) { [weak self] in
            self?.host?.paletteOpenFile(
              result.path, line: line, column: column.map { $0 + 1 })
          }
        }
        self.emptyMessage = "No matches for “\(term)”"
      }
    }
    textSearchWork = work
    Self.worker.asyncAfter(deadline: .now() + 0.15, execute: work)
  }

  // MARK: Branches

  private func refreshBranches(_ term: String) {
    let current = host?.paletteCurrentBranch
    func build(_ names: [String]) {
      let ranked = FuzzyMatcher.rank(names, query: term) { $0 }
      var built: [PaletteRow] = ranked.map { entry in
        let name = entry.item
        let isRemote = remoteBranchNames.contains(name)
        return PaletteRow(
          id: "branch:" + name, glyph: .lucide(isRemote ? .globe : .gitBranch), title: name,
          highlights: entry.match.positions,
          trailing: name == current ? "current" : isRemote ? "remote" : nil
        ) { [weak self] in
          guard name != current else { return }
          self?.host?.paletteSwitchBranch(isRemote ? Self.localName(forRemote: name) : name)
        }
      }
      let trimmed = term.trimmingCharacters(in: .whitespaces)
      if !trimmed.isEmpty, !names.contains(trimmed), Self.isValidBranchName(trimmed) {
        built.append(
          PaletteRow(
            id: "branch-create:" + trimmed, glyph: .lucide(.gitBranchPlus),
            title: "Create branch “\(trimmed)”", subtitle: current.map { "from \($0)" }
          ) { [weak self] in
            self?.host?.paletteCreateBranch(trimmed)
          })
      }
      rows = built
      emptyMessage = names.isEmpty ? "Not a git repository" : "No matching branches"
    }
    if let branches {
      build(branches)
      return
    }
    let root = fileIndexRoot
    isBusy = true
    Self.worker.async { [weak self] in
      let lists = GitOperations.branches(root: root)
      DispatchQueue.main.async {
        guard let self else { return }
        self.isBusy = false
        // Local branches (most recent first, current on top), then remotes
        // that don't already have a local branch.
        let local = lists.local.filter { $0 == current } + lists.local.filter { $0 != current }
        let remote = lists.remote.filter { !lists.local.contains(Self.localName(forRemote: $0)) }
        self.remoteBranchNames = Set(remote)
        self.branches = local + remote
        if self.mode == .branches { self.refresh() }
      }
    }
    rows = []
    emptyMessage = "Loading branches…"
  }

  @ObservationIgnored private var remoteBranchNames: Set<String> = []

  /// "origin/feature" → "feature" (git switch creates a tracking branch).
  static func localName(forRemote name: String) -> String {
    guard let slash = name.firstIndex(of: "/") else { return name }
    return String(name[name.index(after: slash)...])
  }

  /// Rough `git check-ref-format --branch` rules, enough to avoid offering
  /// obviously invalid names.
  static func isValidBranchName(_ name: String) -> Bool {
    GitRefName.isValid(name)
  }

  // MARK: Tabs

  private func refreshTabs(_ term: String) {
    let tabs = host?.paletteTabs ?? []
    let visible = host?.paletteVisibleTabIndices ?? []
    let showWorkspace = (host?.paletteWorkspaces.count ?? 1) > 1
    let ranked = FuzzyMatcher.rank(tabs, query: term) { $0.title }
    rows = ranked.map { entry in
      let tab = entry.item
      let position = visible.firstIndex(of: tab.index)
      let subtitle = [showWorkspace ? tab.workspaceName : nil, tab.directory]
        .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
      return PaletteRow(
        id: "tab:\(tab.id)",
        glyph: tab.isTerminal ? .lucide(.terminal) : tab.icon.map { .image($0) },
        title: tab.title, highlights: entry.match.positions,
        subtitle: subtitle.isEmpty ? nil : subtitle,
        trailing: position.flatMap { $0 < 9 ? "⌘\($0 + 1)" : nil }
      ) { [weak self] in
        self?.host?.paletteSelectTab(tab.index)
      }
    }
    emptyMessage = "No matching tabs"
  }

  // MARK: History

  private static let relativeTime: RelativeDateTimeFormatter = {
    let formatter = RelativeDateTimeFormatter()
    formatter.unitsStyle = .abbreviated
    return formatter
  }()

  /// `@here`, `@repo`, `@failed` and `@today` narrow the search; the rest
  /// of the query is matched fuzzily.
  private func refreshHistory(_ term: String) {
    var filter = HistoryFilter()
    var words: [String] = []
    let context = host?.paletteHistoryContext
    for word in term.split(separator: " ") {
      switch word.lowercased() {
      case "@here": filter.cwd = context?.cwd
      case "@repo": filter.repo = context?.repo
      case "@failed": filter.failedOnly = true
      case "@today": filter.since = Calendar.current.startOfDay(for: Date())
      default: words.append(String(word))
      }
    }
    let text = words.joined(separator: " ")
    let generation = self.generation
    isBusy = true
    CommandHistory.shared.search(text, filter: filter) { [weak self] results in
      guard let self, self.generation == generation else { return }
      self.isBusy = false
      self.rows = results.map { result in
        let entry = result.hit.entry
        let title = entry.command.replacingOccurrences(of: "\n", with: " ⏎ ")
        var details: [String] = []
        if let cwd = entry.cwd { details.append(TabManager.abbreviateHomePath(cwd)) }
        if entry.startedAt > Date(timeIntervalSince1970: 0) {
          details.append(Self.relativeTime.localizedString(for: entry.startedAt, relativeTo: Date()))
        }
        if let code = entry.exitCode, code != 0 { details.append("exit \(code)") }
        let failed = (entry.exitCode ?? 0) != 0
        return PaletteRow(
          id: "history:\(entry.command)",
          glyph: .lucide(failed ? .circleX : .history),
          title: title, highlights: result.positions,
          subtitle: details.isEmpty ? nil : details.joined(separator: " · "),
          trailing: result.hit.uses > 1 ? "×\(result.hit.uses)" : nil
        ) { [weak self] in
          self?.host?.paletteInsertCommand(entry.command)
        }
      }
      self.selectedIndex = 0
    }
    emptyMessage = text.isEmpty && filter == HistoryFilter() ? "No history yet" : "No matching commands"
  }

  // MARK: Symbols

  private func refreshSymbols(_ term: String) {
    func build(_ list: [OutlineSymbol]) {
      let entries: [(item: OutlineSymbol, positions: [Int])] =
        term.isEmpty
        ? list.map { ($0, []) }
        : FuzzyMatcher.rank(list, query: term) { $0.name }.map { ($0.item, $0.match.positions) }
      rows = entries.map { entry in
        let symbol = entry.item
        let indent = term.isEmpty ? String(repeating: "  ", count: min(symbol.depth, 6)) : ""
        let subtitle = ([symbol.container.joined(separator: " › ")].filter { !$0.isEmpty } + [symbol.detail ?? ""])
          .filter { !$0.isEmpty }.joined(separator: "  ")
        return PaletteRow(
          id: "symbol:\(symbol.line):\(symbol.column):\(symbol.name)", glyph: .lucide(Self.icon(forSymbolKind: symbol.kind)),
          title: indent + symbol.name,
          highlights: entry.positions.map { $0 + indent.count },
          subtitle: subtitle.isEmpty ? nil : subtitle,
          trailing: "\(symbol.kindName) · \(symbol.line)"
        ) { [weak self] in
          self?.host?.paletteGoToLine(UInt32(symbol.line), column: UInt32(symbol.column))
        }
      }
      emptyMessage = list.isEmpty ? "No symbols in this file" : "No matching symbols"
    }
    if let symbols {
      build(symbols)
      return
    }
    guard host?.paletteHasEditor == true else {
      rows = []
      emptyMessage = "Open a file to see its symbols"
      return
    }
    isBusy = true
    rows = []
    emptyMessage = "Asking the language server…"
    host?.paletteDocumentSymbols { [weak self] list in
      guard let self else { return }
      self.isBusy = false
      guard let list else {
        self.emptyMessage = "No language server for this file"
        return
      }
      self.symbols = list
      if self.mode == .symbols { self.refresh() }
    }
  }

  private func refreshWorkspaceSymbols(_ term: String) {
    rows = []
    guard host?.paletteHasEditor == true else {
      emptyMessage = "Open a file so its language server can search the project"
      return
    }
    guard !term.isEmpty else {
      emptyMessage = "Type a symbol name"
      return
    }
    isBusy = true
    emptyMessage = "Searching…"
    let generation = self.generation
    let root = fileIndexRoot
    // Debounced like text search: servers search the whole project.
    let work = DispatchWorkItem { [weak self] in
      self?.host?.paletteWorkspaceSymbols(term) { [weak self] results in
        guard let self, self.generation == generation else { return }
        self.isBusy = false
        guard let results else {
          self.emptyMessage = "No language server for this file"
          return
        }
        let ranked = FuzzyMatcher.rank(results, query: term, limit: 200) { $0.symbol.name }
        self.rows = ranked.map { entry in
          let (symbol, path) = entry.item
          let relative = path.hasPrefix(root + "/") ? String(path.dropFirst(root.count + 1)) : path
          return PaletteRow(
            id: "wsymbol:\(path):\(symbol.line):\(symbol.name)", glyph: .lucide(Self.icon(forSymbolKind: symbol.kind)),
            title: symbol.name, highlights: entry.match.positions,
            subtitle: ([symbol.container.joined(separator: " › ")].filter { !$0.isEmpty } + ["\(relative):\(symbol.line)"])
              .joined(separator: "  "),
            trailing: symbol.kindName
          ) { [weak self] in
            self?.host?.paletteOpenFile(path, line: UInt32(symbol.line), column: UInt32(symbol.column))
          }
        }
        self.emptyMessage = "No symbols match “\(term)”"
      }
    }
    textSearchWork = work
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.2, execute: work)
  }

  private func refreshActions(_ term: String) {
    let actions = host?.paletteProjectActions ?? []
    let ranked = FuzzyMatcher.rank(actions, query: term) { $0.name }
    rows = ranked.map { entry in
      let action = entry.item
      return PaletteRow(
        id: "action:" + action.name, glyph: .lucide(.play), title: action.name, highlights: entry.match.positions,
        subtitle: action.command, trailing: action.open == "right" || action.open == "down" ? "split" : nil
      ) { [weak self] in
        self?.host?.paletteRunProjectAction(action)
      }
    }
    rows.append(
      PaletteRow(
        id: "action-edit", glyph: .lucide(.pencil),
        title: actions.isEmpty ? "Add project actions…" : "Edit project actions…",
        subtitle: ProjectConfig.relativePath
      ) { [weak self] in
        self?.host?.paletteEditProjectConfig()
      })
    emptyMessage = ""
  }

  private static func icon(forSymbolKind kind: Int) -> LucideIcon {
    switch kind {
    case 5, 10, 11, 23, 26: return .layers  // class, enum, interface, struct, type parameter
    case 6, 9, 12, 25: return .code  // method, constructor, function, operator
    case 2, 3, 4: return .package  // module, namespace, package
    default: return .tag
    }
  }

  // MARK: Settings

  private func refreshSettings(_ term: String) {
    let ranked = FuzzyMatcher.rank(SettingsCatalog.items, query: term) { "\($0.title) \($0.key)" }
    let current = SettingsStore.shared.settings
    rows = ranked.map { entry in
      let item = entry.item
      return PaletteRow(
        id: "setting:" + item.key, glyph: .lucide(item.category.icon), title: item.title,
        subtitle: "\(item.category.rawValue) › \(item.section)",
        trailing: item.isModified(current) ? "changed" : nil
      ) { [weak self] in
        self?.host?.paletteOpenSetting(item.key)
      }
    }
    emptyMessage = "No matching settings"
  }

  // MARK: Pull requests

  private func refreshPullRequests(_ term: String) {
    func build(_ list: [PullRequestSummary]) {
      let ranked = FuzzyMatcher.rank(list, query: term) { "#\($0.number) \($0.title) \($0.headBranch)" }
      rows = ranked.map { entry in
        let pr = entry.item
        return PaletteRow(
          id: "pr:\(pr.number)", glyph: .lucide(.gitPullRequest), title: "#\(pr.number) \(pr.title)",
          subtitle: [pr.headBranch, pr.author].filter { !$0.isEmpty }.joined(separator: " · "),
          trailing: pr.isDraft ? "draft" : nil
        ) { [weak self] in
          self?.host?.paletteCheckOutPullRequest(pr)
        }
      }
      emptyMessage = list.isEmpty ? "No open pull requests" : "No matching pull requests"
    }
    if let pullRequests {
      build(pullRequests)
      return
    }
    isBusy = true
    rows = []
    emptyMessage = "Asking GitHub…"
    let generation = self.generation
    host?.palettePullRequests { [weak self] list in
      guard let self else { return }
      self.isBusy = false
      guard let list else {
        self.emptyMessage = "Couldn't list pull requests (is gh installed and signed in?)"
        return
      }
      self.pullRequests = list
      if self.mode == .pullRequests, self.generation == generation { build(list) } else if self.mode == .pullRequests {
        self.refresh()
      }
    }
  }

  // MARK: Workspaces

  private func refreshWorkspaces(_ term: String) {
    let open = host?.paletteWorkspaces ?? []
    let openRoots = Set(open.filter { !$0.isScratch }.map(\.root))
    let recents = RecentWorkspaces.folders.filter {
      !openRoots.contains($0) && FileManager.default.fileExists(atPath: $0)
    }

    var built: [PaletteRow] = []
    for entry in FuzzyMatcher.rank(open, query: term, text: { $0.name }) {
      let workspace = entry.item
      let detail = [
        workspace.isScratch ? "Follows the active tab" : TabManager.abbreviateHomePath(workspace.root),
        "\(workspace.tabs.count) tab\(workspace.tabs.count == 1 ? "" : "s")",
      ].joined(separator: " · ")
      built.append(
        PaletteRow(
          id: "workspace:\(workspace.id)",
          glyph: .lucide(workspace.isScratch ? .squareTerminal : .folderGit2),
          title: workspace.name, highlights: entry.match.positions, subtitle: detail,
          trailing: workspace.isActive ? "current" : nil
        ) { [weak self] in
          self?.host?.paletteSelectWorkspace(workspace.id)
        })
    }
    let rankedRecents = FuzzyMatcher.rank(recents, query: term, isPath: true) { $0 }
    for entry in rankedRecents {
      let folder = entry.item
      let name = (folder as NSString).lastPathComponent
      built.append(
        PaletteRow(
          id: "recent:\(folder)", glyph: .lucide(.folder), title: name,
          highlights: FuzzyMatcher.match(term, in: name)?.positions ?? [],
          subtitle: TabManager.abbreviateHomePath(folder), trailing: "recent"
        ) { [weak self] in
          self?.host?.paletteOpenWorkspace(folder: folder)
        })
    }
    built.append(
      PaletteRow(
        id: "workspace:open", glyph: .lucide(.folderOpen), title: "Open Folder as Workspace…"
      ) { [weak self] in
        self?.host?.paletteOpenWorkspace(folder: nil)
      })
    rows = built
    emptyMessage = "No matching workspaces"
  }

  // MARK: Help

  private func refreshHelp() {
    let modes: [(String, String, LucideIcon)] = [
      ("", "Go to file", .file),
      (">", "Run a command", .command),
      (":", "Go to line in the current file", .cornerDownLeft),
      ("%", "Search text in the project", .search),
      ("b:", "Switch branch", .gitBranch),
      ("t:", "Switch tab", .layers),
      ("w:", "Switch workspace", .folderGit2),
      ("h:", "Search command history", .history),
      ("pr:", "Check out a pull request", .gitPullRequest),
      ("set:", "Find a setting", .settings),
      ("@", "Go to a symbol in this file", .code),
      ("#", "Go to a symbol in the project", .code),
      ("a:", "Run a project action", .play),
    ]
    rows = modes.map { prefix, title, icon in
      PaletteRow(
        id: "help:" + prefix, glyph: .lucide(icon), title: title,
        trailing: prefix.isEmpty ? "type a name" : prefix
      ) { [weak self] in
        DispatchQueue.main.async { self?.query = prefix }
      }
    }
  }

  /// Help rows change the query instead of dismissing.
  func activate(_ row: PaletteRow) {
    if row.id.hasPrefix("help:") {
      query = String(row.id.dropFirst("help:".count))
    } else {
      run(row)
    }
  }
}
