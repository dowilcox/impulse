import Foundation
import os.log

private let sessionStateVersion = 1

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

  static func load() -> SessionState? {
    let url = Self.filePath()
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
      let state = try JSONDecoder().decode(SessionState.self, from: data)
      guard state.version == sessionStateVersion else {
        os_log(.error, "Unsupported session state version %d", state.version)
        return nil
      }
      return state
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
  var projectRoot: String?
  var tabs: [SessionTabState]
  var activeTabIndex: Int?
  var layout: SessionLayoutState

  enum CodingKeys: String, CodingKey {
    case projectRoot = "project_root"
    case tabs
    case activeTabIndex = "active_tab_index"
    case layout
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
