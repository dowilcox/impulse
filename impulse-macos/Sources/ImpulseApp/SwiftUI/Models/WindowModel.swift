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
  let id: Int  // Stable unique ID (survives reorders)
  let index: Int  // Current position in the tab array
  let title: String
  let icon: NSImage?
  let isPinned: Bool
  let isTerminal: Bool
  let needsAttention: Bool
  /// Git branch of the tab's working directory (vertical tab subtitle).
  var gitBranch: String? = nil
  /// Abbreviated working directory, shown when no branch is available.
  var directory: String? = nil
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
  /// "sidebar" (Warp-style vertical list) or "top" (horizontal bar).
  var tabBarPosition: String = "sidebar"

  // MARK: Sidebar

  var sidebarPanel: SidebarPanel = .files
  var sidebarVisible: Bool = true {
    didSet { onSidebarVisibilityChanged?(sidebarVisible) }
  }
  var sidebarWidth: CGFloat = 250
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

  /// Path currently selected in the file tree. Used by titlebar file actions
  /// so they act on the SwiftUI sidebar selection.
  var selectedFileTreePath: String? = nil

  // MARK: Search

  var searchQuery: String = ""
  var searchResults: [SearchResult] = []
  var searchCaseSensitive: Bool = false

  // MARK: Status bar — left group

  var shellName: String = ""
  var gitBranch: String? = nil
  var currentCwd: String = ""

  // MARK: Terminal input bar

  /// Whether the input bar is enabled in settings.
  var contextBarEnabled: Bool = true
  /// True while the active terminal is executing a command.
  var commandRunning: Bool = false
  /// Exit code and duration of the active terminal's last command.
  var lastCommandExitCode: Int32? = nil
  var lastCommandDurationMs: UInt64? = nil
  /// True while the active terminal is in a full-screen/raw TUI — the
  /// alternate screen (vim, htop) or a running command that turned on
  /// bracketed-paste/mouse reporting (Claude Code, fzf). The input bar hides so
  /// every keystroke (and image paste) goes straight to the program.
  var terminalDirectInteraction: Bool = false
  /// Bumped whenever the input bar should grab keyboard focus.
  var inputBarFocusToken: Int = 0

  // MARK: Status bar — right group

  var cursorLine: Int? = nil
  var cursorCol: Int? = nil
  var currentLanguage: String? = nil
  var currentEncoding: String = "UTF-8"
  var currentIndent: String? = nil
  var isPreviewable: Bool = false
  var isPreviewing: Bool = false

  // MARK: Updates

  var updateAvailableVersion: String? = nil
  var updateCurrentVersion: String? = nil
  var updateURL: URL? = nil

  // MARK: Overlays

  var commandPaletteVisible: Bool = false
  var settingsLoadWarning: SettingsLoadWarning? = nil

  // MARK: Theme

  var theme: Theme = ThemeManager.theme(forName: "nord")

  // MARK: Icons

  /// Shared icon cache for themed file/folder icons in the sidebar.
  var iconCache: IconCache?

  // MARK: Callbacks (set by MainWindowController for SwiftUI → AppKit)

  var onTabSelected: ((Int) -> Void)?
  var onTabClosed: ((Int) -> Void)?
  var onTabMoved: ((Int, Int) -> Void)?
  var onTabPinToggled: ((Int) -> Void)?
  var onNewTab: (() -> Void)?
  var onShowCommandHistory: (() -> Void)?
  var onClearTerminal: (() -> Void)?
  /// Run a command from the input bar in the active terminal.
  var onRunCommand: ((String) -> Void)?
  /// Synchronously resolve a history ghost suggestion for the typed prefix.
  var onInputSuggestion: ((String) -> String?)?
  /// Most recent commands, newest first, for ↑/↓ cycling in the input bar.
  var onRecentCommands: ((Int) -> [String])?
  /// Send SIGINT to the active terminal (input-bar Stop button / ⌃C).
  var onSendInterrupt: (() -> Void)?
  /// Move keyboard focus into the terminal grid (Esc from the input bar).
  var onFocusTerminal: (() -> Void)?
  var onSidebarVisibilityChanged: ((Bool) -> Void)?
  var onPreviewToggle: (() -> Void)?
  var onOpenFile: ((String, Int?) -> Void)?
  var onNewFile: ((String) -> Void)?
  var onNewFolder: ((String) -> Void)?
  /// Sidebar action-bar buttons (act on the selected tree dir, or the root):
  /// new file, new folder.
  var onCreateFile: (() -> Void)?
  var onCreateFolder: (() -> Void)?
  var onRefreshTree: (() -> Void)?
  var onCollapseAll: (() -> Void)?
  var onFileTreeExpansionChanged: (() -> Void)?
  var onToggleHidden: (() -> Void)?
  var onOpenSettingsFile: (() -> Void)?
  var onDismissSettingsWarning: (() -> Void)?

  // MARK: Methods

  /// Replace the tab display info array. Called by TabManager.
  func refreshTabs(_ infos: [TabDisplayInfo], selectedIndex: Int) {
    self.tabDisplayInfos = infos
    self.selectedTabIndex = selectedIndex
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

  /// Expand a directory node, lazily loading its children off the main
  /// thread if they haven't been loaded yet. Shared by mouse and keyboard
  /// navigation in the sidebar.
  func expandDirectory(_ node: FileTreeNode) {
    guard node.isDirectory, !node.isExpanded else { return }
    if node.isLoaded {
      node.isExpanded = true
      rebuildFlatTree()
      onFileTreeExpansionChanged?()
      refreshGitStatusForChildren(of: node)
    } else {
      // Flip the chevron immediately so the user knows the action registered.
      node.isExpanded = true
      let showHidden = showHiddenFiles
      let path = node.path
      DispatchQueue.global(qos: .userInitiated).async { [weak self] in
        let children = FileTreeNode.buildChildren(path: path, showHidden: showHidden)
        DispatchQueue.main.async {
          guard let self else { return }
          node.children = children
          self.rebuildFlatTree()
          self.onFileTreeExpansionChanged?()
          self.refreshGitStatusForChildren(of: node)
        }
      }
    }
  }

  /// Collapse an expanded directory node.
  func collapseDirectory(_ node: FileTreeNode) {
    guard node.isDirectory, node.isExpanded else { return }
    node.isExpanded = false
    rebuildFlatTree()
    onFileTreeExpansionChanged?()
  }

  private func refreshGitStatusForChildren(of node: FileTreeNode) {
    guard let children = node.children, !children.isEmpty else { return }
    let nodePath = node.path
    let rootPath = fileTreeRootPath
    DispatchQueue.global(qos: .utility).async {
      FileTreeNode.refreshGitStatus(
        nodes: children, repoPath: rootPath, dirPath: nodePath
      )
    }
  }
}
