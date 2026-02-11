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

/// Discriminated union representing either a terminal or editor tab.
/// Stores the NSView and the metadata needed for display in the tab bar.
enum TabEntry {
    case terminal(TerminalContainer)
    case editor(EditorTab)

    /// The view to display in the content area.
    var view: NSView {
        switch self {
        case .terminal(let container): return container
        case .editor(let editor): return editor
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
        }
    }

    /// Focus the primary interactive view.
    func focus() {
        switch self {
        case .terminal(let container):
            container.activeTerminal?.focus()
        case .editor(let editor):
            editor.focus()
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
        }
    }
}

// MARK: - Tab Manager

/// Manages the collection of open tabs (terminal or editor) and the segmented
/// control used to switch between them. The segmented control is placed in the
/// window's toolbar.
final class TabManager: NSObject {
    /// The ordered list of open tabs.
    private(set) var tabs: [TabEntry] = []

    /// The index of the currently selected tab, or -1 if no tabs are open.
    private(set) var selectedIndex: Int = -1

    /// The segmented control displayed in the toolbar.
    let segmentedControl: NSSegmentedControl

    /// The container view that hosts the active tab's view.
    let contentView: NSView

    private let settings: Settings
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

        segmentedControl = NSSegmentedControl()
        segmentedControl.segmentStyle = .texturedSquare
        segmentedControl.trackingMode = .selectOne
        segmentedControl.controlSize = .regular

        contentView = NSView()
        contentView.wantsLayer = true
        contentView.layer?.backgroundColor = theme.bg.cgColor

        super.init()

        segmentedControl.target = self
        segmentedControl.action = #selector(segmentSelected(_:))
    }

    // MARK: - Adding Tabs

    /// Creates a new terminal tab (wrapped in a TerminalContainer for split
    /// support) and makes it active.
    func addTerminalTab() {
        let termSettings = TerminalSettings(
            terminalFontSize: settings.terminalFontSize,
            terminalFontFamily: settings.terminalFontFamily,
            terminalCursorShape: settings.terminalCursorShape,
            terminalCursorBlink: settings.terminalCursorBlink,
            terminalScrollback: settings.terminalScrollback,
            lastDirectory: settings.lastDirectory
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
    /// creating a duplicate.
    func addEditorTab(path: String) {
        // Deduplicate: if a tab for this file already exists, select it.
        if let existingIndex = tabs.firstIndex(where: {
            if case .editor(let e) = $0 { return e.filePath == path }
            return false
        }) {
            selectTab(index: existingIndex)
            return
        }

        let editorTab = EditorTab(frame: NSRect(x: 0, y: 0, width: 800, height: 600))

        // Read the file and open it in the editor.
        let content = (try? String(contentsOfFile: path, encoding: .utf8)) ?? ""
        let language = languageIdForPath(path)
        editorTab.openFile(path: path, content: content, language: language)
        editorTab.loadEditor()

        let entry = TabEntry.editor(editorTab)
        insertTab(entry)
    }

    /// Inserts a new tab at the end and selects it.
    private func insertTab(_ entry: TabEntry) {
        tabs.append(entry)
        rebuildSegments()
        selectTab(index: tabs.count - 1)
    }

    // MARK: - Removing Tabs

    /// Closes the tab at the given index. If it is the active tab, the
    /// nearest neighbor is selected. If it was the last tab, `selectedIndex`
    /// becomes -1.
    func closeTab(index: Int) {
        guard index >= 0, index < tabs.count else { return }

        let entry = tabs[index]

        // Remove the tab's view from the content area if it is currently displayed.
        if index == selectedIndex {
            entry.view.removeFromSuperview()
        }

        tabs.remove(at: index)
        rebuildSegments()

        if tabs.isEmpty {
            selectedIndex = -1
            NotificationCenter.default.post(name: .impulseActiveTabDidChange, object: nil)
            return
        }

        // Select the nearest valid tab.
        let newIndex = min(index, tabs.count - 1)
        selectTab(index: newIndex)
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
        segmentedControl.selectedSegment = index

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

        NotificationCenter.default.post(name: .impulseActiveTabDidChange, object: nil)
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
        for tab in tabs {
            tab.applyTheme(theme)
        }
    }

    // MARK: - Segmented Control

    /// Rebuilds the segmented control segments to match the current tab list.
    private func rebuildSegments() {
        segmentedControl.segmentCount = tabs.count
        for (i, tab) in tabs.enumerated() {
            segmentedControl.setLabel(tab.title, forSegment: i)
            segmentedControl.setWidth(0, forSegment: i) // Auto-size to fit label.

            // Provide a close-tab option via the segment's right-click menu.
            let menu = NSMenu()
            let closeItem = NSMenuItem(title: "Close Tab", action: #selector(closeTabFromMenu(_:)), keyEquivalent: "")
            closeItem.tag = i
            closeItem.target = self
            menu.addItem(closeItem)
            segmentedControl.setMenu(menu, forSegment: i)
        }
        if selectedIndex >= 0, selectedIndex < tabs.count {
            segmentedControl.selectedSegment = selectedIndex
        }
    }

    /// Updates segment labels to reflect current tab titles (e.g., after a
    /// terminal title change or editor save).
    func refreshSegmentLabels() {
        for (i, tab) in tabs.enumerated() {
            segmentedControl.setLabel(tab.title, forSegment: i)
        }
    }

    @objc private func segmentSelected(_ sender: NSSegmentedControl) {
        let index = sender.selectedSegment
        guard index >= 0, index < tabs.count else { return }
        selectTab(index: index)
    }

    @objc private func closeTabFromMenu(_ sender: NSMenuItem) {
        closeTab(index: sender.tag)
    }

    // MARK: - Language Detection

    /// Maps a file path to its Monaco language identifier based on extension.
    private func languageIdForPath(_ path: String) -> String {
        let ext = (path as NSString).pathExtension.lowercased()
        switch ext {
        case "rs": return "rust"
        case "swift": return "swift"
        case "py": return "python"
        case "js": return "javascript"
        case "ts": return "typescript"
        case "jsx": return "javascriptreact"
        case "tsx": return "typescriptreact"
        case "c": return "c"
        case "cpp", "cc", "cxx": return "cpp"
        case "h", "hpp": return "cpp"
        case "go": return "go"
        case "java": return "java"
        case "rb": return "ruby"
        case "sh", "bash", "zsh", "fish": return "shellscript"
        case "json": return "json"
        case "yaml", "yml": return "yaml"
        case "toml": return "toml"
        case "md", "markdown": return "markdown"
        case "html", "htm": return "html"
        case "css": return "css"
        case "scss": return "scss"
        case "xml": return "xml"
        case "sql": return "sql"
        case "lua": return "lua"
        case "zig": return "zig"
        case "kt", "kts": return "kotlin"
        case "dart": return "dart"
        case "ex", "exs": return "elixir"
        case "hs": return "haskell"
        default: return "plaintext"
        }
    }
}
