import AppKit
import ImpulseGit
import ImpulseKit

/// The app's trusted folders (see `ImpulseKit.WorkspaceTrust`): language
/// servers, formatters and commands on save, and background fetch only run
/// in them.
enum Trust {
  static let shared = WorkspaceTrust(
    file: AppState.persistenceEnabled ? AppPaths.dataDirectory.appendingPathComponent("trusted-folders.json") : nil)

  /// Folders the user kept restricted this session (not asked again), with
  /// a prompt up, or already told about.
  static var declined: Set<String> = []
  static var prompting: Set<String> = []
  static var noted: Set<String> = []
  /// Snapshot checks of restricted mode (snapshots otherwise never ask).
  static var enabledForSnapshot = false

  /// Whether to ask about folders: the setting, except in snapshots.
  static func shouldAsk(_ settings: Settings) -> Bool {
    settings.askToTrustFolders && (!DebugSnapshot.isActive || enabledForSnapshot)
  }
}

/// Workspace trust: the prompt when a folder opens, the restricted-mode
/// indicator, and keeping language servers in step with what's trusted.
extension MainWindowController {

  /// A folder was opened as a workspace: ask about it, unless it's trusted
  /// or the user kept it restricted this session.
  func requestTrustIfNeeded(forFolder folder: String) {
    let folder = WorkspaceTrust.normalize(folder)
    guard Trust.shared.isEnabled, !Trust.shared.isTrusted(folder), !Trust.declined.contains(folder) else {
      updateRestrictedIndicator()
      return
    }
    presentTrustPrompt(for: folder)
  }

  func presentTrustPrompt(for folder: String) {
    let folder = WorkspaceTrust.normalize(folder)
    guard let window, Trust.prompting.insert(folder).inserted else { return }
    updateRestrictedIndicator()
    let name = (folder as NSString).lastPathComponent
    let parent = (folder as NSString).deletingLastPathComponent

    let alert = NSAlert()
    alert.messageText = "Do you trust the files in “\(name)”?"
    alert.informativeText = """
      Impulse can run code from this folder on its own: language servers (some run a project's \
      build scripts and tools), formatters and commands on save, and background git fetch. \
      Until you trust it, they stay off. The terminal and editing work as usual.

      \(TabManager.abbreviateHomePath(folder))
      """
    alert.addButton(withTitle: "Trust Folder")
    alert.addButton(withTitle: "Stay Restricted")
    // Where projects are kept: trust them all at once.
    var parentBox: NSButton?
    if Self.offersParent(parent) {
      let box = NSButton(
        checkboxWithTitle: "Trust everything in “\(TabManager.abbreviateHomePath(parent))”", target: nil, action: nil)
      box.sizeToFit()
      alert.accessoryView = box
      parentBox = box
    }
    alert.beginSheetModal(for: window.attachedSheet ?? window) { [weak self] response in
      Trust.prompting.remove(folder)
      if response == .alertFirstButtonReturn {
        Trust.shared.trust(parentBox?.state == .on ? parent : folder)
        Trust.declined.remove(folder)
        Self.trustDidChange()
      } else {
        Trust.declined.insert(folder)
        self?.updateRestrictedIndicator()
      }
    }
  }

  /// Not the home folder or the disk: trusting those trusts everything.
  private static func offersParent(_ parent: String) -> Bool {
    let normalized = WorkspaceTrust.normalize(parent)
    return !["/", "/Users", "/Volumes", WorkspaceTrust.normalize(NSHomeDirectory())].contains(normalized)
  }

  /// "Trust This Folder…": the active folder workspace, else the active
  /// file's repository or folder.
  func trustActiveFolder() {
    guard let folder = trustFolderForActiveContext() else {
      toasts.show(Toast(kind: .info, message: "Open a folder or a file to trust it."))
      return
    }
    if Trust.shared.isTrusted(folder), Trust.shared.isEnabled {
      toasts.show(
        Toast(kind: .info, message: "“\((folder as NSString).lastPathComponent)” is already trusted."))
      return
    }
    presentTrustPrompt(for: folder)
  }

  /// "Restrict This Folder".
  func restrictActiveFolder() {
    guard let folder = trustFolderForActiveContext() else { return }
    let name = (folder as NSString).lastPathComponent
    if let above = Trust.shared.revoke(folder) {
      let aboveName = (above as NSString).lastPathComponent
      toasts.show(
        Toast(
          kind: .info, message: "“\(name)” is trusted as part of “\(aboveName)”.", actionTitle: "Restrict \(aboveName)",
          action: {
            Trust.shared.revoke(above)
            Self.trustDidChange()
          }, lifetime: 10))
      return
    }
    Trust.declined.insert(WorkspaceTrust.normalize(folder))
    Self.trustDidChange()
    toasts.show(
      Toast(kind: .info, message: "“\(name)” is restricted: language servers, formatters and background fetch are off."))
  }

  /// "Forget Trusted Folders".
  func forgetTrustedFolders() {
    Trust.shared.revokeAll()
    Self.trustDidChange()
    toasts.show(Toast(kind: .info, message: "Every folder is restricted until you trust it again."))
  }

  func trustFolderForActiveContext() -> String? {
    let workspace = tabManager.activeWorkspace
    if workspace.kind == .folder { return workspace.root }
    return tabManager.selectedEditor?.filePath.map(Self.trustFolder(forFile:))
  }

  /// What trusting a lone file means: its repository, else its folder.
  static func trustFolder(forFile path: String) -> String {
    let directory = (path as NSString).deletingLastPathComponent
    return GitClient.repoRoot(forPath: directory) ?? directory
  }

  /// Language servers stayed off for a file outside the trusted folders:
  /// say so once per folder (not while its prompt is up or after a no).
  func noteRestrictedFile(_ path: String, language: String) {
    guard core.lspHasServers(languageId: language) else { return }
    let folder = WorkspaceTrust.normalize(Self.trustFolder(forFile: path))
    let workspaceRoot = tabManager.activeWorkspace.kind == .folder
      ? WorkspaceTrust.normalize(tabManager.activeWorkspace.root) : nil
    let asked = [folder, workspaceRoot].compactMap { $0 }
    guard !asked.contains(where: { Trust.prompting.contains($0) || Trust.declined.contains($0) }),
      Trust.noted.insert(folder).inserted
    else { return }
    toasts.show(
      Toast(
        kind: .info,
        message: "Language servers are off in “\((folder as NSString).lastPathComponent)”: it isn't a trusted folder.",
        actionTitle: "Trust…", action: { [weak self] in self?.presentTrustPrompt(for: folder) }, lifetime: 10))
  }

  /// Bring every window in line with the trusted folders: language servers
  /// for files that became trusted, none for those that didn't, and the
  /// restricted-mode indicator.
  static func trustDidChange() {
    for controller in AppDelegate.shared?.allWindowControllers ?? [] { controller.applyTrust() }
    // Servers already running for a folder that's restricted now stop.
    AppDelegate.shared?.core.lspShutdownServers { rootPath in !Trust.shared.isTrusted(rootPath) }
  }

  func applyTrust() {
    for case .editor(let editor) in tabManager.allSurfaces {
      guard let path = editor.filePath else { continue }
      let uri = filePathToUri(path)
      if Trust.shared.isTrusted(path) {
        if !lspOpenFiles.contains(uri) { lspDidOpenIfNeeded(path: path) }
      } else if lspOpenFiles.contains(uri) {
        lspDidClose(editor: editor)
        editor.applyDiagnostics(uri: uri, markers: [])
        windowModel.problemsByPath[path] = nil
      }
    }
    updateRestrictedIndicator()
  }

  /// The status bar's "Restricted" item, for a folder workspace that isn't
  /// trusted.
  func updateRestrictedIndicator() {
    let workspace = tabManager.activeWorkspace
    let restricted = workspace.kind == .folder && !Trust.shared.isTrusted(workspace.root)
    if windowModel.workspaceRestricted != restricted { windowModel.workspaceRestricted = restricted }
  }
}
