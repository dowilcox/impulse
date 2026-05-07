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

  static func editor(path: String, pinned: Bool) -> SessionTabState {
    SessionTabState(
      kind: "editor",
      path: path,
      cwd: nil,
      title: nil,
      shell: nil,
      pinned: pinned
    )
  }

  static func terminal(
    cwd: String,
    title: String?,
    shell: String?,
    pinned: Bool
  ) -> SessionTabState {
    SessionTabState(
      kind: "terminal",
      path: nil,
      cwd: cwd,
      title: title,
      shell: shell,
      pinned: pinned
    )
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
