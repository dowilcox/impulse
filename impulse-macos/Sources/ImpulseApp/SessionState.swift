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
  /// Height the workspaces section was dragged to (nil: fits its rows).
  var workspacesHeight: Double?

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
    case workspacesHeight = "workspaces_height"
  }

  init(
    workspaces: [SessionWorkspaceState], activeWorkspaceIndex: Int?, frame: String?,
    sidebarVisible: Bool?, sidebarWidth: Double?, workspacesHeight: Double? = nil
  ) {
    self.workspaces = workspaces
    self.activeWorkspaceIndex = activeWorkspaceIndex
    self.frame = frame
    self.sidebarVisible = sidebarVisible
    self.sidebarWidth = sidebarWidth
    self.workspacesHeight = workspacesHeight
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

/// A restorable surface.
struct SessionSurface: Codable {
  /// "terminal", "file" (editor or image preview, by extension), "review" or
  /// "history" (`path` is the repository root).
  var kind: String
  var path: String?
  var cwd: String?
  var title: String?
  var shell: String?
  /// Editor cursor, 1-based.
  var line: Int?
  var column: Int?
  /// Terminal: file name of its saved output under `SessionScrollback`.
  var scrollback: String?
  /// Terminal: a command that resumes the agent that was running in it.
  var resume: String?
  /// Terminal output carried in memory (closed tabs, or loaded from
  /// `scrollback` on restore). Not written to the session file.
  var transcript: String?
  /// Review: the scope it showed.
  var scope: DiffScope?
  /// History: the file or folder it followed (repository-relative).
  var subpath: String?

  enum CodingKeys: String, CodingKey {
    case kind, path, cwd, title, shell, line, column, scrollback, resume, scope, subpath
  }

  static func terminal(cwd: String, title: String?, shell: String?, transcript: String? = nil)
    -> SessionSurface
  {
    SessionSurface(kind: "terminal", cwd: cwd, title: title, shell: shell, transcript: transcript)
  }

  static func file(path: String, line: Int? = nil, column: Int? = nil) -> SessionSurface {
    SessionSurface(kind: "file", path: path, line: line, column: column)
  }

  static func review(root: String, scope: DiffScope) -> SessionSurface {
    SessionSurface(kind: "review", path: root, scope: scope)
  }

  static func history(root: String, path: String?) -> SessionSurface {
    SessionSurface(kind: "history", path: root, subpath: path)
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

/// Terminal output saved beside the session file, one file per terminal, so
/// the session JSON stays small. Files no session refers to are removed on
/// each save.
enum SessionScrollback {
  static var directory: URL {
    SessionState.filePath().deletingLastPathComponent().appendingPathComponent("scrollback")
  }

  /// Write every in-memory transcript to a file and point the surface at it.
  static func store(_ state: inout SessionState) {
    let fm = FileManager.default
    try? fm.createDirectory(
      at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
    var kept = Set<String>()
    for w in state.windows.indices {
      guard var workspaces = state.windows[w].workspaces else { continue }
      for ws in workspaces.indices {
        for t in workspaces[ws].tabs.indices {
          for p in workspaces[ws].tabs[t].panes.indices {
            var surface = workspaces[ws].tabs[t].panes[p]
            guard let text = surface.transcript, !text.isEmpty else { continue }
            let name = UUID().uuidString + ".ansi"
            let url = directory.appendingPathComponent(name)
            do {
              try Data(text.utf8).write(to: url, options: .atomic)
              try fm.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
              surface.scrollback = name
              kept.insert(name)
            } catch {
              continue
            }
            workspaces[ws].tabs[t].panes[p] = surface
          }
        }
      }
      state.windows[w].workspaces = workspaces
    }
    // Forget output from earlier saves.
    for name in (try? fm.contentsOfDirectory(atPath: directory.path)) ?? [] where !kept.contains(name) {
      try? fm.removeItem(at: directory.appendingPathComponent(name))
    }
  }

  /// Read saved output for a session's terminals (off the main thread).
  static func load(into workspaces: inout [SessionWorkspaceState]) {
    for ws in workspaces.indices {
      for t in workspaces[ws].tabs.indices {
        for p in workspaces[ws].tabs[t].panes.indices {
          guard let name = workspaces[ws].tabs[t].panes[p].scrollback,
            !name.contains("/"),
            let data = FileManager.default.contents(
              atPath: directory.appendingPathComponent(name).path)
          else { continue }
          workspaces[ws].tabs[t].panes[p].transcript = String(decoding: data, as: UTF8.self)
        }
      }
    }
  }
}
