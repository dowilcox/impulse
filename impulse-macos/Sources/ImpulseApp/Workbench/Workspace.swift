import Foundation

/// A folder the window works in, with its own set of tabs. Folder
/// workspaces pin the file tree and git state to their root. The scratch
/// workspace has no fixed folder: its file tree follows the active tab's
/// directory, which is how a window behaves with no workspace opened.
final class Workspace {
  enum Kind: String, Codable {
    case folder
    case scratch
  }

  let id: UUID
  let kind: Kind
  /// The folder (scratch: home, used for new terminals).
  let root: String
  /// Name set by the user, replacing the folder name.
  var customName: String?
  /// The tab to return to when the workspace is shown again.
  var lastSelectedUID: Int?
  /// Whether the sidebar lists this workspace's tabs under its row.
  var isExpanded = false
  /// The repository at `root`, once resolved (folder workspaces only).
  var repository: GitRepositoryState?

  init(id: UUID = UUID(), kind: Kind, root: String, customName: String? = nil) {
    self.id = id
    self.kind = kind
    self.root = kind == .scratch ? NSHomeDirectory() : Self.normalize(root)
    self.customName = customName
  }

  var name: String {
    if let customName, !customName.isEmpty { return customName }
    switch kind {
    case .scratch: return "Scratch"
    case .folder:
      let name = (root as NSString).lastPathComponent
      return name.isEmpty ? root : name
    }
  }

  /// Where new terminals in this workspace start.
  var defaultDirectory: String { root }

  static func normalize(_ path: String) -> String {
    let standardized = ((path as NSString).expandingTildeInPath as NSString).standardizingPath
    return URL(fileURLWithPath: standardized).resolvingSymlinksInPath().path
  }
}

/// Recently opened workspace folders, newest first, for the workspace
/// switcher.
enum RecentWorkspaces {
  private static let key = "recentWorkspaceFolders"
  private static let limit = 20

  static var folders: [String] {
    UserDefaults.standard.stringArray(forKey: key) ?? []
  }

  static func note(_ folder: String) {
    guard AppState.persistenceEnabled else { return }
    var list = folders.filter { $0 != folder }
    list.insert(folder, at: 0)
    UserDefaults.standard.set(Array(list.prefix(limit)), forKey: key)
  }

  static func forget(_ folder: String) {
    UserDefaults.standard.set(folders.filter { $0 != folder }, forKey: key)
  }
}
