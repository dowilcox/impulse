import AppKit
import SwiftTerm

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

    /// The view to display in the content area.
    var view: NSView {
        switch self {
        case .terminal(let container): return container
        case .editor(let editor): return editor
        case .imagePreview(_, let view): return view
        }
    }

    /// The title to show in the tab bar segment.
    var title: String {
        switch self {
        case .terminal(let container):
            if let active = container.activeTerminal {
                let title = active.tabTitle
                return title.isEmpty ? ImpulseCore.getUserLoginShellName() : title
            }
            return ImpulseCore.getUserLoginShellName()
        case .editor(let editor):
            if let path = editor.filePath {
                let name = (path as NSString).lastPathComponent
                return editor.isModified ? "\(name) *" : name
            }
            return "Untitled"
        case .imagePreview(let path, _):
            return (path as NSString).lastPathComponent
        }
    }

    /// Extracts a `TabInfo` snapshot for the status bar.
    var info: TabInfo {
        switch self {
        case .terminal(let container):
            return TabInfo(
                cwd: container.activeTerminal?.currentWorkingDirectory,
                gitBranch: nil,
                shellName: ImpulseCore.getUserLoginShellName(),
                cursorLine: nil, cursorCol: nil,
                language: nil, encoding: nil, indentInfo: nil
            )
        case .editor(let editor):
            return TabInfo(
                cwd: editor.filePath,
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
        }
    }

    /// Focus the primary interactive view.
    func focus() {
        switch self {
        case .terminal(let container):
            container.activeTerminal?.focus()
        case .editor(let editor):
            editor.focus()
        case .imagePreview:
            break
        }
    }

    /// Apply a new theme.
    func applyTheme(_ theme: Theme) {
        switch self {
        case .terminal(let container):
            let termTheme = TerminalTheme(
                bg: theme.bgHex,
                fg: theme.fgHex,
                terminalPalette: theme.terminalPalette.map { $0.hexString }
            )
            container.applyTheme(theme: termTheme)
        case .editor(let editor):
            editor.applyTheme(theme.monacoThemeDefinition())
        case .imagePreview(_, let view):
            view.layer?.backgroundColor = theme.bg.cgColor
        }
    }
}

// MARK: - Closed Tab Info

/// Information about a closed tab, used for the "reopen closed tab" feature.
/// Only editor and image preview tabs are recorded (terminals cannot be reopened).
enum ClosedTabInfo {
    case editor(path: String)
    case imagePreview(path: String)

    var path: String {
        switch self {
        case .editor(let p), .imagePreview(let p): return p
        }
    }
}

// MARK: - Tab Manager

/// Manages the collection of open tabs (terminal or editor) and the
/// segmented control used to switch between them. The segmented control is
/// placed in the window's titlebar container.
final class TabManager: NSObject {
    /// The ordered list of open tabs.
    private(set) var tabs: [TabEntry] = []

    /// Per-tab pinned state, indexed in parallel with `tabs`.
    private(set) var pinnedTabs: [Bool] = []

    /// Set of file paths currently open in editor/image tabs for O(1) deduplication.
    private var openFilePaths: Set<String> = []

    /// Stack of recently closed tabs for "reopen closed tab" (Cmd+Shift+T).
    private(set) var closedTabs: [ClosedTabInfo] = []

    /// Maximum number of closed tabs to remember.
    private let maxClosedTabs = 20

    /// The index of the currently selected tab, or -1 if no tabs are open.
    private(set) var selectedIndex: Int = -1

    /// The custom tab bar displayed in the titlebar for tab switching.
    let tabBar: CustomTabBar

    /// Icon cache for themed file icons in tab bar.
    private(set) var iconCache: IconCache?

    /// The container view that hosts the active tab's view.
    let contentView: NSView

    var settings: Settings
    private var theme: Theme
    private let core: ImpulseCore

    /// Returns a `TabInfo` snapshot for the currently active tab, or `nil` if
    /// no tabs are open.
    var activeTabInfo: TabInfo? {
        guard selectedIndex >= 0, selectedIndex < tabs.count else { return nil }
        return tabs[selectedIndex].info
    }

    init(settings: Settings, theme: Theme, core: ImpulseCore) {
        self.settings = settings
        self.theme = theme
        self.core = core

        tabBar = CustomTabBar()
        tabBar.translatesAutoresizingMaskIntoConstraints = false

        iconCache = IconCache(theme: theme)

        contentView = NSView()
        contentView.wantsLayer = true
        contentView.layer?.backgroundColor = theme.bg.cgColor

        super.init()

        tabBar.delegate = self
    }

    // MARK: - Adding Tabs

    /// Creates a new terminal tab (wrapped in a TerminalContainer for split
    /// support) and makes it active.
    func addTerminalTab(directory: String? = nil) {
        let dir = directory ?? NSHomeDirectory()
        let termSettings = TerminalSettings(
            terminalFontSize: settings.terminalFontSize,
            terminalFontFamily: settings.terminalFontFamily,
            terminalCursorShape: settings.terminalCursorShape,
            terminalCursorBlink: settings.terminalCursorBlink,
            terminalScrollback: settings.terminalScrollback,
            lastDirectory: dir,
            terminalCopyOnSelect: settings.terminalCopyOnSelect
        )
        let termTheme = TerminalTheme(
            bg: theme.bgHex,
            fg: theme.fgHex,
            terminalPalette: theme.terminalPalette.map { $0.hexString }
        )
        let container = TerminalContainer(
            frame: NSRect(x: 0, y: 0, width: 800, height: 600),
            settings: termSettings,
            theme: termTheme
        )
        let entry = TabEntry.terminal(container)
        insertTab(entry)
    }

    /// Creates a new editor tab for the given file path.
    ///
    /// If a tab for the same file is already open, it is selected instead of
    /// creating a duplicate. Image files are opened in a preview tab. Binary
    /// files (>10 MB or containing null bytes) are skipped with an alert.
    func addEditorTab(path: String) {
        // O(1) deduplication using the openFilePaths set.
        if openFilePaths.contains(path) {
            if let existingIndex = tabs.firstIndex(where: {
                switch $0 {
                case .editor(let e): return e.filePath == path
                case .imagePreview(let p, _): return p == path
                default: return false
                }
            }) {
                selectTab(index: existingIndex)
            }
            return
        }

        // Image files get a preview tab instead of an editor.
        if Self.isImageFile(path) {
            addImagePreviewTab(path: path)
            return
        }

        // Reject binary files.
        if Self.isBinaryFile(path) {
            let alert = NSAlert()
            alert.messageText = "Binary File"
            alert.informativeText = "The file \"\((path as NSString).lastPathComponent)\" appears to be a binary file and cannot be opened in the editor."
            alert.alertStyle = .informational
            alert.addButton(withTitle: "OK")
            alert.runModal()
            return
        }

        // Read file content off the main thread, then create the editor tab on main.
        let editorOptions = editorOptionsFromSettings()
        let themeDef = theme.monacoThemeDefinition()
        let language = languageIdForPath(path)

        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let fileContent = (try? String(contentsOfFile: path, encoding: .utf8)) ?? ""

            DispatchQueue.main.async { [weak self] in
                guard let self else { return }

                // Re-check deduplication in case a tab was opened while reading.
                if self.openFilePaths.contains(path) { return }

                let editorTab = EditorTab(frame: NSRect(x: 0, y: 0, width: 800, height: 600))
                editorTab.openFile(path: path, content: fileContent, language: language)
                editorTab.loadEditor()

                // Apply editor settings (font, tab size, etc.) from the current settings.
                editorTab.applySettings(editorOptions)
                editorTab.applyTheme(themeDef)

                let entry = TabEntry.editor(editorTab)
                self.insertTab(entry)
            }
        }
    }

    /// Creates an image preview tab with a scrollable NSImageView.
    private func addImagePreviewTab(path: String) {
        let container = NSView()
        container.wantsLayer = true

        let scrollView = NSScrollView()
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.drawsBackground = false
        scrollView.translatesAutoresizingMaskIntoConstraints = false

        let imageView = NSImageView()
        imageView.image = NSImage(contentsOfFile: path)
        imageView.imageScaling = .scaleProportionallyUpOrDown
        imageView.translatesAutoresizingMaskIntoConstraints = false
        imageView.setContentHuggingPriority(.defaultLow, for: .horizontal)
        imageView.setContentHuggingPriority(.defaultLow, for: .vertical)

        scrollView.documentView = imageView
        container.addSubview(scrollView)

        NSLayoutConstraint.activate([
            scrollView.topAnchor.constraint(equalTo: container.topAnchor, constant: 20),
            scrollView.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 20),
            scrollView.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -20),
            scrollView.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -20),
        ])

        let entry = TabEntry.imagePreview(path: path, view: container)
        insertTab(entry)
    }

    /// Inserts a new tab at the end and selects it.
    private func insertTab(_ entry: TabEntry) {
        tabs.append(entry)
        pinnedTabs.append(false)

        // Track open file paths for O(1) deduplication.
        switch entry {
        case .editor(let e):
            if let p = e.filePath { openFilePaths.insert(p) }
        case .imagePreview(let p, _):
            openFilePaths.insert(p)
        default:
            break
        }

        rebuildSegments()
        selectTab(index: tabs.count - 1)
    }

    // MARK: - Removing Tabs

    /// Release resources owned by a tab entry (kill processes, tear down
    /// WebViews) so they don't linger after the tab is removed.
    private func cleanupTab(_ entry: TabEntry) {
        switch entry {
        case .terminal(let container):
            container.terminateAllProcesses()
        case .editor(let editor):
            editor.cleanup()
        case .imagePreview:
            break
        }
    }

    /// Records a tab entry in the closed tabs stack for later reopening.
    private func recordClosedTab(_ entry: TabEntry) {
        switch entry {
        case .editor(let e):
            if let p = e.filePath {
                closedTabs.append(.editor(path: p))
                if closedTabs.count > maxClosedTabs {
                    closedTabs.removeFirst()
                }
            }
        case .imagePreview(let p, _):
            closedTabs.append(.imagePreview(path: p))
            if closedTabs.count > maxClosedTabs {
                closedTabs.removeFirst()
            }
        case .terminal:
            break // Terminals cannot be reopened
        }
    }

    /// Closes the tab at the given index. If it is the active tab, the
    /// nearest neighbor is selected. If it was the last tab, `selectedIndex`
    /// becomes -1.
    func closeTab(index: Int) {
        guard index >= 0, index < tabs.count else { return }

        let entry = tabs[index]
        recordClosedTab(entry)
        cleanupTab(entry)

        // Remove from open file paths tracking.
        switch entry {
        case .editor(let e):
            if let p = e.filePath { openFilePaths.remove(p) }
        case .imagePreview(let p, _):
            openFilePaths.remove(p)
        default:
            break
        }

        // Remove the tab's view from the content area if it is currently displayed.
        if index == selectedIndex {
            entry.view.removeFromSuperview()
        }

        tabs.remove(at: index)
        pinnedTabs.remove(at: index)
        rebuildSegments()

        if tabs.isEmpty {
            selectedIndex = -1
            NotificationCenter.default.post(name: .impulseActiveTabDidChange, object: self)
            return
        }

        // Select the nearest valid tab.
        let newIndex = min(index, tabs.count - 1)
        selectTab(index: newIndex)
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
        guard index >= 0, index < tabs.count else { return }
        pinnedTabs[index].toggle()
        rebuildSegments()
    }

    /// Closes all tabs except the one at `keepIndex`. Pinned tabs are preserved.
    func closeOtherTabs(keepIndex: Int) {
        guard keepIndex >= 0, keepIndex < tabs.count else { return }

        // Remember the tab entry to keep so we can find it after removal.
        let keepView = tabs[keepIndex].view

        // Remove the currently displayed view before modifying the array.
        if selectedIndex >= 0, selectedIndex < tabs.count, selectedIndex != keepIndex, !pinnedTabs[selectedIndex] {
            tabs[selectedIndex].view.removeFromSuperview()
        }

        // Collect indices to close in reverse order to preserve index validity.
        for i in stride(from: tabs.count - 1, through: 0, by: -1) {
            if i != keepIndex && !pinnedTabs[i] {
                let closedEntry = tabs[i]
                recordClosedTab(closedEntry)
                cleanupTab(closedEntry)

                // Remove from open file paths tracking.
                switch closedEntry {
                case .editor(let e):
                    if let p = e.filePath { openFilePaths.remove(p) }
                case .imagePreview(let p, _):
                    openFilePaths.remove(p)
                default:
                    break
                }

                tabs.remove(at: i)
                pinnedTabs.remove(at: i)
            }
        }

        rebuildSegments()

        if tabs.isEmpty {
            selectedIndex = -1
            NotificationCenter.default.post(name: .impulseActiveTabDidChange, object: self)
        } else {
            // Find the kept tab's new index.
            let newIndex = tabs.firstIndex(where: { $0.view === keepView }) ?? 0
            selectTab(index: newIndex)
        }
    }

    // MARK: - Reopening Closed Tabs

    /// Reopens the most recently closed editor or image preview tab.
    /// Returns the file path that was reopened, or `nil` if the stack was empty
    /// or the file no longer exists on disk.
    @discardableResult
    func reopenLastClosedTab() -> String? {
        guard let info = closedTabs.popLast() else { return nil }
        let path = info.path
        guard FileManager.default.fileExists(atPath: path) else { return nil }
        // Use the existing addEditorTab method, which handles deduplication
        // and image detection internally.
        addEditorTab(path: path)
        return path
    }

    // MARK: - Reordering

    /// Moves a tab from one index to another, preserving pinned state and
    /// updating selection to follow the moved tab.
    func moveTab(from sourceIndex: Int, to destinationIndex: Int) {
        guard sourceIndex != destinationIndex,
              sourceIndex >= 0, sourceIndex < tabs.count,
              destinationIndex >= 0, destinationIndex < tabs.count else { return }

        let entry = tabs.remove(at: sourceIndex)
        let pinned = pinnedTabs.remove(at: sourceIndex)
        tabs.insert(entry, at: destinationIndex)
        pinnedTabs.insert(pinned, at: destinationIndex)

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
        guard index >= 0, index < tabs.count else { return }

        // Remove the previous tab's view.
        if selectedIndex >= 0, selectedIndex < tabs.count {
            tabs[selectedIndex].view.removeFromSuperview()
        }

        selectedIndex = index
        tabBar.selectTab(index: index)

        // Activate the new tab.
        let entry = tabs[index]
        let view = entry.view
        view.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(view)
        NSLayoutConstraint.activate([
            view.topAnchor.constraint(equalTo: contentView.topAnchor),
            view.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            view.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
            view.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
        ])
        entry.focus()

        NotificationCenter.default.post(name: .impulseActiveTabDidChange, object: self)
    }

    // MARK: - Ownership Queries

    /// Returns `true` if this TabManager owns the given terminal (i.e. it lives
    /// in one of our terminal containers).
    func ownsTerminal(_ terminal: TerminalTab) -> Bool {
        for tab in tabs {
            if case .terminal(let container) = tab {
                if container.terminals.contains(where: { $0 === terminal }) {
                    return true
                }
            }
        }
        return false
    }

    /// Returns `true` if this TabManager owns the given editor tab.
    func ownsEditor(_ editor: EditorTab) -> Bool {
        return tabs.contains {
            if case .editor(let e) = $0 { return e === editor }
            return false
        }
    }

    // MARK: - Terminal Splitting

    /// Splits the active terminal tab horizontally (top/bottom).
    func splitTerminalHorizontally() {
        guard selectedIndex >= 0, selectedIndex < tabs.count else { return }
        if case .terminal(let container) = tabs[selectedIndex] {
            container.splitHorizontally()
        }
    }

    /// Splits the active terminal tab vertically (left/right).
    func splitTerminalVertically() {
        guard selectedIndex >= 0, selectedIndex < tabs.count else { return }
        if case .terminal(let container) = tabs[selectedIndex] {
            container.splitVertically()
        }
    }

    // MARK: - Theming

    func applyTheme(_ theme: Theme) {
        self.theme = theme
        contentView.layer?.backgroundColor = theme.bg.cgColor
        if let cache = iconCache {
            cache.rebuild(theme: theme)
        } else {
            iconCache = IconCache(theme: theme)
        }
        tabBar.applyTheme(theme)
        rebuildSegments()
        for tab in tabs {
            tab.applyTheme(theme)
        }
    }

    // MARK: - Segmented Control

    /// Rebuilds the tab bar to match the current tab list.
    private func rebuildSegments() {
        let items = tabs.enumerated().map { (i, tab) in
            TabItemData(title: tab.title, icon: tabIcon(for: tab), isPinned: pinnedTabs[i])
        }
        tabBar.rebuild(tabs: items, selectedIndex: selectedIndex, theme: theme)
    }

    /// Updates tab labels to reflect current tab titles (e.g., after a
    /// terminal title change or editor save).
    func refreshSegmentLabels() {
        let items = tabs.enumerated().map { (i, tab) in
            TabItemData(title: tab.title, icon: tabIcon(for: tab), isPinned: pinnedTabs[i])
        }
        tabBar.updateLabels(tabs: items)
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
        }
    }

    // MARK: - Context Menu

    /// Returns a context menu for the tab at the given index.
    func contextMenu(forTabIndex index: Int) -> NSMenu? {
        guard index >= 0, index < tabs.count else { return nil }
        let menu = NSMenu()

        let pinTitle = pinnedTabs[index] ? "Unpin Tab" : "Pin Tab"
        let pinItem = NSMenuItem(title: pinTitle, action: #selector(pinTabFromMenu(_:)), keyEquivalent: "")
        pinItem.tag = index
        pinItem.target = self
        menu.addItem(pinItem)

        let closeItem = NSMenuItem(title: "Close Tab", action: #selector(closeTabFromMenu(_:)), keyEquivalent: "")
        closeItem.tag = index
        closeItem.target = self
        menu.addItem(closeItem)

        let closeOthersItem = NSMenuItem(title: "Close Other Tabs", action: #selector(closeOtherTabsFromMenu(_:)), keyEquivalent: "")
        closeOthersItem.tag = index
        closeOthersItem.target = self
        menu.addItem(closeOthersItem)

        return menu
    }

    @objc private func closeTabFromMenu(_ sender: NSMenuItem) {
        closeTab(index: sender.tag)
    }

    @objc private func pinTabFromMenu(_ sender: NSMenuItem) {
        togglePin(index: sender.tag)
    }

    @objc private func closeOtherTabsFromMenu(_ sender: NSMenuItem) {
        closeOtherTabs(keepIndex: sender.tag)
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
            renderLineHighlight: settings.highlightCurrentLine ? "all" : "none",
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
            autoClosingBrackets: settings.editorAutoClosingBrackets
        )
    }

    // MARK: - Language Detection

    /// Maps a file path to its Monaco language identifier based on extension.
    private func languageIdForPath(_ path: String) -> String {
        // Check filename (without extension) for special cases.
        let filename = (path as NSString).lastPathComponent.lowercased()
        if filename == "dockerfile" || filename == "containerfile" || filename.hasPrefix("dockerfile.") || filename.hasPrefix("containerfile.") {
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
        case "png", "jpg", "jpeg", "gif", "svg", "webp", "bmp", "ico", "tiff", "tif":
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
              let size = attrs[.size] as? UInt64 else { return false }

        // Files larger than 10 MB are treated as binary.
        if size > 10 * 1024 * 1024 { return true }

        // Read the first 8 KB and check for null bytes.
        guard let handle = FileHandle(forReadingAtPath: path) else { return false }
        defer { handle.closeFile() }
        let data = handle.readData(ofLength: 8192)
        return data.contains(0)
    }
}

// MARK: - CustomTabBarDelegate

extension TabManager: CustomTabBarDelegate {
    func tabItemClicked(index: Int) {
        selectTab(index: index)
    }

    func tabItemCloseClicked(index: Int) {
        closeTab(index: index)
    }

    func tabItemContextMenu(index: Int) -> NSMenu? {
        return contextMenu(forTabIndex: index)
    }

    func tabItemMoved(from sourceIndex: Int, to destinationIndex: Int) {
        moveTab(from: sourceIndex, to: destinationIndex)
    }
}
