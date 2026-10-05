import AppKit
import ImageIO
import ImpulseKit

private func nonEmpty(_ value: String?) -> String? {
  guard let value else { return nil }
  let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
  return trimmed.isEmpty ? nil : trimmed
}

private func loadImagePreview(path: String, maxPixelSize: Int = 4096) -> NSImage? {
  let url = URL(fileURLWithPath: path) as CFURL
  guard let source = CGImageSourceCreateWithURL(url, nil) else { return nil }
  let options: [CFString: Any] = [
    kCGImageSourceCreateThumbnailFromImageAlways: true,
    kCGImageSourceCreateThumbnailWithTransform: true,
    kCGImageSourceShouldCacheImmediately: false,
    kCGImageSourceThumbnailMaxPixelSize: maxPixelSize,
  ]
  guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else {
    return nil
  }
  return NSImage(cgImage: image, size: NSSize(width: image.width, height: image.height))
}

// MARK: - Tab Info

/// Lightweight snapshot of the active tab's state for the status bar.
struct TabInfo {
  var cwd: String?
  var gitBranch: String?
  var shellName: String?
  var cursorLine: Int?
  var cursorCol: Int?
  var language: String?
  var encoding: String?
  var indentInfo: String?
}

// MARK: - Tab Entry

/// Discriminated union representing either a terminal, editor, or image preview tab.
/// Stores the NSView and the metadata needed for display in the tab bar.
enum TabEntry {
  case terminal(TerminalContainer)
  case editor(EditorTab)
  case imagePreview(path: String, view: NSView)
  case diffReview(repoRoot: String, view: ReviewSurface)
  /// Several of the above side by side (never nested).
  case split(SplitTab)

  /// The view to display in the content area.
  var view: NSView {
    switch self {
    case .terminal(let container): return container
    case .editor(let editor): return editor
    case .imagePreview(_, let view): return view
    case .diffReview(_, let view): return view
    case .split(let split): return split.view
    }
  }

  var isSplit: Bool {
    if case .split = self { return true }
    return false
  }

  /// The surface that has focus: the focused pane of a split, else itself.
  var focused: TabEntry {
    if case .split(let split) = self { return split.focused }
    return self
  }

  /// Every surface in the tab, in reading order.
  var surfaces: [TabEntry] {
    if case .split(let split) = self { return split.orderedPanes.map(\.entry) }
    return [self]
  }

  /// The title to show in the tab bar segment.
  var title: String {
    switch self {
    case .split(let split):
      return split.focused.title
    case .terminal(let container):
      if let active = container.activeTerminal {
        let title = active.tabTitle
        return title.isEmpty ? LoginShell.defaultShellName() : title
      }
      return LoginShell.defaultShellName()
    case .editor(let editor):
      if let path = editor.filePath {
        let name = (path as NSString).lastPathComponent
        return editor.isModified ? "\(name) *" : name
      }
      return editor.isModified ? "Untitled *" : "Untitled"
    case .imagePreview(let path, _):
      return (path as NSString).lastPathComponent
    case .diffReview(let repoRoot, _):
      let name = (repoRoot as NSString).lastPathComponent
      return name.isEmpty ? "Review" : "Review · \(name)"
    }
  }

  /// Whether this tab should display an attention indicator.
  var needsAttention: Bool {
    switch self {
    case .terminal(let container):
      return container.needsAttention
    case .split(let split):
      return split.panes.values.contains { $0.needsAttention }
    case .editor, .imagePreview, .diffReview:
      return false
    }
  }

  /// Extracts a `TabInfo` snapshot for the status bar.
  var info: TabInfo {
    switch self {
    case .split(let split):
      return split.focused.info
    case .terminal(let container):
      return TabInfo(
        cwd: container.activeTerminal?.currentWorkingDirectory,
        gitBranch: nil,
        shellName: LoginShell.defaultShellName(),
        cursorLine: nil, cursorCol: nil,
        language: nil, encoding: nil, indentInfo: nil
      )
    case .editor(let editor):
      return TabInfo(
        cwd: editor.projectDirectory
          ?? editor.filePath.map { ($0 as NSString).deletingLastPathComponent },
        gitBranch: nil,
        shellName: nil,
        cursorLine: nil, cursorCol: nil,
        language: editor.language,
        encoding: "UTF-8",
        indentInfo: nil
      )
    case .imagePreview(let path, _):
      return TabInfo(
        cwd: path,
        gitBranch: nil,
        shellName: nil,
        cursorLine: nil, cursorCol: nil,
        language: "Image",
        encoding: nil,
        indentInfo: nil
      )
    case .diffReview(let repoRoot, _):
      return TabInfo(
        cwd: repoRoot,
        gitBranch: nil,
        shellName: nil,
        cursorLine: nil, cursorCol: nil,
        language: nil,
        encoding: nil,
        indentInfo: nil
      )
    }
  }

  /// Focus the primary interactive view.
  func focus() {
    switch self {
    case .split(let split):
      split.focused.focus()
    case .terminal(let container):
      container.activeTerminal?.focus()
    case .editor(let editor):
      editor.focus()
    case .imagePreview:
      break
    case .diffReview(_, let view):
      view.focus()
    }
  }

  /// Apply a new theme.
  func applyTheme(_ theme: Theme) {
    switch self {
    case .split(let split):
      split.applyPalette(ChromePalette(theme: theme))
      for pane in split.panes.values { pane.applyTheme(theme) }
    case .terminal(let container):
      let termTheme = TerminalTheme(
        bg: theme.terminalBg,
        fg: theme.terminalFg,
        selection: theme.selection,
        cursor: theme.cursor,
        border: theme.border,
        fgMuted: theme.fgMuted,
        accent: theme.accent,
        red: theme.red,
        terminalPalette: theme.terminalPalette
      )
      container.applyTheme(theme: termTheme, dividerColor: theme.bgHighlightColor)
      container.setInputBarColors(background: theme.bgDarkColor, border: theme.borderColor)
    case .editor(let editor):
      editor.applyTheme(ThemeManager.monacoTheme(forName: theme.id))
    case .imagePreview(_, let view):
      view.layer?.backgroundColor = theme.bgColor.cgColor
    case .diffReview(_, let view):
      view.applyTheme(theme)
    }
  }
}

// MARK: - Closed Tab Info

/// A closed tab or pane that "Reopen Closed Tab" can bring back: files
/// reopen where they were, terminals get a new shell in the same folder,
/// split tabs come back with their layout.
struct ClosedTabInfo {
  let tab: SessionTab
  let workspaceID: UUID
  /// When a single pane closed: the tab it was in, so it can rejoin it.
  let fromTabUID: Int?
}

// MARK: - Tab Manager

/// Manages the collection of open tabs (terminal or editor) and the
/// segmented control used to switch between them. The segmented control is
/// placed in the window's titlebar container.
final class TabManager: NSObject {
  /// One open tab and its bookkeeping.
  private struct TabRecord {
    var entry: TabEntry
    var pinned = false
    /// Stable id (survives reorders); SwiftUI tracks tabs by it.
    let uid: Int
    /// The tab this one was opened from, selected again when it closes.
    var closeReturnUID: Int?
    var workspaceID: UUID
  }

  private var records: [TabRecord] = []
  private var nextTabUID = 0

  /// Every open tab in strip order, across all workspaces. Indexes into this
  /// array are the tab indexes used everywhere (selection, close, move).
  var tabs: [TabEntry] { records.map(\.entry) }
  var pinnedTabs: [Bool] { records.map(\.pinned) }

  /// The window's workspaces, in sidebar order. Never empty.
  private(set) var workspaces: [Workspace]
  private(set) var activeWorkspaceID: UUID
  /// Workspaces from most to least recently active.
  private var workspaceHistory: [UUID] = []

  /// Set of file paths currently open in editor/image tabs for O(1) deduplication.
  private var openFilePaths: Set<String> = []

  /// Stack of recently closed tabs for "reopen closed tab" (Cmd+Shift+T).
  private(set) var closedTabs: [ClosedTabInfo] = []

  /// Maximum number of closed tabs to remember.
  private let maxClosedTabs = 20

  /// The index of the currently selected tab, or -1 if no tabs are open.
  private(set) var selectedIndex: Int = -1

  /// The custom tab bar displayed in the titlebar for tab switching.
  // CustomTabBar removed — SwiftUI TabBarView reads from windowModel

  /// Icon cache for themed file icons in tab bar.
  private(set) var iconCache: IconCache?

  /// Short-lived git branch cache for vertical tab subtitles, keyed by
  /// directory. Entries expire after 15 seconds so branch switches show up.
  private var tabBranchCache: [String: (branch: String, at: Date)] = [:]
  private var tabBranchPending: Set<String> = []

  /// The container view that hosts the active tab's view.
  let contentView: NSView

  /// Backed by `SettingsStore.shared` (no private copy to keep in sync).
  var settings: Settings {
    get { SettingsStore.shared.settings }
    set { SettingsStore.shared.settings = newValue }
  }
  private var theme: Theme
  private let core: ImpulseCore

  /// Observable state for SwiftUI views. Set by MainWindowController.
  weak var windowModel: WindowModel?

  /// Optional callback invoked instead of directly closing a tab. When set,
  /// the caller (MainWindowController) can show a save confirmation dialog
  /// for unsaved editor tabs.
  var tabCloseHandler: ((Int) -> Void)?

  /// Returns a `TabInfo` snapshot for the currently active tab, or `nil` if
  /// no tabs are open.
  var activeTabInfo: TabInfo? {
    guard records.indices.contains(selectedIndex) else { return nil }
    return records[selectedIndex].entry.info
  }

  init(theme: Theme, core: ImpulseCore) {
    self.theme = theme
    self.core = core
    let scratch = Workspace(kind: .scratch, root: NSHomeDirectory())
    workspaces = [scratch]
    activeWorkspaceID = scratch.id

    iconCache = IconCache(theme: theme)

    contentView = NSView()
    contentView.wantsLayer = true
    contentView.layer?.backgroundColor = theme.bgColor.cgColor

    super.init()
  }

  // MARK: - Adding Tabs

  /// Creates a new terminal tab and makes it active. Without a directory it
  /// starts in the active workspace's folder.
  func addTerminalTab(directory: String? = nil, initialCommand: String? = nil) {
    let directory = directory ?? activeWorkspace.defaultDirectory
    insertTab(.terminal(makeTerminalContainer(directory: directory, initialCommand: initialCommand)))
  }

  /// A new terminal surface (shell spawns once it's laid out).
  func makeTerminalContainer(directory: String?, initialCommand: String? = nil)
    -> TerminalContainer
  {
    let dir = directory ?? NSHomeDirectory()
    let container = TerminalContainer(
      frame: NSRect(x: 0, y: 0, width: 800, height: 600),
      settings: settings.terminalSettings(directory: dir),
      theme: terminalTheme,
      initialCommand: initialCommand
    )
    container.applyTheme(theme: terminalTheme, dividerColor: theme.bgHighlightColor)
    container.setInputBarColors(background: theme.bgDarkColor, border: theme.borderColor)
    return container
  }

  func addRestoredTerminalTab(_ tab: SessionTabState) {
    let dir = nonEmpty(tab.cwd) ?? NSHomeDirectory()
    let container = TerminalContainer(
      frame: NSRect(x: 0, y: 0, width: 800, height: 600),
      settings: settings.terminalSettings(directory: dir),
      theme: terminalTheme,
      sessionTab: tab
    )
    container.applyTheme(theme: terminalTheme, dividerColor: theme.bgHighlightColor)
    container.setInputBarColors(background: theme.bgDarkColor, border: theme.borderColor)
    insertTab(.terminal(container))
  }

  private var terminalTheme: TerminalTheme {
    TerminalTheme(
      bg: theme.terminalBg,
      fg: theme.terminalFg,
      selection: theme.selection,
      cursor: theme.cursor,
      border: theme.border,
      fgMuted: theme.fgMuted,
      accent: theme.accent,
      red: theme.red,
      terminalPalette: theme.terminalPalette
    )
  }

  /// Creates a new editor tab for the given file path.
  ///
  /// If a tab for the same file is already open, it is selected instead of
  /// creating a duplicate. Image files are opened in a preview tab. Binary
  /// files (>10 MB or containing null bytes) are skipped with an alert.
  func addEditorTab(
    path: String, projectDirectory: String? = nil, goToLine: UInt32? = nil,
    goToColumn: UInt32? = nil, beside: Bool = false
  ) {
    // O(1) deduplication using the openFilePaths set.
    if openFilePaths.contains(path) {
      let location = locate {
        switch $0 {
        case .editor(let e): return e.filePath == path
        case .imagePreview(let p, _): return p == path
        default: return false
        }
      }
      if let location {
        updateCloseReturnTarget(forTabAt: location.tabIndex, sourceIndex: selectedIndex)
        reveal(location)
        // Navigate to position in the already-open editor.
        if let line = goToLine, let column = goToColumn,
          case .editor(let editor) = tabs[location.tabIndex].focused
        {
          editor.goToPosition(line: line, column: column)
        }
      }
      return
    }

    // Image files get a preview tab instead of an editor.
    if Self.isImageFile(path) {
      addImagePreviewTab(path: path, beside: beside)
      return
    }

    // Reject binary files.
    if Self.isBinaryFile(path) {
      let alert = NSAlert()
      alert.messageText = "Binary File"
      alert.informativeText =
        "The file \"\((path as NSString).lastPathComponent)\" appears to be a binary file and cannot be opened in the editor."
      alert.alertStyle = .informational
      alert.addButton(withTitle: "OK")
      alert.runModal()
      return
    }

    // Read file content off the main thread, then create the editor tab on main.
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let fileContent = (try? String(contentsOfFile: path, encoding: .utf8)) ?? ""
      let largeFile =
        (try? FileManager.default.attributesOfItem(atPath: path)[.size] as? Int).flatMap({ $0 })
        ?? 0 > 5 * 1024 * 1024

      DispatchQueue.main.async { [weak self] in
        guard let self else { return }

        // Re-check deduplication in case a tab was opened while reading.
        if self.openFilePaths.contains(path) { return }
        self.insertLoadedEditorTab(
          path: path, content: fileContent, largeFile: largeFile,
          projectDirectory: projectDirectory, goToLine: goToLine, goToColumn: goToColumn,
          beside: beside)
      }
    }
  }

  /// Creates and inserts an editor tab for a file whose content was already
  /// read (off the main thread). Used by `addEditorTab` and by session restore,
  /// which preloads every file first so tabs can be inserted in saved order.
  func insertLoadedEditorTab(
    path: String, content fileContent: String, largeFile: Bool,
    projectDirectory: String?, goToLine: UInt32? = nil, goToColumn: UInt32? = nil,
    beside: Bool = false
  ) {
    guard !openFilePaths.contains(path) else { return }
    let editorTab = makeEditorTab(
      path: path, content: fileContent, largeFile: largeFile, projectDirectory: projectDirectory,
      goToLine: goToLine, goToColumn: goToColumn)
    if beside {
      splitSelectedTab(with: .editor(editorTab), axis: .horizontal)
    } else {
      insertTab(.editor(editorTab))
    }
  }

  /// A new editor surface for a file whose content was already read.
  func makeEditorTab(
    path: String, content fileContent: String, largeFile: Bool, projectDirectory: String?,
    goToLine: UInt32? = nil, goToColumn: UInt32? = nil
  ) -> EditorTab {
    let editorOptions = editorOptionsFromSettings()
    let themeDef = ThemeManager.monacoTheme(forName: theme.id)
    let language = languageIdForPath(path)

    let editorTab = EditorTab(frame: NSRect(x: 0, y: 0, width: 800, height: 600))
    editorTab.projectDirectory =
      projectDirectory
      ?? (path as NSString).deletingLastPathComponent
    editorTab.openFile(path: path, content: fileContent, language: language)
    editorTab.loadEditor()

    // Apply editor settings (font, tab size, etc.) from the current settings.
    editorTab.applySettings(editorOptions)
    editorTab.applyTheme(themeDef)

    // Open large files in read-only mode to avoid WebView freezes.
    if largeFile {
      editorTab.setReadOnly(true)
    }

    // Queue go-to-position; pendingCommands will flush after Monaco fires Ready.
    if let line = goToLine, let column = goToColumn {
      editorTab.goToPosition(line: line, column: column)
    }
    return editorTab
  }

  /// Creates a new untitled editor tab with no file on disk.
  func addUntitledEditorTab(cwd: String?) {
    let editorTab = EditorTab(frame: NSRect(x: 0, y: 0, width: 800, height: 600))
    editorTab.untitledCwd = cwd
    editorTab.projectDirectory = cwd
    editorTab.openBlank()
    editorTab.loadEditor()

    let editorOptions = editorOptionsFromSettings()
    editorTab.applySettings(editorOptions)
    let themeDef = ThemeManager.monacoTheme(forName: theme.id)
    editorTab.applyTheme(themeDef)

    let entry = TabEntry.editor(editorTab)
    insertTab(entry)
    // filePath is nil, so insertTab won't add to openFilePaths — correct.
  }

  /// Register a file path in the open-file dedup set (e.g. after save-as).
  func registerOpenFilePath(_ path: String) {
    openFilePaths.insert(path)
  }

  /// Opens the Review Changes tab for `repoRoot`. Review tabs are per-repo: if
  /// one already exists for this exact `repoRoot` it is selected, focused, and
  /// reloaded; otherwise a new tab is created. A different repo gets its own tab
  /// so the user never reviews/commits/discards against a stale repository.
  /// Open (or reuse) the review tab for a repository, optionally switching
  /// its scope and scrolling to a file.
  func addReviewTab(
    repository: GitRepositoryState, scope: DiffScope? = nil, focusPath: String? = nil,
    host: GitPanelHost?
  ) {
    if let location = locate(where: {
      if case .diffReview(let r, _) = $0 { return r == repository.root }
      return false
    }) {
      reveal(location)
      if case .diffReview(_, let view) = tabs[location.tabIndex].focused {
        view.show(scope: scope, focusPath: focusPath)
        view.refresh()
        view.focus()
      }
      return
    }
    let review = ReviewSurface(
      repository: repository, scope: scope, focusPath: focusPath, theme: theme, host: host)
    insertTab(TabEntry.diffReview(repoRoot: repository.root, view: review))
  }

  /// Detect the Monaco language ID for a file path.
  func detectLanguage(forPath path: String) -> String {
    languageIdForPath(path)
  }

  /// Creates an image preview tab that scales large images to fit.
  private func addImagePreviewTab(path: String, beside: Bool = false) {
    let entry = makeImagePreview(path: path)
    if beside {
      splitSelectedTab(with: entry, axis: .horizontal)
    } else {
      insertTab(entry)
    }
  }

  /// A new image preview surface.
  func makeImagePreview(path: String) -> TabEntry {
    let container = NSView()
    container.wantsLayer = true

    let imageView = NSImageView()
    imageView.image = loadImagePreview(path: path)
    imageView.imageScaling = .scaleProportionallyDown
    imageView.imageAlignment = .alignCenter
    imageView.translatesAutoresizingMaskIntoConstraints = false
    // Prevent the image's natural size from expanding the container.
    imageView.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
    imageView.setContentCompressionResistancePriority(.defaultLow, for: .vertical)
    imageView.setContentHuggingPriority(.defaultLow, for: .horizontal)
    imageView.setContentHuggingPriority(.defaultLow, for: .vertical)

    container.addSubview(imageView)

    NSLayoutConstraint.activate([
      imageView.topAnchor.constraint(equalTo: container.topAnchor, constant: 20),
      imageView.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 20),
      imageView.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -20),
      imageView.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -20),
    ])

    container.layer?.backgroundColor = theme.bgColor.cgColor
    return TabEntry.imagePreview(path: path, view: container)
  }

  /// Inserts a new tab after the currently selected tab and selects it.
  /// With no selection in the workspace, it goes after the workspace's last
  /// tab.
  private func insertTab(_ entry: TabEntry) {
    let workspaceID = activeWorkspaceID
    let insertionIndex: Int
    if records.indices.contains(selectedIndex), records[selectedIndex].workspaceID == workspaceID {
      if records[selectedIndex].pinned {
        // Selected tab is pinned — insert after the workspace's last pinned
        // tab so new tabs never land between pinned and unpinned sections.
        let lastPinned = records.indices.last {
          records[$0].pinned && records[$0].workspaceID == workspaceID
        }
        insertionIndex = (lastPinned ?? selectedIndex) + 1
      } else {
        insertionIndex = selectedIndex + 1
      }
    } else {
      let last = records.indices.last { records[$0].workspaceID == workspaceID }
      insertionIndex = last.map { $0 + 1 } ?? records.count
    }
    let returnUID = records.indices.contains(selectedIndex) ? records[selectedIndex].uid : nil
    records.insert(
      TabRecord(entry: entry, uid: nextTabUID, closeReturnUID: returnUID, workspaceID: workspaceID),
      at: insertionIndex)
    nextTabUID += 1
    if selectedIndex >= insertionIndex { selectedIndex += 1 }

    track(entry)

    // No rebuildSegments() here: selectTab() syncs to the WindowModel, and
    // syncing before selection would push the transient (selectedIndex not
    // yet updated) state to SwiftUI — and trip the consistency assert on the
    // very first tab insert in debug builds.
    selectTab(index: insertionIndex)
  }

  private func updateCloseReturnTarget(forTabAt index: Int, sourceIndex: Int) {
    guard records.indices.contains(index), records.indices.contains(sourceIndex),
      index != sourceIndex
    else { return }
    records[index].closeReturnUID = records[sourceIndex].uid
  }

  // MARK: - Removing Tabs

  /// Release resources owned by a tab entry (kill processes, tear down
  /// WebViews) so they don't linger after the tab is removed.
  private func cleanupTab(_ entry: TabEntry) {
    switch entry {
    case .split(let split):
      for pane in split.panes.values { cleanupTab(pane) }
    case .terminal(let container):
      container.terminateAllProcesses()
    case .editor(let editor):
      editor.cleanup()
    case .imagePreview:
      break
    case .diffReview(_, let view):
      view.cleanup()
    }
  }

  /// Records a closing tab (or pane) for "Reopen Closed Tab".
  private func recordClosedTab(
    _ entry: TabEntry, pinned: Bool = false, workspaceID: UUID, fromTabUID: Int? = nil
  ) {
    guard let tab = sessionTab(for: entry, pinned: pinned) else { return }
    closedTabs.append(ClosedTabInfo(tab: tab, workspaceID: workspaceID, fromTabUID: fromTabUID))
    if closedTabs.count > maxClosedTabs {
      closedTabs.removeFirst()
    }
  }

  /// Closes the tab at the given index. If it is the active tab, the tab that
  /// opened it is selected when possible, otherwise the nearest neighbor in
  /// its workspace. A workspace closes with its last tab while others remain;
  /// the last workspace gets a fresh terminal instead.
  func closeTab(index: Int) {
    guard records.indices.contains(index) else { return }

    let record = records[index]
    let closingSelectedTab = index == selectedIndex
    recordClosedTab(record.entry, pinned: record.pinned, workspaceID: record.workspaceID)
    cleanupTab(record.entry)
    untrack(record.entry)

    // Remove the tab's view from the content area if it is currently displayed.
    if closingSelectedTab {
      record.entry.view.removeFromSuperview()
      selectedIndex = -1
    } else if index < selectedIndex {
      selectedIndex -= 1
    }
    records.remove(at: index)

    let siblings = records.indices.filter { records[$0].workspaceID == record.workspaceID }
    if siblings.isEmpty {
      if workspaces.count > 1 {
        removeWorkspace(record.workspaceID)
      } else {
        // Keep the window from ever being empty.
        addTerminalTab()
      }
      return
    }

    guard closingSelectedTab else {
      rebuildSegments()
      return
    }

    if let returnUID = record.closeReturnUID,
      let returnIndex = siblings.first(where: { records[$0].uid == returnUID })
    {
      selectTab(index: returnIndex)
      return
    }

    // Select the nearest tab in the same workspace.
    selectTab(index: siblings.first { $0 >= index } ?? siblings[siblings.count - 1])
  }

  /// Clean up all tabs (kill processes, tear down WebViews). Called when the
  /// window closes to ensure nothing lingers.
  func cleanupAllTabs() {
    for tab in tabs {
      cleanupTab(tab)
    }
  }

  /// Toggles the pinned state of the tab at the given index.
  func togglePin(index: Int) {
    guard records.indices.contains(index) else { return }
    records[index].pinned.toggle()
    refreshSegmentLabels()
  }

  /// Sets the pinned state of the tab at the given index (session restore).
  func setPinned(_ pinned: Bool, index: Int) {
    guard records.indices.contains(index), records[index].pinned != pinned else { return }
    records[index].pinned = pinned
    refreshSegmentLabels()
  }

  /// Unpins the tab at the given index (used before closing a pinned tab).
  func unpin(index: Int) {
    guard records.indices.contains(index) else { return }
    records[index].pinned = false
    refreshSegmentLabels()
  }

  // MARK: - Reopening Closed Tabs

  /// Reopens the most recently closed tab or pane. A pane rejoins its old
  /// tab when that tab is still open.
  func reopenLastClosedTab() {
    guard let info = closedTabs.popLast() else { return }
    let paths = info.tab.panes.compactMap(\.path)
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let contents = Self.preloadFileContents(paths)
      DispatchQueue.main.async { self?.insertReopened(info, contents: contents) }
    }
  }

  private func insertReopened(
    _ info: ClosedTabInfo, contents: [String: (text: String, large: Bool)]
  ) {
    let target = workspace(info.workspaceID) ?? activeWorkspace
    if target.id != activeWorkspaceID { activateWorkspace(target.id) }
    var panes: [Int: TabEntry] = [:]
    for (id, surface) in info.tab.panes.enumerated() {
      if let entry = makeRestoredSurface(
        surface, contents: contents,
        projectDirectory: target.kind == .folder ? target.root : nil)
      {
        panes[id] = entry
      }
    }
    guard !panes.isEmpty else {
      // A file that's open again: show it instead.
      if let path = info.tab.panes.first?.path { addEditorTab(path: path) }
      return
    }
    if panes.count == 1, let pane = panes.values.first, let uid = info.fromTabUID,
      let index = records.firstIndex(where: { $0.uid == uid && $0.workspaceID == target.id })
    {
      selectTab(index: index)
      splitSelectedTab(with: pane, axis: .horizontal)
      return
    }
    if let index = appendRestoredTab(
      panes: panes, layout: info.tab.layout, focusedPane: info.tab.focusedPane,
      pinned: info.tab.pinned, workspaceID: target.id)
    {
      selectTab(index: index)
    }
  }

  /// Read files for restored editors (off the main thread): text and
  /// whether it's large enough to open read-only. Skips images and binaries.
  static func preloadFileContents(_ paths: [String]) -> [String: (text: String, large: Bool)] {
    var contents: [String: (text: String, large: Bool)] = [:]
    for path in paths where !isImageFile(path) {
      guard FileManager.default.fileExists(atPath: path), !isBinaryFile(path) else { continue }
      let size =
        (try? FileManager.default.attributesOfItem(atPath: path)[.size] as? Int).flatMap { $0 }
        ?? 0
      let text = (try? String(contentsOfFile: path, encoding: .utf8)) ?? ""
      contents[path] = (text, size > 5 * 1024 * 1024)
    }
    return contents
  }

  // MARK: - Reordering

  /// Moves a tab from one index to another, preserving pinned state and
  /// updating selection to follow the moved tab.
  func moveTab(from sourceIndex: Int, to destinationIndex: Int) {
    guard sourceIndex != destinationIndex,
      records.indices.contains(sourceIndex), records.indices.contains(destinationIndex)
    else { return }

    let record = records.remove(at: sourceIndex)
    records.insert(record, at: destinationIndex)

    // Track the moved tab's new position.
    if selectedIndex == sourceIndex {
      selectedIndex = destinationIndex
    } else if sourceIndex < selectedIndex && destinationIndex >= selectedIndex {
      selectedIndex -= 1
    } else if sourceIndex > selectedIndex && destinationIndex <= selectedIndex {
      selectedIndex += 1
    }

    rebuildSegments()
  }

  // MARK: - Selection

  /// Switches the visible tab to the one at `index`.
  func selectTab(index: Int) {
    guard records.indices.contains(index) else { return }

    // Remove the previous tab's view.
    if records.indices.contains(selectedIndex) {
      records[selectedIndex].entry.view.removeFromSuperview()
    }

    // Selecting another workspace's tab switches to that workspace.
    let workspaceChanged = records[index].workspaceID != activeWorkspaceID
    if workspaceChanged { noteWorkspaceActivated(records[index].workspaceID) }
    selectedIndex = index
    activeWorkspace.lastSelectedUID = records[index].uid
    if case .terminal(let container) = records[index].entry.focused {
      container.activeTerminal?.clearAttention()
    }
    syncToWindowModel()

    // Activate the new tab.
    let entry = records[index].entry
    install(entry.view)
    activateKeyboardFocus(entry.focused)

    if workspaceChanged {
      NotificationCenter.default.post(name: .impulseActiveWorkspaceDidChange, object: self)
    }
    NotificationCenter.default.post(name: .impulseActiveTabDidChange, object: self)
  }

  /// Show a tab's view in the content area.
  private func install(_ view: NSView) {
    view.removeFromSuperview()
    view.translatesAutoresizingMaskIntoConstraints = false
    contentView.addSubview(view, positioned: .below, relativeTo: nil)
    NSLayoutConstraint.activate([
      view.topAnchor.constraint(equalTo: contentView.topAnchor),
      view.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
      view.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
      view.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
    ])
  }

  /// Give a surface the keyboard.
  ///
  /// Terminals are normally driven by the input bar (the read-only grid
  /// refuses first-responder), so focus moves there. But when the terminal is
  /// already running a TUI (vim, Claude Code), the input bar is hidden and the
  /// grid owns the keyboard — keep focus on the grid instead, or the token
  /// bump would steal the first keystrokes (and image paste) into the
  /// about-to-disappear bar. Async either way so it lands after the new
  /// terminal's own focus attempt during spawn.
  private func activateKeyboardFocus(_ surface: TabEntry) {
    surface.focus()
    if case .terminal(let container) = surface {
      let model = windowModel
      if container.activeTerminal?.wantsGridFocus == true {
        DispatchQueue.main.async { container.activeTerminal?.focus() }
      } else {
        DispatchQueue.main.async { model?.inputBarFocusToken += 1 }
      }
    }
  }

  // MARK: - Workspaces

  var activeWorkspace: Workspace {
    workspaces.first { $0.id == activeWorkspaceID } ?? workspaces[0]
  }

  func workspace(_ id: UUID) -> Workspace? {
    workspaces.first { $0.id == id }
  }

  /// Indexes of a workspace's tabs, in strip order.
  func tabIndices(inWorkspace id: UUID) -> [Int] {
    records.indices.filter { records[$0].workspaceID == id }
  }

  /// Indexes of the tabs the strip shows (the active workspace's).
  var visibleTabIndices: [Int] { tabIndices(inWorkspace: activeWorkspaceID) }

  func workspaceID(ofTabAt index: Int) -> UUID? {
    records.indices.contains(index) ? records[index].workspaceID : nil
  }

  /// Show a workspace: its last selected tab, or a new terminal in its
  /// folder when it has none.
  func activateWorkspace(_ id: UUID) {
    guard id != activeWorkspaceID, let workspace = workspace(id) else { return }
    let indices = tabIndices(inWorkspace: id)
    let remembered = workspace.lastSelectedUID.flatMap { uid in
      indices.first { records[$0].uid == uid }
    }
    if let index = remembered ?? indices.first {
      selectTab(index: index)
      return
    }
    if records.indices.contains(selectedIndex) {
      records[selectedIndex].entry.view.removeFromSuperview()
    }
    selectedIndex = -1
    noteWorkspaceActivated(id)
    NotificationCenter.default.post(name: .impulseActiveWorkspaceDidChange, object: self)
    addTerminalTab()
  }

  /// Open a folder as a workspace (or show it if it's already open).
  @discardableResult
  func openWorkspace(folder: String) -> Workspace {
    let root = Workspace.normalize(folder)
    if let existing = workspaces.first(where: { $0.kind == .folder && $0.root == root }) {
      activateWorkspace(existing.id)
      return existing
    }
    let workspace = Workspace(kind: .folder, root: root)
    addWorkspace(workspace)
    RecentWorkspaces.note(root)
    activateWorkspace(workspace.id)
    return workspace
  }

  /// Add a workspace without showing it (session restore).
  func addWorkspace(_ workspace: Workspace) {
    workspaces.append(workspace)
    resolveRepository(for: workspace)
    syncToWindowModel()
  }

  /// Close a workspace and every tab in it (callers confirm first). The
  /// last workspace stays, with a fresh terminal.
  func closeWorkspace(_ id: UUID) {
    guard workspace(id) != nil else { return }
    for index in tabIndices(inWorkspace: id).reversed() {
      let record = records[index]
      recordClosedTab(record.entry, pinned: record.pinned, workspaceID: record.workspaceID)
      cleanupTab(record.entry)
      untrack(record.entry)
      if index == selectedIndex {
        record.entry.view.removeFromSuperview()
        selectedIndex = -1
      } else if index < selectedIndex {
        selectedIndex -= 1
      }
      records.remove(at: index)
    }
    if workspaces.count > 1 {
      removeWorkspace(id)
    } else {
      addTerminalTab()
    }
  }

  func renameWorkspace(_ id: UUID, to name: String?) {
    guard let workspace = workspace(id) else { return }
    let trimmed = name?.trimmingCharacters(in: .whitespacesAndNewlines)
    workspace.customName = (trimmed?.isEmpty ?? true) ? nil : trimmed
    syncToWindowModel()
  }

  func setWorkspaceExpanded(_ id: UUID, _ expanded: Bool) {
    guard let workspace = workspace(id), workspace.isExpanded != expanded else { return }
    workspace.isExpanded = expanded
    syncToWindowModel()
  }

  /// Move workspace `id` before the one at `index` in sidebar order.
  func moveWorkspace(_ id: UUID, to index: Int) {
    guard let from = workspaces.firstIndex(where: { $0.id == id }) else { return }
    let workspace = workspaces.remove(at: from)
    workspaces.insert(workspace, at: max(0, min(index, workspaces.count)))
    syncToWindowModel()
  }

  /// Drop a workspace whose tabs are gone, then show the most recently
  /// used remaining one.
  private func removeWorkspace(_ id: UUID) {
    workspaces.removeAll { $0.id == id }
    workspaceHistory.removeAll { $0 == id }
    if workspaces.isEmpty {
      workspaces = [Workspace(kind: .scratch, root: NSHomeDirectory())]
    }
    guard activeWorkspaceID == id else {
      syncToWindowModel()
      return
    }
    let nextID =
      workspaceHistory.first { candidate in workspaces.contains { $0.id == candidate } }
      ?? workspaces[0].id
    guard let next = workspace(nextID) else { return }
    let indices = tabIndices(inWorkspace: nextID)
    let remembered = next.lastSelectedUID.flatMap { uid in indices.first { records[$0].uid == uid } }
    if let index = remembered ?? indices.first {
      selectTab(index: index)
    } else {
      noteWorkspaceActivated(nextID)
      NotificationCenter.default.post(name: .impulseActiveWorkspaceDidChange, object: self)
      addTerminalTab()
    }
  }

  private func noteWorkspaceActivated(_ id: UUID) {
    activeWorkspaceID = id
    workspaceHistory.removeAll { $0 == id }
    workspaceHistory.insert(id, at: 0)
  }

  private func resolveRepository(for workspace: Workspace) {
    guard workspace.kind == .folder else { return }
    GitRepositoryStore.shared.resolve(directory: workspace.root) { [weak self, weak workspace] state in
      guard let workspace, let state else { return }
      workspace.repository = state
      self?.syncToWindowModel()
    }
  }

  // MARK: - Session

  /// The scratch workspace, if the window has one.
  var scratchWorkspace: Workspace? { workspaces.first { $0.kind == .scratch } }

  /// A surface restored from a session, or nil if it can't be (a missing
  /// file, a file whose content wasn't preloaded).
  func makeRestoredSurface(
    _ surface: SessionSurface, contents: [String: (text: String, large: Bool)],
    projectDirectory: String?
  ) -> TabEntry? {
    switch surface.kind {
    case "terminal":
      let cwd = surface.cwd.flatMap(nonEmpty) ?? activeWorkspace.defaultDirectory
      let container = TerminalContainer(
        frame: NSRect(x: 0, y: 0, width: 800, height: 600),
        settings: settings.terminalSettings(directory: cwd),
        theme: terminalTheme,
        sessionTab: .terminal(cwd: cwd, title: surface.title, shell: surface.shell, pinned: false),
        restoredTranscript: settings.restoreScrollback ? surface.transcript : nil
      )
      container.applyTheme(theme: terminalTheme, dividerColor: theme.bgHighlightColor)
      container.setInputBarColors(background: theme.bgDarkColor, border: theme.borderColor)
      return .terminal(container)
    case "file":
      guard let path = surface.path, !openFilePaths.contains(path),
        FileManager.default.fileExists(atPath: path)
      else { return nil }
      if Self.isImageFile(path) { return makeImagePreview(path: path) }
      guard let loaded = contents[path] else { return nil }
      let editor = makeEditorTab(
        path: path, content: loaded.text, largeFile: loaded.large,
        projectDirectory: projectDirectory,
        // Saved 1-based (as Monaco reports it); go-to positions are 0-based.
        goToLine: surface.line.map { UInt32(max(1, $0) - 1) },
        goToColumn: surface.column.map { UInt32(max(1, $0) - 1) })
      openFilePaths.insert(path)
      return .editor(editor)
    default:
      return nil
    }
  }

  /// Append a restored tab to a workspace without selecting it. `panes` are
  /// keyed by the saved layout's pane ids; missing panes drop out of the
  /// layout. Returns the tab's index, or nil when nothing survived.
  @discardableResult
  func appendRestoredTab(
    panes: [Int: TabEntry], layout: LayoutTree<Int>?, focusedPane: Int?, pinned: Bool,
    workspaceID: UUID
  ) -> Int? {
    let entry: TabEntry
    if let layout, panes.count > 1,
      let split = SplitTab(
        layout: layout, panes: panes, focusedPane: focusedPane ?? layout.leaves[0],
        palette: ChromePalette(theme: theme))
    {
      wire(split)
      entry = .split(split)
    } else if let id = layout?.leaves.first(where: { panes[$0] != nil }) ?? panes.keys.min(),
      let single = panes[id]
    {
      entry = single
    } else {
      return nil
    }
    track(entry)
    let last = records.indices.last { records[$0].workspaceID == workspaceID }
    let index = last.map { $0 + 1 } ?? records.count
    records.insert(
      TabRecord(entry: entry, pinned: pinned, uid: nextTabUID, workspaceID: workspaceID),
      at: index)
    nextTabUID += 1
    if selectedIndex >= index { selectedIndex += 1 }
    return index
  }

  /// After restoring: drop an empty scratch workspace (when others exist)
  /// and show the saved workspace and tab.
  func finishRestore(activeWorkspaceID targetID: UUID?, activeTabIndex: Int?) {
    if let scratch = scratchWorkspace, workspaces.count > 1,
      tabIndices(inWorkspace: scratch.id).isEmpty, scratch.id != targetID
    {
      workspaces.removeAll { $0.id == scratch.id }
      workspaceHistory.removeAll { $0 == scratch.id }
    }
    let target = targetID.flatMap(workspace) ?? workspaces[0]
    let indices = tabIndices(inWorkspace: target.id)
    if let activeTabIndex, indices.indices.contains(activeTabIndex) {
      target.lastSelectedUID = records[indices[activeTabIndex]].uid
    }
    if activeWorkspaceID != target.id || !records.indices.contains(selectedIndex) {
      // Force a full activation even if it's already the active id.
      if activeWorkspaceID == target.id { activeWorkspaceID = UUID() }
      activateWorkspace(target.id)
    }
  }

  /// A saveable description of a tab's surfaces and layout, or nil when
  /// nothing in it can be restored (review tabs, unsaved files).
  func sessionTab(for entry: TabEntry, pinned: Bool, withScrollback: Bool = true) -> SessionTab? {
    let shellName = LoginShell.defaultShellName()
    func surfaceState(_ entry: TabEntry) -> SessionSurface? {
      switch entry {
      case .terminal(let container):
        guard let terminal = container.activeTerminal else { return nil }
        let cwd = terminal.currentWorkingDirectory
        return .terminal(
          cwd: cwd.isEmpty ? NSHomeDirectory() : cwd, title: nonEmpty(terminal.tabTitle),
          shell: nonEmpty(shellName), transcript: withScrollback ? terminal.transcript() : nil)
      case .editor(let editor):
        guard let path = editor.filePath, FileManager.default.fileExists(atPath: path) else {
          return nil
        }
        return .file(
          path: path, line: editor.cursorPosition.map { Int($0.line) },
          column: editor.cursorPosition.map { Int($0.column) })
      case .imagePreview(let path, _):
        return FileManager.default.fileExists(atPath: path) ? .file(path: path) : nil
      case .diffReview, .split:
        return nil
      }
    }

    guard case .split(let split) = entry else {
      return surfaceState(entry).map { SessionTab(pinned: pinned, panes: [$0]) }
    }
    // Pane ids become indexes into `panes`; unsaveable panes drop out.
    var panes: [SessionSurface] = []
    var idMap: [Int: Int] = [:]
    for (id, pane) in split.orderedPanes {
      if let state = surfaceState(pane) {
        idMap[id] = panes.count
        panes.append(state)
      }
    }
    var layout: LayoutTree<Int>? = split.layout
    for id in split.layout.leaves where idMap[id] == nil { layout = layout?.removing(id) }
    guard let layout, !panes.isEmpty else { return nil }
    return SessionTab(
      pinned: pinned, panes: panes,
      layout: panes.count > 1 ? layout.mapPanes { idMap[$0] ?? $0 } : nil,
      focusedPane: idMap[split.focusedPane])
  }

  /// The window's workspaces and tabs for the session file.
  func sessionWorkspaces() -> (workspaces: [SessionWorkspaceState], activeIndex: Int?) {
    var result: [SessionWorkspaceState] = []
    for workspace in workspaces {
      var tabs: [SessionTab] = []
      var activeTab: Int?
      for index in tabIndices(inWorkspace: workspace.id) {
        let record = records[index]
        guard
          let tab = sessionTab(
            for: record.entry, pinned: record.pinned, withScrollback: settings.restoreScrollback)
        else { continue }
        if index == selectedIndex || (activeTab == nil && record.uid == workspace.lastSelectedUID) {
          activeTab = tabs.count
        }
        tabs.append(tab)
      }
      if workspace.kind == .scratch, tabs.isEmpty, workspaces.count > 1 { continue }
      result.append(
        SessionWorkspaceState(
          kind: workspace.kind.rawValue, root: workspace.root, name: workspace.customName,
          expanded: workspace.isExpanded ? true : nil, tabs: tabs, activeTabIndex: activeTab,
          fileTreeRoot: nil))
    }
    let activeIndex = result.firstIndex { state in
      state.kind == activeWorkspace.kind.rawValue && state.root == activeWorkspace.root
    }
    return (result, activeIndex)
  }

  // MARK: - Panes

  /// Where a surface lives: its tab, and its pane when the tab is split.
  struct SurfaceLocation {
    let tabIndex: Int
    let paneID: Int?
  }

  /// Every surface in every tab.
  var allSurfaces: [TabEntry] { tabs.flatMap(\.surfaces) }

  func locate(where predicate: (TabEntry) -> Bool) -> SurfaceLocation? {
    for (index, tab) in tabs.enumerated() {
      if case .split(let split) = tab {
        if let id = split.paneID(where: predicate) {
          return SurfaceLocation(tabIndex: index, paneID: id)
        }
      } else if predicate(tab) {
        return SurfaceLocation(tabIndex: index, paneID: nil)
      }
    }
    return nil
  }

  func location(of editor: EditorTab) -> SurfaceLocation? {
    locate {
      if case .editor(let candidate) = $0 { return candidate === editor }
      return false
    }
  }

  func location(ofTerminal terminal: TerminalTab) -> SurfaceLocation? {
    locate {
      if case .terminal(let container) = $0 { return container.terminals.contains { $0 === terminal } }
      return false
    }
  }

  /// Select the tab holding a surface and focus its pane.
  func reveal(_ location: SurfaceLocation) {
    if location.tabIndex != selectedIndex { selectTab(index: location.tabIndex) }
    if let id = location.paneID { focusPane(id, inTabAt: location.tabIndex) }
  }

  /// The split of the selected tab, if it has one.
  var selectedSplit: SplitTab? {
    if case .split(let split)? = selectedTab { return split }
    return nil
  }

  /// Put `entry` beside the selected tab's focused surface (turning the tab
  /// into a split if needed) and focus it.
  func splitSelectedTab(with entry: TabEntry, axis: SplitAxis, before: Bool = false) {
    guard selectedIndex >= 0, selectedIndex < tabs.count else {
      insertTab(entry)
      return
    }
    let index = selectedIndex
    track(entry)
    let split: SplitTab
    if case .split(let existing) = tabs[index] {
      split = existing
    } else {
      let current = tabs[index]
      current.view.removeFromSuperview()
      split = SplitTab(first: current, palette: ChromePalette(theme: theme))
      wire(split)
      records[index].entry = .split(split)
      install(split.view)
    }
    let id = split.insert(entry, beside: split.focusedPane, axis: axis, before: before)
    focusPane(id, inTabAt: index)
  }

  /// Focus one pane of a split tab.
  func focusPane(_ id: Int, inTabAt index: Int) {
    guard tabs.indices.contains(index), case .split(let split) = tabs[index] else { return }
    split.focus(id)
    guard index == selectedIndex else {
      syncToWindowModel()
      return
    }
    if case .terminal(let container) = split.focused {
      container.activeTerminal?.clearAttention()
    }
    syncToWindowModel()
    activateKeyboardFocus(split.focused)
    NotificationCenter.default.post(name: .impulseActiveTabDidChange, object: self)
  }

  /// Close one pane (callers confirm first). The last pane left turns the
  /// tab back into a plain one; closing a plain tab's only surface closes it.
  func closePane(_ id: Int?, inTabAt index: Int) {
    guard tabs.indices.contains(index) else { return }
    guard let id, case .split(let split) = tabs[index] else {
      closeTab(index: index)
      return
    }
    guard let removed = split.remove(id) else { return }
    recordClosedTab(
      removed, workspaceID: records[index].workspaceID, fromTabUID: records[index].uid)
    cleanupTab(removed)
    untrack(removed)
    removed.view.removeFromSuperview()
    collapseIfSingle(split, at: index)
    afterPaneChange(at: index)
  }

  /// Move the focused pane of the selected tab into a tab of its own.
  func movePaneToNewTab() {
    guard let split = selectedSplit, let removed = split.remove(split.focusedPane) else { return }
    removed.view.removeFromSuperview()
    collapseIfSingle(split, at: selectedIndex)
    insertTab(removed)
  }

  /// Move another tab's surfaces into the selected tab as panes.
  func joinTab(at sourceIndex: Int, axis: SplitAxis) {
    guard tabs.indices.contains(sourceIndex), selectedIndex >= 0, sourceIndex != selectedIndex
    else { return }
    let source = records[sourceIndex].entry
    source.view.removeFromSuperview()
    records.remove(at: sourceIndex)
    if sourceIndex < selectedIndex { selectedIndex -= 1 }
    for surface in source.surfaces {
      splitSelectedTab(with: surface, axis: axis)
    }
  }

  @discardableResult
  func focusNeighborPane(_ direction: PaneDirection) -> Bool {
    guard let split = selectedSplit,
      let id = split.layout.neighbor(of: split.focusedPane, toward: direction)
    else { return false }
    focusPane(id, inTabAt: selectedIndex)
    return true
  }

  func cyclePane(by step: Int) {
    guard let split = selectedSplit else { return }
    let order = split.layout.leaves
    guard let position = order.firstIndex(of: split.focusedPane) else { return }
    let next = order[(position + step % order.count + order.count) % order.count]
    focusPane(next, inTabAt: selectedIndex)
  }

  func toggleZoomSelectedPane() {
    guard let split = selectedSplit else { return }
    split.toggleZoom()
    syncToWindowModel()
  }

  func equalizeSelectedPanes() {
    guard let split = selectedSplit else { return }
    split.setLayout(split.layout.equalized())
  }

  func resizeFocusedPane(toward direction: PaneDirection, by delta: Double = 0.05) {
    guard let split = selectedSplit else { return }
    split.setLayout(split.layout.resizing(split.focusedPane, toward: direction, by: delta))
  }

  private func wire(_ split: SplitTab) {
    split.onFocusRequest = { [weak self, weak split] id in
      guard let self, let split,
        let index = self.tabs.firstIndex(where: {
          if case .split(let candidate) = $0 { return candidate === split }
          return false
        })
      else { return }
      self.focusPane(id, inTabAt: index)
    }
  }

  private func collapseIfSingle(_ split: SplitTab, at index: Int) {
    guard split.count == 1, let last = split.orderedPanes.first?.entry,
      tabs.indices.contains(index)
    else { return }
    split.view.removeFromSuperview()
    last.view.removeFromSuperview()
    records[index].entry = last
    if index == selectedIndex { install(last.view) }
  }

  private func afterPaneChange(at index: Int) {
    syncToWindowModel()
    guard index == selectedIndex, tabs.indices.contains(index) else { return }
    activateKeyboardFocus(tabs[index].focused)
    NotificationCenter.default.post(name: .impulseActiveTabDidChange, object: self)
  }

  /// Track (or forget) open file paths for O(1) deduplication.
  private func track(_ entry: TabEntry) {
    for surface in entry.surfaces {
      switch surface {
      case .editor(let e):
        if let p = e.filePath { openFilePaths.insert(p) }
      case .imagePreview(let p, _):
        openFilePaths.insert(p)
      default:
        break
      }
    }
  }

  private func untrack(_ entry: TabEntry) {
    for surface in entry.surfaces {
      switch surface {
      case .editor(let e):
        if let p = e.filePath { openFilePaths.remove(p) }
      case .imagePreview(let p, _):
        openFilePaths.remove(p)
      default:
        break
      }
    }
  }

  // MARK: - Selected Tab Helpers

  /// The currently selected tab entry, or `nil` if no tabs are open.
  var selectedTab: TabEntry? {
    guard records.indices.contains(selectedIndex) else { return nil }
    return records[selectedIndex].entry
  }

  /// The currently selected terminal container, or `nil` if the selection is
  /// not a terminal tab.
  var selectedTerminal: TerminalContainer? {
    if case .terminal(let tc)? = selectedTab?.focused { return tc }
    return nil
  }

  /// The currently selected editor tab, or `nil` if the selection is not an
  /// editor tab.
  var selectedEditor: EditorTab? {
    if case .editor(let et)? = selectedTab?.focused { return et }
    return nil
  }

  // MARK: - Ownership Queries

  /// Returns `true` if this TabManager owns the given terminal (i.e. it lives
  /// in one of our terminal containers).
  func ownsTerminal(_ terminal: TerminalTab) -> Bool {
    allSurfaces.contains {
      if case .terminal(let container) = $0 {
        return container.terminals.contains { $0 === terminal }
      }
      return false
    }
  }

  /// Returns `true` if this TabManager owns the given editor tab.
  func ownsEditor(_ editor: EditorTab) -> Bool {
    allSurfaces.contains {
      if case .editor(let e) = $0 { return e === editor }
      return false
    }
  }


  // MARK: - Theming

  func applyTheme(_ theme: Theme) {
    self.theme = theme
    contentView.layer?.backgroundColor = theme.bgColor.cgColor
    if let cache = iconCache {
      cache.rebuild(theme: theme)
    } else {
      iconCache = IconCache(theme: theme)
    }
    rebuildSegments()
    for tab in tabs {
      tab.applyTheme(theme)
    }
  }

  // MARK: - Segmented Control

  /// Rebuilds the tab display to match the current tab list.
  private func rebuildSegments() {
    syncToWindowModel()
  }

  /// Push current tab state to the SwiftUI-observable WindowModel.
  func syncToWindowModel() {
    assert(Thread.isMainThread, "syncToWindowModel must run on the main thread")
    guard let ws = windowModel else { return }

    // Internal consistency: selectedIndex must always point at a real tab,
    // or be -1 when there are no tabs. Catches races where two paths both
    // mutate `tabs` / `selectedIndex` without coordinating.
    assert(
      selectedIndex == -1 || records.indices.contains(selectedIndex),
      "TabManager state inconsistent: selectedIndex=\(selectedIndex), tabs.count=\(records.count)"
    )

    let names = Dictionary(uniqueKeysWithValues: workspaces.map { ($0.id, $0.name) })
    let infos = records.enumerated().map { (i, record) in
      let tab = record.entry
      let surface = tab.focused
      let directory = tabDirectory(for: surface)
      var isDirectInteractionActive = false
      var progress: TerminalProgress? = nil
      let isDirty = tab.surfaces.contains {
        if case .editor(let editor) = $0 { return editor.isModified }
        return false
      }
      switch surface {
      case .terminal(let container):
        isDirectInteractionActive = container.activeTerminal?.isDirectInteraction ?? false
        if let report = container.activeTerminal?.progress, report.state != .hidden {
          progress = report
        }
      default:
        break
      }
      var paneCount = 1
      var isZoomed = false
      if case .split(let split) = tab {
        paneCount = split.count
        isZoomed = split.isZoomed
      }
      return TabDisplayInfo(
        id: record.uid,
        index: i,
        title: tab.title,
        icon: tabIcon(for: surface),
        isPinned: record.pinned,
        isTerminal: { if case .terminal = surface { return true } else { return false } }(),
        needsAttention: tab.needsAttention,
        gitBranch: directory.flatMap { cachedGitBranch(forDirectory: $0) },
        directory: directory.map(Self.abbreviateHomePath),
        isDirectInteractionActive: isDirectInteractionActive,
        progress: progress,
        isDirty: isDirty,
        paneCount: paneCount,
        isZoomed: isZoomed,
        workspaceID: record.workspaceID,
        workspaceName: names[record.workspaceID] ?? ""
      )
    }
    ws.allTabs = infos
    ws.refreshTabs(
      infos.filter { $0.workspaceID == activeWorkspaceID }, selectedIndex: selectedIndex)
    ws.workspaces = workspaces.map { workspace in
      let tabs = infos.filter { $0.workspaceID == workspace.id }
      return WorkspaceInfo(
        id: workspace.id,
        name: workspace.name,
        root: workspace.root,
        isScratch: workspace.kind == .scratch,
        isActive: workspace.id == activeWorkspaceID,
        isExpanded: workspace.isExpanded,
        tabs: tabs,
        attentionCount: tabs.filter(\.needsAttention).count,
        progress: tabs.compactMap(\.progress).first,
        repository: workspace.repository
      )
    }

    // Update the active file path for sidebar highlighting.
    if let editor = selectedEditor {
      ws.activeFilePath = editor.filePath
    } else {
      ws.activeFilePath = nil
    }
  }

  /// Updates tab labels to reflect current tab titles (e.g., after a
  /// terminal title change or editor save).
  func refreshSegmentLabels() {
    syncToWindowModel()
  }

  // MARK: - Vertical Tab Subtitles

  /// Working directory used for a tab's sidebar subtitle.
  private func tabDirectory(for tab: TabEntry) -> String? {
    switch tab {
    case .terminal(let container):
      let cwd = container.activeTerminal?.currentWorkingDirectory
      return (cwd?.isEmpty ?? true) ? nil : cwd
    case .editor(let editor):
      return editor.projectDirectory
        ?? editor.filePath.map { ($0 as NSString).deletingLastPathComponent }
    case .imagePreview(let path, _):
      return (path as NSString).deletingLastPathComponent
    case .diffReview(let repoRoot, _):
      return repoRoot
    case .split(let split):
      return tabDirectory(for: split.focused)
    }
  }

  /// Git branch for a directory, resolved off the main thread and cached
  /// briefly. Returns nil on cache miss and re-syncs once resolved, so the
  /// sidebar updates without ever blocking tab switching.
  private func cachedGitBranch(forDirectory dir: String) -> String? {
    if let entry = tabBranchCache[dir], Date().timeIntervalSince(entry.at) < 15 {
      return entry.branch.isEmpty ? nil : entry.branch
    }
    guard !tabBranchPending.contains(dir) else {
      return tabBranchCache[dir].flatMap { $0.branch.isEmpty ? nil : $0.branch }
    }
    tabBranchPending.insert(dir)
    DispatchQueue.global(qos: .utility).async { [weak self] in
      let branch = ImpulseCore.gitBranch(path: dir) ?? ""
      DispatchQueue.main.async {
        guard let self else { return }
        self.tabBranchPending.remove(dir)
        let previous = self.tabBranchCache[dir]?.branch
        if self.tabBranchCache.count > 128 {
          self.tabBranchCache.removeAll(keepingCapacity: true)
        }
        self.tabBranchCache[dir] = (branch: branch, at: Date())
        if previous != branch {
          self.syncToWindowModel()
        }
      }
    }
    return tabBranchCache[dir].flatMap { $0.branch.isEmpty ? nil : $0.branch }
  }

  /// "/Users/me/Code/x" → "~/Code/x" for compact sidebar subtitles.
  static func abbreviateHomePath(_ path: String) -> String {
    let home = NSHomeDirectory()
    if path == home { return "~" }
    if path.hasPrefix(home + "/") {
      return "~" + path.dropFirst(home.count)
    }
    return path
  }

  /// Returns the appropriate icon for a tab entry.
  private func tabIcon(for tab: TabEntry) -> NSImage? {
    switch tab {
    case .terminal:
      return iconCache?.toolbarIcon(name: "console")
        ?? NSImage(systemSymbolName: "terminal.fill", accessibilityDescription: "Terminal")
    case .editor(let editor):
      if let path = editor.filePath {
        let filename = (path as NSString).lastPathComponent
        return iconCache?.icon(filename: filename, isDirectory: false, expanded: false)
          ?? NSWorkspace.shared.icon(forFile: path)
      }
      return NSImage(systemSymbolName: "doc.text", accessibilityDescription: "Editor")
    case .imagePreview:
      return iconCache?.toolbarIcon(name: "image")
        ?? NSImage(systemSymbolName: "photo", accessibilityDescription: "Image")
    case .diffReview:
      return NSImage(
        systemSymbolName: "arrow.triangle.branch", accessibilityDescription: "Review Changes")
    case .split(let split):
      return tabIcon(for: split.focused)
    }
  }

  // MARK: - Editor Options

  /// Builds an `EditorOptions` value from the current `Settings` so that
  /// newly opened editor tabs inherit the user's preferences.
  func editorOptionsFromSettings() -> EditorOptions {
    return EditorOptions(
      fontSize: UInt32(settings.fontSize),
      fontFamily: settings.fontFamily,
      tabSize: UInt32(settings.tabWidth),
      insertSpaces: settings.useSpaces,
      wordWrap: settings.wordWrap ? "on" : "off",
      minimapEnabled: settings.minimapEnabled,
      lineNumbers: settings.showLineNumbers ? "on" : "off",
      renderWhitespace: settings.renderWhitespace,
      renderLineHighlight: settings.highlightCurrentLine ? "line" : "none",
      rulers: settings.showRightMargin ? [UInt32(settings.rightMarginPosition)] : [],
      stickyScroll: settings.stickyScroll,
      bracketPairColorization: settings.bracketPairColorization,
      indentGuides: settings.indentGuides,
      fontLigatures: settings.fontLigatures,
      folding: settings.folding,
      scrollBeyondLastLine: settings.scrollBeyondLastLine,
      smoothScrolling: settings.smoothScrolling,
      cursorStyle: settings.editorCursorStyle,
      cursorBlinking: settings.editorCursorBlinking,
      lineHeight: settings.editorLineHeight > 0 ? UInt32(settings.editorLineHeight) : nil,
      autoClosingBrackets: settings.editorAutoClosingBrackets,
      cursorSurroundingLines: UInt32(settings.editorCursorSurroundingLines),
      selectionHighlight: settings.editorSelectionHighlight,
      occurrencesHighlight: settings.editorOccurrencesHighlight,
      wordBasedSuggestions: settings.editorWordBasedSuggestions
    )
  }

  // MARK: - Language Detection

  /// Maps a file path to its Monaco language identifier based on extension.
  private func languageIdForPath(_ path: String) -> String {
    // Check filename (without extension) for special cases.
    let filename = (path as NSString).lastPathComponent.lowercased()
    if filename == "dockerfile" || filename == "containerfile" || filename.hasPrefix("dockerfile.")
      || filename.hasPrefix("containerfile.")
    {
      return "dockerfile"
    }
    switch filename {
    case "makefile", "gnumakefile": return "plaintext"
    case "cmakelists.txt": return "plaintext"
    case ".gitignore", ".dockerignore": return "ini"
    case ".env", ".env.local", ".env.example": return "ini"
    default: break
    }

    let ext = (path as NSString).pathExtension.lowercased()
    switch ext {
    case "rs": return "rust"
    case "swift": return "swift"
    case "py", "pyi": return "python"
    case "js", "mjs", "cjs", "jsx": return "javascript"
    case "ts", "mts", "cts", "tsx": return "typescript"
    case "c": return "c"
    case "cpp", "cc", "cxx", "hxx", "hh": return "cpp"
    case "h", "hpp": return "cpp"
    case "go": return "go"
    case "java": return "java"
    case "rb": return "ruby"
    case "sh", "bash", "zsh", "fish": return "shell"
    case "json", "jsonc": return "json"
    case "yaml", "yml": return "yaml"
    case "toml": return "toml"
    case "md", "markdown": return "markdown"
    case "html", "htm": return "html"
    case "css": return "css"
    case "scss": return "scss"
    case "less": return "less"
    case "xml", "svg", "xsl", "xslt": return "xml"
    case "php": return "php"
    case "sql": return "sql"
    case "lua": return "lua"
    case "kt", "kts": return "kotlin"
    case "dart": return "dart"
    case "ex", "exs": return "elixir"
    case "graphql", "gql": return "graphql"
    case "cs": return "csharp"
    case "fs", "fsx": return "fsharp"
    case "pl", "pm": return "perl"
    case "r": return "r"
    case "m": return "objective-c"
    case "scala": return "scala"
    case "clj", "cljs", "cljc": return "clojure"
    case "coffee": return "coffee"
    case "pug": return "pug"
    case "tf", "tfvars": return "hcl"
    case "proto": return "protobuf"
    case "ini", "cfg", "conf": return "ini"
    case "bat", "cmd": return "bat"
    case "ps1", "psm1": return "powershell"
    default: return "plaintext"
    }
  }

  // MARK: - File Type Detection

  /// Returns `true` if the file path has an image extension.
  static func isImageFile(_ path: String) -> Bool {
    let ext = (path as NSString).pathExtension.lowercased()
    switch ext {
    case "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "tiff", "tif":
      return true
    default:
      return false
    }
  }

  /// Returns `true` if the file is likely a binary (>10 MB or contains null
  /// bytes in the first 8 KB).
  static func isBinaryFile(_ path: String) -> Bool {
    let fm = FileManager.default
    guard let attrs = try? fm.attributesOfItem(atPath: path),
      let size = attrs[.size] as? UInt64
    else { return false }

    // Files larger than 10 MB are treated as binary.
    if size > 10 * 1024 * 1024 { return true }

    // Read the first 8 KB and check for null bytes.
    guard let handle = FileHandle(forReadingAtPath: path) else { return false }
    defer { handle.closeFile() }
    let data = handle.readData(ofLength: 8192)
    return data.contains(0)
  }
}
