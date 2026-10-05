import Foundation
import ImpulseKit
import os.log

/// 1: one window's flat tab list. 2: every window, each with workspaces whose
/// tabs may be split into panes. Version 1 files are migrated on load.
private let sessionStateVersion = 2

struct SessionState: Codable {
  var version: Int = sessionStateVersion
  var windows: [SessionWindowState] = []
  var activeWindowIndex: Int?

  enum CodingKeys: String, CodingKey {
    case version
    case windows
    case activeWindowIndex = "active_window_index"
  }

  static func snapshot(windows: [SessionWindowState], activeWindowIndex: Int?) -> SessionState {
    SessionState(
      version: sessionStateVersion,
      windows: windows,
      activeWindowIndex: activeWindowIndex
    )
  }

  static func filePath() -> URL {
    Settings.settingsPath()
      .deletingLastPathComponent()
      .appendingPathComponent("session-state.json")
  }

  static func load(from url: URL = Self.filePath()) -> SessionState? {
    let data: Data
    do {
      data = try Data(contentsOf: url)
    } catch {
      if FileManager.default.fileExists(atPath: url.path) {
        os_log(.error, "Failed to read session state from '%{public}@': %{public}@",
               url.path, error.localizedDescription)
      }
      return nil
    }

    do {
      var state = try JSONDecoder().decode(SessionState.self, from: data)
      switch state.version {
      case sessionStateVersion:
        return state
      case 1:
        state.windows = state.windows.map { $0.migratedFromV1() }
        state.version = sessionStateVersion
        return state
      default:
        os_log(.error, "Unsupported session state version %d", state.version)
        return nil
      }
    } catch {
      os_log(.error, "Failed to decode session state from '%{public}@': %{public}@",
             url.path, error.localizedDescription)
      return nil
    }
  }

  var activeWindow: SessionWindowState? {
    if let activeWindowIndex, windows.indices.contains(activeWindowIndex) {
      return windows[activeWindowIndex]
    }
    return windows.first
  }

  func save() {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]

    let data: Data
    do {
      data = try encoder.encode(self)
    } catch {
      os_log(.error, "Failed to encode session state: %{public}@", error.localizedDescription)
      return
    }

    let url = Self.filePath()
    do {
      try data.write(to: url, options: .atomic)
      try FileManager.default.setAttributes(
        [.posixPermissions: 0o600],
        ofItemAtPath: url.path
      )
    } catch {
      os_log(.error, "Failed to write session state to '%{public}@': %{public}@",
             url.path, error.localizedDescription)
    }
  }
}

struct SessionWindowState: Codable {
  /// Version 1 only: the file tree root and the flat tab list.
  var projectRoot: String?
  var tabs: [SessionTabState]?
  var activeTabIndex: Int?
  var layout: SessionLayoutState?

  /// Version 2: the window's workspaces in sidebar order.
  var workspaces: [SessionWorkspaceState]?
  var activeWorkspaceIndex: Int?
  /// `NSStringFromRect` of the window frame.
  var frame: String?
  var sidebarVisible: Bool?
  var sidebarWidth: Double?

  enum CodingKeys: String, CodingKey {
    case projectRoot = "project_root"
    case tabs
    case activeTabIndex = "active_tab_index"
    case layout
    case workspaces
    case activeWorkspaceIndex = "active_workspace_index"
    case frame
    case sidebarVisible = "sidebar_visible"
    case sidebarWidth = "sidebar_width"
  }

  init(
    workspaces: [SessionWorkspaceState], activeWorkspaceIndex: Int?, frame: String?,
    sidebarVisible: Bool?, sidebarWidth: Double?
  ) {
    self.workspaces = workspaces
    self.activeWorkspaceIndex = activeWorkspaceIndex
    self.frame = frame
    self.sidebarVisible = sidebarVisible
    self.sidebarWidth = sidebarWidth
  }

  /// A version 1 window becomes one scratch workspace (whose file tree
  /// follows the active tab, as windows did then). Each old tab is a
  /// single-pane tab; old terminal splits keep only their active pane.
  func migratedFromV1() -> SessionWindowState {
    let migratedTabs: [SessionTab] = (tabs ?? []).compactMap { tab in
      switch tab.kind {
      case "terminal":
        var cwd = tab.cwd ?? ""
        if let panes = tab.panes, !panes.isEmpty {
          let index = tab.activePaneIndex ?? 0
          let pane = panes.indices.contains(index) ? panes[index] : panes[0]
          if !pane.cwd.isEmpty { cwd = pane.cwd }
        }
        return SessionTab(
          pinned: tab.pinned,
          panes: [.terminal(cwd: cwd, title: tab.title, shell: tab.shell)])
      case "editor":
        guard let path = tab.path else { return nil }
        return SessionTab(pinned: tab.pinned, panes: [.file(path: path)])
      default:
        return nil
      }
    }
    var active: Int?
    if let index = activeTabIndex, let tabs, tabs.indices.contains(index) {
      // Count only tabs that survived migration before the active one.
      active = tabs[..<index].filter { $0.kind == "terminal" || $0.path != nil }.count
    }
    return SessionWindowState(
      workspaces: [
        SessionWorkspaceState(
          kind: "scratch", root: NSHomeDirectory(), tabs: migratedTabs,
          activeTabIndex: active, fileTreeRoot: projectRoot)
      ],
      activeWorkspaceIndex: 0, frame: nil, sidebarVisible: nil, sidebarWidth: nil)
  }
}

/// A workspace and its tabs.
struct SessionWorkspaceState: Codable {
  /// "folder" or "scratch".
  var kind: String
  var root: String
  var name: String?
  var expanded: Bool?
  var tabs: [SessionTab]
  /// Index into `tabs`.
  var activeTabIndex: Int?
  /// Scratch only: where the file tree was.
  var fileTreeRoot: String?

  enum CodingKeys: String, CodingKey {
    case kind, root, name, expanded, tabs
    case activeTabIndex = "active_tab_index"
    case fileTreeRoot = "file_tree_root"
  }
}

/// A tab: one surface, or several arranged by `layout` (whose pane ids index
/// into `panes`).
struct SessionTab: Codable {
  var pinned: Bool
  var panes: [SessionSurface]
  var layout: LayoutTree<Int>?
  var focusedPane: Int?

  enum CodingKeys: String, CodingKey {
    case pinned, panes, layout
    case focusedPane = "focused_pane"
  }

  init(pinned: Bool, panes: [SessionSurface], layout: LayoutTree<Int>? = nil, focusedPane: Int? = nil)
  {
    self.pinned = pinned
    self.panes = panes
    self.layout = layout
    self.focusedPane = focusedPane
  }
}

/// A restorable surface. Review tabs aren't saved.
struct SessionSurface: Codable {
  /// "terminal" or "file" (editor or image preview, by extension).
  var kind: String
  var path: String?
  var cwd: String?
  var title: String?
  var shell: String?
  /// Editor cursor, 1-based.
  var line: Int?
  var column: Int?

  static func terminal(cwd: String, title: String?, shell: String?) -> SessionSurface {
    SessionSurface(kind: "terminal", cwd: cwd, title: title, shell: shell)
  }

  static func file(path: String, line: Int? = nil, column: Int? = nil) -> SessionSurface {
    SessionSurface(kind: "file", path: path, line: line, column: column)
  }
}

struct SessionTabState: Codable {
  var kind: String
  var path: String?
  var cwd: String?
  var title: String?
  var shell: String?
  var pinned: Bool
  var panes: [SessionTerminalPaneState]?
  var activePaneIndex: Int?
  var paneLayout: SessionTerminalPaneLayoutState?

  enum CodingKeys: String, CodingKey {
    case kind
    case path
    case cwd
    case title
    case shell
    case pinned
    case panes
    case activePaneIndex = "active_pane_index"
    case paneLayout = "pane_layout"
  }

  static func editor(path: String, pinned: Bool) -> SessionTabState {
    SessionTabState(
      kind: "editor",
      path: path,
      cwd: nil,
      title: nil,
      shell: nil,
      pinned: pinned,
      panes: nil,
      activePaneIndex: nil,
      paneLayout: nil
    )
  }

  static func terminal(
    cwd: String,
    title: String?,
    shell: String?,
    pinned: Bool,
    panes: [SessionTerminalPaneState]? = nil,
    activePaneIndex: Int? = nil,
    paneLayout: SessionTerminalPaneLayoutState? = nil
  ) -> SessionTabState {
    SessionTabState(
      kind: "terminal",
      path: nil,
      cwd: cwd,
      title: title,
      shell: shell,
      pinned: pinned,
      panes: panes,
      activePaneIndex: activePaneIndex,
      paneLayout: paneLayout
    )
  }
}

struct TerminalSessionSnapshot {
  var panes: [SessionTerminalPaneState]
  var activePaneIndex: Int?
  var paneLayout: SessionTerminalPaneLayoutState
}

struct SessionTerminalPaneState: Codable {
  var cwd: String
  var title: String?
  var shell: String?
}

indirect enum SessionTerminalPaneLayoutState: Codable {
  case pane(paneIndex: Int)
  case split(
    axis: String,
    ratio: Double,
    first: SessionTerminalPaneLayoutState,
    second: SessionTerminalPaneLayoutState
  )

  enum CodingKeys: String, CodingKey {
    case kind
    case paneIndex = "pane_index"
    case axis
    case ratio
    case first
    case second
  }

  enum Kind: String, Codable {
    case pane
    case split
  }

  func encode(to encoder: Encoder) throws {
    var container = encoder.container(keyedBy: CodingKeys.self)
    switch self {
    case .pane(let paneIndex):
      try container.encode(Kind.pane, forKey: .kind)
      try container.encode(paneIndex, forKey: .paneIndex)
    case .split(let axis, let ratio, let first, let second):
      try container.encode(Kind.split, forKey: .kind)
      try container.encode(axis, forKey: .axis)
      try container.encode(ratio, forKey: .ratio)
      try container.encode(first, forKey: .first)
      try container.encode(second, forKey: .second)
    }
  }

  init(from decoder: Decoder) throws {
    let container = try decoder.container(keyedBy: CodingKeys.self)
    let kind = try container.decode(Kind.self, forKey: .kind)
    switch kind {
    case .pane:
      self = .pane(paneIndex: try container.decode(Int.self, forKey: .paneIndex))
    case .split:
      self = .split(
        axis: try container.decode(String.self, forKey: .axis),
        ratio: try container.decode(Double.self, forKey: .ratio),
        first: try container.decode(SessionTerminalPaneLayoutState.self, forKey: .first),
        second: try container.decode(SessionTerminalPaneLayoutState.self, forKey: .second)
      )
    }
  }
}

struct SessionLayoutState: Codable {
  var kind: String
  var tabIndices: [Int]
  var activeTabIndex: Int?

  enum CodingKeys: String, CodingKey {
    case kind
    case tabIndices = "tab_indices"
    case activeTabIndex = "active_tab_index"
  }

  static func tabGroup(tabIndices: [Int], activeTabIndex: Int?) -> SessionLayoutState {
    SessionLayoutState(
      kind: "tab_group",
      tabIndices: tabIndices,
      activeTabIndex: activeTabIndex
    )
  }
}
