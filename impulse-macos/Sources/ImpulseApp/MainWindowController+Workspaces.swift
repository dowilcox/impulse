import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI
import os.log

extension MainWindowController {

  // MARK: - File Tree Cache (LRU)

  /// Inserts a key into the file tree cache, evicting the oldest entry if
  /// the cache exceeds `fileTreeCacheMaxSize`.
  func fileTreeCacheInsert(key: String, nodes: [FileTreeNode]) {
    // Remove existing entry from order tracking if present.
    if let idx = fileTreeCacheOrder.firstIndex(of: key) {
      fileTreeCacheOrder.remove(at: idx)
    }
    fileTreeCacheOrder.append(key)
    fileTreeCache[key] = nodes

    // Evict oldest entries if over the limit.
    while fileTreeCacheOrder.count > fileTreeCacheMaxSize {
      let evicted = fileTreeCacheOrder.removeFirst()
      fileTreeCache.removeValue(forKey: evicted)
    }
  }

  /// Touches a cache key to mark it as recently used (moves to end of order).
  private func fileTreeCacheTouch(key: String) {
    if let idx = fileTreeCacheOrder.firstIndex(of: key) {
      fileTreeCacheOrder.remove(at: idx)
      fileTreeCacheOrder.append(key)
    }
  }

  // MARK: - File Tree Expansion Helpers

  /// Collects paths of all expanded directories in the tree.
  static func collectExpandedPaths(_ nodes: [FileTreeNode]) -> Set<String> {
    var paths = Set<String>()
    for node in nodes {
      if node.isDirectory && node.isExpanded {
        paths.insert(node.path)
        if let children = node.children {
          paths.formUnion(collectExpandedPaths(children))
        }
      }
    }
    return paths
  }

  /// Restores expanded state for directories whose paths are in the set.
  /// Loads children for expanded dirs so the tree shows content.
  static func restoreExpandedPaths(
    _ paths: Set<String>, in nodes: [FileTreeNode], showHidden: Bool
  ) {
    for node in nodes where node.isDirectory && paths.contains(node.path) {
      node.isExpanded = true
      if !node.isLoaded {
        node.loadChildren(showHidden: showHidden)
      }
      if let children = node.children {
        restoreExpandedPaths(paths, in: children, showHidden: showHidden)
      }
    }
  }

  // MARK: - File Tree Root Switching

  /// Switches the sidebar file tree to a new directory. Caches the current
  /// tree, shows a cached tree instantly if available, then rebuilds from
  /// disk on a background queue and updates the status bar with the git
  /// branch.
  func switchFileTreeRoot(_ dir: String, updateStatusBar: Bool = true) {
    // Cache current tree before switching away.
    if !fileTreeRootPath.isEmpty {
      fileTreeCacheInsert(key: fileTreeRootPath, nodes: fileTreeData.rootNodes)
    }
    fileTreeRootPath = dir
    windowModel.fileTreeRootPath = dir
    // Drop any active search and its results so stale matches from the old
    // root don't linger against the new project.
    windowModel.resetSearch()

    let shellName = LoginShell.defaultShellName()
    if updateStatusBar {
      windowModel.currentCwd = dir
      windowModel.shellName = shellName
      bindRepository(forDirectory: dir)
    }

    // Show cached tree instantly if available. Skip git refresh since the
    // background rebuild below will fetch fresh git status anyway.
    if let cached = fileTreeCache[dir] {
      fileTreeData.updateTree(nodes: cached, rootPath: dir, skipGitRefresh: true)
      fileTreeCacheTouch(key: dir)
      windowModel.updateFileTree(cached, rootPath: dir)
    }

    // Refresh from disk in the background.
    let showHidden = fileTreeData.showHidden
    DispatchQueue.global(qos: .userInitiated).async {
      let nodes = FileTreeNode.buildTree(rootPath: dir, showHidden: showHidden)
      DispatchQueue.main.async { [weak self] in
        guard let self else { return }
        guard self.fileTreeRootPath == dir else { return }
        self.fileTreeData.updateTree(nodes: nodes, rootPath: dir)
        self.fileTreeCacheInsert(key: dir, nodes: nodes)
        self.windowModel.updateFileTree(nodes, rootPath: dir)
      }
    }
  }

  // MARK: - Active repository

  /// Make the repository containing `dir` the window's active repository
  /// (titlebar breadcrumb, status bar, diff pill all follow it live).
  /// In a folder workspace the repository is the folder's, whatever
  /// directory the active tab is in.
  func bindRepository(forDirectory dir: String) {
    let workspace = tabManager.activeWorkspace
    let anchor = workspace.kind == .folder ? workspace.root : dir
    repositoryAnchor = anchor
    guard !anchor.isEmpty else {
      setRepository(nil)
      return
    }
    if let current = windowModel.repository,
      anchor == current.root || anchor.hasPrefix(current.root + "/")
    {
      return
    }
    GitRepositoryStore.shared.resolve(directory: anchor) { [weak self] state in
      guard let self, self.repositoryAnchor == anchor else { return }
      self.setRepository(state)
    }
  }

  // MARK: - Workspaces

  /// Outside folder workspaces the file tree and repository follow the
  /// active tab's directory.
  var followsActiveDirectory: Bool { tabManager.activeWorkspace.kind == .scratch }

  /// ⌃Tab / ⌃⇧Tab: cycle through the active workspace's tabs.
  func cycleTab(by step: Int) {
    let visible = tabManager.visibleTabIndices
    guard visible.count > 1, let position = visible.firstIndex(of: tabManager.selectedIndex)
    else { return }
    tabManager.selectTab(index: visible[(position + step + visible.count) % visible.count])
  }

  func activeWorkspaceDidChange() {
    let workspace = tabManager.activeWorkspace
    if workspace.kind == .folder, workspace.root != fileTreeRootPath {
      switchFileTreeRoot(workspace.root, updateStatusBar: false)
    }
    updateStatusBar()
  }

  /// Choose a folder to open as a workspace.
  func presentOpenWorkspacePanel() {
    guard let window else { return }
    let panel = NSOpenPanel()
    panel.canChooseFiles = false
    panel.canChooseDirectories = true
    panel.allowsMultipleSelection = false
    panel.prompt = "Open Workspace"
    panel.message = "Choose a folder to work in. It gets its own tabs, file tree and git state."
    panel.beginSheetModal(for: window) { [weak self] response in
      guard response == .OK, let url = panel.url else { return }
      self?.tabManager.openWorkspace(folder: url.path)
    }
  }

  /// Close a workspace after confirming its unsaved files and running
  /// processes.
  func requestCloseWorkspace(_ id: UUID, then closed: (() -> Void)? = nil) {
    let surfaces = tabManager.tabIndices(inWorkspace: id).flatMap { tabManager.tabs[$0].surfaces }
    confirmClosing(surfaces) { [weak self] in
      guard let self else { return }
      for surface in surfaces { self.willCloseSurface(surface) }
      self.tabManager.closeWorkspace(id)
      closed?()
    }
  }

  func presentRenameWorkspace(_ id: UUID) {
    guard let window, let workspace = tabManager.workspace(id) else { return }
    let alert = NSAlert()
    alert.messageText = "Rename Workspace"
    alert.informativeText = "Leave empty to use the folder name."
    alert.addButton(withTitle: "Rename")
    alert.addButton(withTitle: "Cancel")
    let field = NSTextField(string: workspace.customName ?? workspace.name)
    field.frame = NSRect(x: 0, y: 0, width: 260, height: 24)
    alert.accessoryView = field
    alert.window.initialFirstResponder = field
    alert.beginSheetModal(for: window) { [weak self] response in
      guard response == .alertFirstButtonReturn else { return }
      self?.tabManager.renameWorkspace(id, to: field.stringValue)
    }
  }

  private func setRepository(_ state: GitRepositoryState?) {
    guard windowModel.repository !== state else { return }
    if let previous = repositoryListener {
      previous.state.removeChangeListener(previous.token)
      repositoryListener = nil
    }
    windowModel.repository = state
    if let state {
      // Working-tree and index changes recolor the file tree's git badges.
      let token = state.addChangeListener { [weak self] change in
        guard let self else { return }
        self.fileTreeData.refreshGitStatus()
        // Staging/committing/switching changes the editor's diff base.
        if !change.isDisjoint(with: [.index, .refs]), let editor = self.tabManager.selectedEditor {
          self.applyGitDiffDecorations(editor: editor)
        }
      }
      repositoryListener = (state, token)
    }
    if repositoryObservation == nil {
      repositoryObservation = ObservationLoop(owner: self) { [weak self] in
        self?.syncRepositoryToModel()
      }
    }
  }

  private func syncRepositoryToModel() {
    let snapshot = windowModel.repository?.snapshot
    let branch = snapshot.map { snap in
      snap.branch ?? snap.headOid.map { String($0.prefix(7)) } ?? ""
    }
    windowModel.gitBranch = (branch?.isEmpty ?? true) ? nil : branch
    windowModel.reviewChangedFileCount = snapshot?.changedFileCount ?? 0
    windowModel.reviewAddedLines = snapshot?.totalAdded ?? 0
    windowModel.reviewRemovedLines = snapshot?.totalRemoved ?? 0
    if let repository = windowModel.repository {
      PullRequestMonitor.shared.refresh(repository)
    }
  }

  /// Open the branch's pull request, or start one in the browser.
  func openOrCreatePullRequest() {
    guard let repository = windowModel.repository else {
      toasts.show(Toast(kind: .info, message: "Not in a git repository."))
      return
    }
    if let pr = repository.pullRequest, let url = URL(string: pr.url) {
      NSWorkspace.shared.open(url)
      return
    }
    guard PullRequestMonitor.shared.isAvailable else {
      toasts.show(Toast(kind: .info, message: "Install the GitHub CLI (gh) to work with pull requests."))
      return
    }
    PullRequestMonitor.shared.createInBrowser(root: repository.root) { [weak self] ok in
      if !ok {
        self?.toasts.show(
          Toast(kind: .warning, message: "gh couldn't start a pull request (is the branch pushed and gh signed in?)."))
      }
    }
  }

  /// A draft PR for the current branch, titled and described from its
  /// commits. The branch must be pushed (offers to publish it first).
  func createDraftPullRequest() {
    guard let repository = windowModel.repository, let snapshot = repository.snapshot else {
      toasts.show(Toast(kind: .info, message: "Not in a git repository."))
      return
    }
    if let pr = repository.pullRequest, pr.state == .open {
      toasts.show(Toast(kind: .info, message: "This branch already has #\(pr.number)."))
      return
    }
    guard snapshot.upstream != nil else {
      toasts.show(
        Toast(
          kind: .info, message: "Publish the branch first, then create the pull request.",
          actionTitle: "Publish",
          action: { [weak self] in
            guard let self else { return }
            GitActions(repository: repository, host: self).push { [weak self] in
              self?.createDraftPullRequest()
            }
          }, lifetime: 12))
      return
    }
    toasts.show(Toast(kind: .info, message: "Creating a draft pull request…"))
    PullRequestMonitor.shared.createDraft(root: repository.root) { [weak self] result in
      switch result {
      case .success(let url):
        PullRequestMonitor.shared.refresh(repository, force: true)
        self?.toasts.show(
          Toast(
            kind: .success, message: "Created a draft pull request", actionTitle: url.isEmpty ? nil : "Open",
            action: URL(string: url).map { link in { NSWorkspace.shared.open(link) } }, lifetime: 12))
      case .failure(let message):
        self?.toasts.show(Toast(kind: .warning, message: "gh: \(message)", lifetime: 12))
      }
    }
  }

  /// Switch the active tab's repository to `branch` with `git switch`, off the
  /// main thread. On failure, explains why in a sheet (dirty tree, unknown
  /// branch, index.lock held by another process, ...).
  func switchBranch(to branch: String) {
    guard let repository = windowModel.repository else { return }
    GitActions(repository: repository, host: self).switchBranch(branch)
  }

  /// Open the branch switcher (the palette in branch mode).
  func showBranchSwitcher() {
    showPalette(prefix: "b:")
  }

  /// Shows a git failure as a window-modal sheet: the plain-English message,
  /// with git's own output as detail.
  func presentGitError(_ error: GitOperationError, title: String) {
    guard let window else { return }
    let alert = NSAlert()
    alert.alertStyle = .warning
    alert.messageText = title
    let detail = (error.output ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    let isGeneric: Bool = {
      if case .cli(let cli) = error { return cli.kind == .other }
      return false
    }()
    alert.informativeText =
      detail.isEmpty || isGeneric ? error.message : "\(error.message)\n\n\(detail)"
    alert.addButton(withTitle: "OK")
    alert.beginSheetModal(for: window)
  }
}
