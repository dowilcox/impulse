import AppKit
import ImpulseGit
import ImpulseKit
import Observation
import SwiftUI
import WebKit
import os.log

/// Observable header state for the review surface.
@Observable
final class ReviewSurfaceModel {
  var scope: DiffScope
  var options = ReviewOptions()
  var fileCount = 0
  var viewedCount = 0
  var commentCount = 0
  var totalAdded = 0
  var totalRemoved = 0
  var isLoading = false
  /// Branch to compare against for "vs base" (auto-detected).
  var baseBranch: String?
  var palette: ChromePalette

  @ObservationIgnored var onSelectScope: ((DiffScope) -> Void)?
  @ObservationIgnored var onSetLayout: ((String) -> Void)?
  @ObservationIgnored var onToggleWhitespace: (() -> Void)?
  @ObservationIgnored var onCopyPrompt: (() -> Void)?
  @ObservationIgnored var onClearComments: (() -> Void)?
  @ObservationIgnored var onRefresh: (() -> Void)?
  @ObservationIgnored var onShowChanges: (() -> Void)?

  init(scope: DiffScope, palette: ChromePalette) {
    self.scope = scope
    self.palette = palette
  }
}

/// Multi-file change review: a native header (scope, layout, whitespace,
/// progress, comments) over a WebView renderer (web/review.js) with a file
/// navigator, virtualized diff cards, hunk/line staging and reverting, viewed
/// marks and inline comments. Follows the repository live.
final class ReviewSurface: NSView, WKScriptMessageHandler, WKNavigationDelegate {
  let repository: GitRepositoryState
  var repoRoot: String { repository.root }
  private(set) var webView: WKWebView?
  private weak var host: GitPanelHost?

  private let model: ReviewSurfaceModel
  private var theme: Theme
  private var isReady = false
  private var pendingFocus: String?
  private var generation = 0
  private var files: [FileChange] = []
  /// Latest diff per path (for viewed hashes and comment anchoring).
  private var diffs: [String: FileDiff] = [:]
  private var changeListener: UUID?
  private var refreshWork: DispatchWorkItem?
  private let comments: ReviewCommentStore
  private let queue = DispatchQueue(label: "impulse.review", qos: .userInitiated)

  private static let log = OSLog(subsystem: "dev.impulse.Impulse", category: "Review")
  private static let handlerName = "impulseReview"

  init(
    repository: GitRepositoryState, scope: DiffScope?, focusPath: String?, theme: Theme,
    host: GitPanelHost?
  ) {
    self.repository = repository
    self.theme = theme
    self.host = host
    self.comments = ReviewCommentStore.forRepository(repository.root)
    self.model = ReviewSurfaceModel(
      scope: scope ?? Self.defaultScope(repository.snapshot),
      palette: ChromePalette(theme: theme))
    self.pendingFocus = focusPath
    super.init(frame: NSRect(x: 0, y: 0, width: 800, height: 600))
    wantsLayer = true
    layer?.backgroundColor = NSColor(hex: theme.bg).cgColor
    setupViews()
    wireModel()
    loadPage()
    changeListener = repository.addChangeListener { [weak self] _ in
      self?.scheduleRefresh()
    }
    let root = repository.root
    queue.async { [weak self] in
      let base = GitClient.defaultBaseBranch(repoPath: root)
      DispatchQueue.main.async { self?.model.baseBranch = base }
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

  private func setupViews() {
    let header = WorkbenchHosting.make(ReviewHeaderBar(model: model), intrinsicHeight: true)
    header.translatesAutoresizingMaskIntoConstraints = false

    let config = WKWebViewConfiguration()
    config.userContentController.add(WeakScriptHandler(self), name: Self.handlerName)
    let prefs = WKWebpagePreferences()
    prefs.allowsContentJavaScript = true
    config.defaultWebpagePreferences = prefs
    let web = WKWebView(frame: bounds, configuration: config)
    web.navigationDelegate = self
    web.translatesAutoresizingMaskIntoConstraints = false
    web.allowsMagnification = false
    web.underPageBackgroundColor = NSColor(hex: theme.bg)
    webView = web

    addSubview(header)
    addSubview(web)
    NSLayoutConstraint.activate([
      header.topAnchor.constraint(equalTo: topAnchor),
      header.leadingAnchor.constraint(equalTo: leadingAnchor),
      header.trailingAnchor.constraint(equalTo: trailingAnchor),
      web.topAnchor.constraint(equalTo: header.bottomAnchor),
      web.leadingAnchor.constraint(equalTo: leadingAnchor),
      web.trailingAnchor.constraint(equalTo: trailingAnchor),
      web.bottomAnchor.constraint(equalTo: bottomAnchor),
    ])
  }

  private func wireModel() {
    model.onSelectScope = { [weak self] scope in self?.setScope(scope) }
    model.onSetLayout = { [weak self] layout in
      guard let self else { return }
      self.model.options.layout = layout
      self.sendConfigure()
    }
    model.onToggleWhitespace = { [weak self] in
      guard let self else { return }
      self.model.options.ignoreWhitespace.toggle()
      self.diffs.removeAll()
      self.sendConfigure()
      self.refreshFiles()
    }
    model.onCopyPrompt = { [weak self] in self?.copyCommentsAsPrompt() }
    model.onClearComments = { [weak self] in self?.clearComments() }
    model.onRefresh = { [weak self] in self?.refresh() }
    model.onShowChanges = { [weak self] in
      (self?.host as? MainWindowController)?.showChangesPanel()
    }
  }

  private func loadPage() {
    guard let dir = EditorAssets.monacoDirectory else { return }
    webView?.loadFileURL(dir.appendingPathComponent("review.html"), allowingReadAccessTo: dir)
  }

  func cleanup() {
    if let changeListener { repository.removeChangeListener(changeListener) }
    changeListener = nil
    webView?.configuration.userContentController.removeScriptMessageHandler(
      forName: Self.handlerName)
    webView?.navigationDelegate = nil
    webView = nil
  }

  func focus() {
    if let webView { window?.makeFirstResponder(webView) }
  }

  /// Switch between "unified" and "split" rows.
  func setLayout(_ layout: String) {
    model.onSetLayout?(layout)
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
      send(.focus(path: focusPath, line: nil))
    }
  }

  func applyTheme(_ theme: Theme) {
    self.theme = theme
    model.palette = ChromePalette(theme: theme)
    layer?.backgroundColor = NSColor(hex: theme.bg).cgColor
    webView?.underPageBackgroundColor = NSColor(hex: theme.bg)
    sendTheme()
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
    diffs.removeAll()
    files = []
    sendConfigure()
    refreshFiles()
  }

  private var capabilities: ReviewCapabilities {
    switch model.scope {
    case .unstaged: return ReviewCapabilities(stage: true, unstage: false, revert: true)
    case .staged: return ReviewCapabilities(stage: false, unstage: true, revert: false)
    default: return ReviewCapabilities(stage: false, unstage: false, revert: false)
    }
  }

  // MARK: - Messaging

  private func send(_ command: ReviewCommand) {
    guard isReady, let webView,
      let data = try? JSONEncoder().encode(command),
      let json = String(data: data, encoding: .utf8)
    else { return }
    webView.evaluateJavaScript("window.__applyReviewCommand(\(json));") { _, error in
      if let error {
        os_log(.error, log: Self.log, "review command failed: %{public}@", "\(error)")
      }
    }
  }

  private func sendConfigure() {
    send(
      .configure(
        capabilities: capabilities, options: model.options, scopeTitle: model.scope.title))
  }

  private func sendTheme() {
    send(
      .setTheme(
        theme: ThemeManager.monacoTheme(forName: theme.id),
        chrome: ReviewSurface.cssVariables(theme: theme)))
  }

  func userContentController(
    _ userContentController: WKUserContentController, didReceive message: WKScriptMessage
  ) {
    guard let body = message.body as? String, let data = body.data(using: .utf8),
      let event = try? JSONDecoder().decode(ReviewEvent.self, from: data)
    else { return }
    handle(event)
  }

  func webView(
    _ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,
    decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
  ) {
    let scheme = navigationAction.request.url?.scheme
    decisionHandler(scheme == "file" || scheme == "about" ? .allow : .cancel)
  }

  private func handle(_ event: ReviewEvent) {
    switch event {
    case .ready:
      isReady = true
      sendTheme()
      sendConfigure()
      refreshFiles()

    case .requestDiff(let path):
      loadDiff(path: path)

    case let .hunkAction(action, path, hunkIndex, hunkId, lines):
      guard let change = files.first(where: { $0.path == path }), let host else { return }
      let target: PatchTarget = action == .stage ? .stage : action == .unstage ? .unstage : .discard
      let selection: PatchSelection =
        lines.map { .lines(Set($0), inHunk: hunkIndex) } ?? .wholeHunks([hunkIndex])
      send(.setBusy(path: path, busy: true))
      GitActions(repository: repository, host: host).apply(
        target, selection: selection, change: change, expectedHunkIds: [hunkIndex: hunkId],
        options: diffOptions
      ) { [weak self] _ in
        self?.send(.setBusy(path: path, busy: false))
        self?.refreshFiles()
      }

    case let .fileAction(action, path):
      guard let change = files.first(where: { $0.path == path }), let host else { return }
      let actions = GitActions(repository: repository, host: host)
      switch action {
      case .stage: actions.stage([change])
      case .unstage: actions.unstage([change])
      case .revert: actions.discard([change])
      }

    case let .toggleViewed(path, viewed):
      let hash = viewed ? (diffs[path]?.contentHash ?? "unknown") : nil
      ReviewViewedStore.set(path: path, hash: hash, root: repoRoot, scope: scopeKey)
      updateCounts()

    case let .openFile(path, line):
      let absolute = (repoRoot as NSString).appendingPathComponent(path)
      if let controller = host as? MainWindowController {
        controller.paletteOpenFile(absolute, line: line.map(UInt32.init), column: nil)
      } else {
        host?.gitOpenFile(absolute)
      }

    case let .addComment(path, side, line, endLine, text, snippet):
      let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
      guard !trimmed.isEmpty else { return }
      comments.add(
        ReviewComment(
          path: path, side: side == "old" ? .old : .new, line: line, endLine: endLine,
          snippet: snippet, text: trimmed))
      resendDiff(path: path)

    case let .editComment(id, text):
      comments.update(id: id, text: text)
      if let path = comments.comments.first(where: { $0.id == id })?.path { resendDiff(path: path) }

    case .deleteComment(let id):
      let path = comments.comments.first(where: { $0.id == id })?.path
      comments.remove(id: id)
      if let path { resendDiff(path: path) }

    case .copyPath(let path):
      NSPasteboard.general.clearContents()
      NSPasteboard.general.setString(path, forType: .string)
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
    guard isReady else { return }
    generation += 1
    let generation = self.generation
    let root = repoRoot
    let scope = model.scope
    let options = diffOptions
    let viewed = ReviewViewedStore.viewed(root: root, scope: scopeKey)
    model.isLoading = true
    queue.async { [weak self] in
      let result = Result { try GitClient.changedFiles(repoPath: root, scope: scope) }
      // Check viewed files' current diff so changed ones come back unviewed.
      var viewedDiffs: [String: FileDiff] = [:]
      if case .success(let changes) = result {
        for change in changes where viewed[change.path] != nil {
          if let diff = try? GitClient.fileDiff(
            repoPath: root, path: change.path, oldPath: change.oldPath, scope: scope,
            options: options)
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
          self.files = changes
          for (path, diff) in viewedDiffs { self.diffs[path] = diff }
          self.sendFiles(generation: generation)
          if let focus = self.pendingFocus {
            self.pendingFocus = nil
            self.send(.focus(path: focus, line: nil))
          }
        case .failure(let error):
          self.files = []
          self.send(
            .setFiles(
              generation: generation, files: [],
              emptyMessage: "Couldn't read changes: \(error)"))
          self.updateCounts()
        }
      }
    }
  }

  private func fileItems() -> [ReviewFileItem] {
    let viewed = ReviewViewedStore.viewed(root: repoRoot, scope: scopeKey)
    return files.map { change in
      var isViewed = viewed[change.path] != nil
      var changedSince = false
      if let hash = viewed[change.path], let diff = diffs[change.path], hash != diff.contentHash {
        isViewed = false
        changedSince = true
      }
      return ReviewFileItem(
        path: change.path, oldPath: change.oldPath, status: change.status.letter,
        added: change.added, removed: change.removed, binary: change.isBinary, viewed: isViewed,
        changedSinceViewed: changedSince, commentCount: comments.comments(for: change.path).count)
    }
  }

  private func sendFiles(generation: Int) {
    let items = fileItems()
    send(.setFiles(generation: generation, files: items, emptyMessage: emptyMessage))
    updateCounts(items)
  }

  private var emptyMessage: String {
    switch model.scope {
    case .unstaged: return "No unstaged changes."
    case .staged: return "Nothing is staged."
    case .uncommitted: return "No uncommitted changes."
    case .branch(let base): return "No changes compared to \(base)."
    default: return "No changes."
    }
  }

  private func updateCounts(_ items: [ReviewFileItem]? = nil) {
    let list = items ?? fileItems()
    model.fileCount = list.count
    model.viewedCount = list.filter(\.viewed).count
    model.commentCount = comments.comments.count
    model.totalAdded = files.compactMap(\.added).reduce(0, +)
    model.totalRemoved = files.compactMap(\.removed).reduce(0, +)
  }

  private func loadDiff(path: String) {
    guard let change = files.first(where: { $0.path == path }) else { return }
    let root = repoRoot
    let scope = model.scope
    let options = diffOptions
    let generation = self.generation
    queue.async { [weak self] in
      let result = Result {
        try GitClient.fileDiff(
          repoPath: root, path: change.path, oldPath: change.oldPath, scope: scope,
          options: options)
      }
      DispatchQueue.main.async {
        guard let self, generation == self.generation else { return }
        switch result {
        case .success(let diff):
          let previousHash = self.diffs[path]?.contentHash
          self.diffs[path] = diff
          self.sendDiff(diff)
          // A viewed file whose diff changed is now unviewed.
          if previousHash != diff.contentHash {
            let viewed = ReviewViewedStore.viewed(root: root, scope: self.scopeKey)
            if let hash = viewed[path], hash != diff.contentHash {
              self.send(.setViewed(path: path, viewed: false))
              self.updateCounts()
            }
          }
        case .failure(let error):
          self.send(.diffError(path: path, message: "\(error)"))
        }
      }
    }
  }

  private func resendDiff(path: String) {
    if let diff = diffs[path] { sendDiff(diff) }
    updateCounts()
    sendFiles(generation: generation)
  }

  private func sendDiff(_ diff: FileDiff) {
    var oldLines: [Int: String] = [:]
    var newLines: [Int: String] = [:]
    for hunk in diff.hunks {
      for line in hunk.lines {
        if let old = line.oldLineno, line.kind != .added { oldLines[Int(old)] = line.content }
        if let new = line.newLineno, line.kind != .removed { newLines[Int(new)] = line.content }
      }
    }
    let items = comments.comments(for: diff.path).map { comment in
      ReviewCommentItem(
        id: comment.id, side: comment.side.rawValue, line: comment.line, endLine: comment.endLine,
        text: comment.text,
        outdated: ReviewCommentAnchoring.isOutdated(
          comment, lines: comment.side == .old ? oldLines : newLines))
    }
    send(.setFileDiff(ReviewFileDiff(diff, diffHash: diff.contentHash, comments: items)))
  }

  // MARK: - Comments

  private func copyCommentsAsPrompt() {
    let prompt = ReviewCommentAnchoring.prompt(for: comments.comments)
    guard !prompt.isEmpty else {
      host?.toasts.show(Toast(kind: .info, message: "There are no review comments yet."))
      return
    }
    NSPasteboard.general.clearContents()
    NSPasteboard.general.setString(prompt, forType: .string)
    host?.toasts.show(
      Toast(
        kind: .success,
        message: "Copied \(comments.comments.count) comment\(comments.comments.count == 1 ? "" : "s") as a prompt"))
  }

  private func clearComments() {
    guard !comments.comments.isEmpty else { return }
    host?.gitConfirm(
      title: "Delete all review comments?",
      message: "\(comments.comments.count) comment(s) in this repository will be removed.",
      confirmTitle: "Delete", destructive: true
    ) { [weak self] proceed in
      guard proceed, let self else { return }
      let paths = Set(self.comments.comments.map(\.path))
      self.comments.removeAll()
      for path in paths { self.resendDiff(path: path) }
    }
  }

  // MARK: - Theme → CSS

  /// CSS custom properties for the review page, derived like ChromePalette.
  static func cssVariables(theme: Theme) -> [String: String] {
    let bg = NSColor(hex: theme.bg)
    let fg = NSColor(hex: theme.fg)
    let accent = NSColor(hex: theme.accent)
    let isLight = theme.isLight
    func mix(_ a: NSColor, _ b: NSColor, _ t: CGFloat) -> String {
      ChromePalette.mix(a, toward: b, amount: t).hexString
    }
    func rgba(_ hex: String, _ alpha: Double) -> String {
      let c = NSColor(hex: hex).usingColorSpace(.sRGB) ?? .gray
      return String(
        format: "rgba(%d,%d,%d,%.3f)", Int(c.redComponent * 255), Int(c.greenComponent * 255),
        Int(c.blueComponent * 255), alpha)
    }
    return [
      "--bg": theme.bg,
      "--panel": mix(bg, .black, isLight ? 0.02 : 0.14),
      "--chrome": mix(bg, .black, isLight ? 0.035 : 0.22),
      "--raised": mix(bg, fg, isLight ? 0.06 : 0.07),
      "--hairline": mix(bg, fg, 0.12),
      "--hairline-strong": mix(bg, fg, 0.2),
      "--hover": rgba(theme.fg, isLight ? 0.06 : 0.07),
      "--text": theme.fg,
      "--text2": theme.fgMuted,
      "--text3": theme.fgComment,
      "--accent": theme.accent,
      "--accent-soft": rgba(theme.accent, 0.16),
      "--on-accent": ChromePalette.readableText(on: accent).hexString,
      "--added": theme.gitAdded,
      "--removed": theme.gitDeleted,
      "--modified": theme.gitModified,
      "--renamed": theme.gitRenamed,
      "--conflict": theme.gitConflict,
      "--added-bg": rgba(theme.gitAdded, isLight ? 0.12 : 0.1),
      "--removed-bg": rgba(theme.gitDeleted, isLight ? 0.12 : 0.11),
      "--added-word": rgba(theme.gitAdded, isLight ? 0.3 : 0.28),
      "--removed-word": rgba(theme.gitDeleted, isLight ? 0.3 : 0.3),
      "--selection": rgba(theme.accent, isLight ? 0.18 : 0.22),
      "--warning": theme.yellow,
      "--danger": theme.red,
    ]
  }
}

/// Avoids a retain cycle through WKUserContentController.
private final class WeakScriptHandler: NSObject, WKScriptMessageHandler {
  weak var target: WKScriptMessageHandler?
  init(_ target: WKScriptMessageHandler) { self.target = target }
  func userContentController(
    _ controller: WKUserContentController, didReceive message: WKScriptMessage
  ) {
    target?.userContentController(controller, didReceive: message)
  }
}

// MARK: - Header

/// Native header: scope, layout, whitespace, progress, comment actions.
struct ReviewHeaderBar: View {
  var model: ReviewSurfaceModel

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

      HStack(spacing: 0) {
        segment("Unified", selected: model.options.layout == "unified") {
          model.onSetLayout?("unified")
        }
        segment("Split", selected: model.options.layout == "split") {
          model.onSetLayout?("split")
        }
      }
      .padding(2)
      .background(
        RoundedRectangle(cornerRadius: Metrics.radius, style: .continuous).fill(chrome.raised))

      ChromeIconButton(
        icon: .space,
        help: model.options.ignoreWhitespace ? "Show whitespace changes" : "Hide whitespace changes",
        isActive: model.options.ignoreWhitespace
      ) { model.onToggleWhitespace?() }

      ChromeMenuButton(help: "Review comments") {
        [
          ChromeMenuItem("Copy Comments as Prompt", isEnabled: model.commentCount > 0) {
            model.onCopyPrompt?()
          },
          .separator,
          ChromeMenuItem("Delete All Comments…", isEnabled: model.commentCount > 0) {
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

  private func segment(_ title: String, selected: Bool, action: @escaping () -> Void) -> some View
  {
    let chrome = model.palette
    return Button(action: action) {
      Text(title)
        .font(ChromeFont.ui(11, weight: selected ? .semibold : .regular))
        .foregroundStyle(selected ? chrome.text : chrome.textTertiary)
        .padding(.horizontal, 8)
        .frame(height: 20)
        .background(
          RoundedRectangle(cornerRadius: Metrics.radiusSmall, style: .continuous)
            .fill(selected ? chrome.content : .clear))
        .contentShape(Rectangle())
    }
    .buttonStyle(ChromePressStyle())
  }
}
