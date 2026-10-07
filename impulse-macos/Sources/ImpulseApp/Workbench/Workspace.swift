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
  /// The folder (scratch: the Scratch folder setting, else home).
  let root: String
  /// Name set by the user, replacing the folder name.
  var customName: String?
  /// The tab to return to when the workspace is shown again.
  var lastSelectedUID: Int?
  /// Whether the sidebar lists this workspace's tabs under its row.
  var isExpanded = false
  /// The repository at `root`, once resolved (folder workspaces only).
  var repository: GitRepositoryState?
  /// A linked worktree made by "New Task" (or by hand), not a main checkout.
  var isTask = false
  /// Ports that processes in this workspace's terminals listen on.
  var ports: [ListeningPort] = []

  init(id: UUID = UUID(), kind: Kind, root: String, customName: String? = nil) {
    self.id = id
    self.kind = kind
    self.root = kind == .scratch ? Self.scratchRoot : Self.normalize(root)
    self.customName = customName
  }

  /// Where Scratch starts: the "Scratch folder" setting when it names an
  /// existing folder, else home.
  static var scratchRoot: String {
    let setting = SettingsStore.shared.settings.scratchDirectory.trimmingCharacters(in: .whitespaces)
    guard !setting.isEmpty else { return NSHomeDirectory() }
    let path = normalize(setting)
    var isDirectory: ObjCBool = false
    guard FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory), isDirectory.boolValue else {
      return NSHomeDirectory()
    }
    return path
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

  /// Where new terminals in this workspace start (Scratch follows the
  /// setting as it changes).
  var defaultDirectory: String { kind == .scratch ? Self.scratchRoot : root }

  /// Workspaces with the same group (worktrees of one repository) sit
  /// together in the sidebar.
  var sidebarGroup: String { Self.sidebarGroup(repository: repository, id: id) }

  static func sidebarGroup(repository: GitRepositoryState?, id: UUID) -> String {
    repository?.snapshot?.commonDir ?? id.uuidString
  }

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
