import AppKit
import Observation

// MARK: - Sidebar Mode

enum SidebarPanel {
    case files
    case search
}

// MARK: - Tab Display Info

/// Lightweight snapshot of a tab for the SwiftUI tab bar.
struct TabDisplayInfo: Identifiable {
    let id: Int       // Stable unique ID (survives reorders)
    let index: Int    // Current position in the tab array
    let title: String
    let icon: NSImage?
    let isPinned: Bool
    let isTerminal: Bool
}

// MARK: - Window Model

/// Per-window observable state shared between AppKit (MainWindowController)
/// and SwiftUI views. MainWindowController owns this object and mutates it;
/// SwiftUI views read it for automatic re-rendering via @Observable.
@Observable
final class WindowModel {

    // MARK: Tabs

    var tabDisplayInfos: [TabDisplayInfo] = []
    var selectedTabIndex: Int = -1

    // MARK: Sidebar

    var sidebarPanel: SidebarPanel = .files
    var showHiddenFiles: Bool = false

    // MARK: File tree (populated by MainWindowController)

    var fileTreeNodes: [FileTreeNode] = []
    var fileTreeRootPath: String = ""

    /// Flattened view of the file tree for LazyVStack rendering.
    /// Rebuilt explicitly when tree structure changes (expand/collapse/rebuild),
    /// NOT on git status changes — individual row views observe those directly
    /// via @Bindable on each node.
    var flatFileTree: [FlatTreeEntry] = []

    /// Path of the file currently open in the active editor tab.
    /// Used to highlight the active file in the sidebar.
    var activeFilePath: String? = nil

    // MARK: Search

    var searchQuery: String = ""
    var searchResults: [SearchResult] = []
    var searchCaseSensitive: Bool = false

    // MARK: Status bar — left group

    var shellName: String = ""
    var gitBranch: String? = nil
    var currentCwd: String = ""
    var blameInfo: String? = nil

    // MARK: Status bar — right group

    var cursorLine: Int? = nil
    var cursorCol: Int? = nil
    var currentLanguage: String? = nil
    var currentEncoding: String = "UTF-8"
    var currentIndent: String? = nil
    var isPreviewable: Bool = false
    var isPreviewing: Bool = false

    // MARK: Overlays

    var commandPaletteVisible: Bool = false

    // MARK: Theme

    var theme: Theme = ThemeManager.theme(forName: "nord")

    // MARK: Icons

    /// Shared icon cache for themed file/folder icons in the sidebar.
    var iconCache: IconCache?

    // MARK: Callbacks (set by MainWindowController for SwiftUI → AppKit)

    var onTabSelected: ((Int) -> Void)?
    var onTabClosed: ((Int) -> Void)?
    var onTabMoved: ((Int, Int) -> Void)?
    var onNewTab: (() -> Void)?
    var onSidebarToggle: (() -> Void)?
    var onPreviewToggle: (() -> Void)?
    var onOpenFile: ((String, Int?) -> Void)?
    var onNewFile: ((String) -> Void)?
    var onNewFolder: ((String) -> Void)?
    var onRefreshTree: (() -> Void)?
    var onCollapseAll: (() -> Void)?
    var onToggleHidden: (() -> Void)?

    // MARK: Methods

    /// Replace the tab display info array. Called by TabManager.
    func refreshTabs(_ infos: [TabDisplayInfo], selectedIndex: Int) {
        self.tabDisplayInfos = infos
        self.selectedTabIndex = selectedIndex
    }

    /// Update status bar from a TabInfo snapshot.
    func updateStatusBar(from info: TabInfo) {
        shellName = info.shellName ?? ""
        currentCwd = info.cwd ?? ""
        cursorLine = info.cursorLine
        cursorCol = info.cursorCol
        currentLanguage = info.language
        currentEncoding = info.encoding ?? "UTF-8"
        currentIndent = info.indentInfo
    }

    /// Replace the file tree nodes and rebuild the flat rendering list.
    /// Use this instead of setting `fileTreeNodes` directly.
    func updateFileTree(_ nodes: [FileTreeNode], rootPath: String? = nil) {
        fileTreeNodes = nodes
        if let rootPath { fileTreeRootPath = rootPath }
        rebuildFlatTree()
    }

    /// Rebuild the flat tree from current nodes. Call after any structural
    /// change (expand, collapse, children loaded) but NOT after git status
    /// changes — row views observe those via @Bindable.
    func rebuildFlatTree() {
        flatFileTree = FileTreeNode.flatten(fileTreeNodes)
    }
}
