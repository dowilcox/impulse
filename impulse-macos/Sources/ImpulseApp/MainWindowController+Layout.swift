import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI
import os.log

extension MainWindowController {

  // MARK: - Layout

  func setupLayout() {
    guard let contentView = window?.contentView else { return }

    // Wire SwiftUI callbacks → AppKit actions
    windowModel.onTabSelected = { [weak self] index in
      self?.tabManager.selectTab(index: index)
    }
    windowModel.onTabClosed = { [weak self] index in
      if let handler = self?.tabManager.tabCloseHandler {
        handler(index)
      } else {
        self?.tabManager.closeTab(index: index)
      }
    }
    windowModel.onNewTab = { [weak self] in
      self?.tabManager.addTerminalTab()
    }
    windowModel.onShowCommandHistory = { [weak self] in
      self?.showPalette(prefix: "h:")
    }
    windowModel.onClearTerminal = { [weak self] in
      self?.tabManager.selectedTerminal?.activeTerminal?.clearScreen()
    }
    windowModel.onRunCommand = { [weak self] command in
      self?.tabManager.selectedTerminal?.activeTerminal?.runCommand(command)
    }
    windowModel.onSwitchBranch = { [weak self] branch in
      self?.switchBranch(to: branch)
    }
    windowModel.onSendSecureInput = { [weak self] text in
      self?.tabManager.selectedTerminal?.activeTerminal?.sendSecureLine(text)
    }
    windowModel.onInputSuggestion = { [weak self] text in
      self?.tabManager.selectedTerminal?.activeTerminal?.historySuggestion(for: text)
    }
    windowModel.onIsKnownCommand = { [weak self] word in
      guard let terminal = self?.tabManager.selectedTerminal?.activeTerminal else { return nil }
      return terminal.commandLookup.isKnown(word, cwd: terminal.currentWorkingDirectory)
    }
    windowModel.onCompletionCandidates = { [weak self] text in
      self?.tabManager.selectedTerminal?.activeTerminal?.completionCandidates(for: text)
    }
    windowModel.onRecentCommands = { [weak self] limit in
      self?.tabManager.selectedTerminal?.activeTerminal?.recentCommands(limit: limit) ?? []
    }
    windowModel.onSendInterrupt = { [weak self] in
      self?.tabManager.selectedTerminal?.activeTerminal?.sendInterrupt()
    }
    windowModel.onFocusTerminal = { [weak self] in
      self?.tabManager.selectedTerminal?.activeTerminal?.focus()
    }
    windowModel.onSelectBlocks = { [weak self] in
      self?.tabManager.selectedTerminal?.activeTerminal?.beginBlockSelection() ?? false
    }
    windowModel.onTabMoved = { [weak self] from, to in
      self?.tabManager.moveTab(from: from, to: to)
    }
    windowModel.onTabPinToggled = { [weak self] index in
      self?.tabManager.togglePin(index: index)
    }
    windowModel.onPreviewToggle = { [weak self] in
      self?.previewButtonClicked(nil)
    }
    windowModel.onPreviewFile = { path in
      NotificationCenter.default.post(
        name: .impulseOpenFile, object: nil, userInfo: ["path": path, "preview": true])
    }
    windowModel.onKeepTab = { [weak self] index in self?.tabManager.keepPreview(at: index) }
    windowModel.onOpenFile = { path, line in
      NotificationCenter.default.post(
        name: .impulseOpenFile,
        object: nil,
        userInfo: ["path": path, "line": line as Any]
      )
    }
    windowModel.onShowHistory = { [weak self] path in
      self?.showHistory(path: path)
    }
    windowModel.onMentionInAgent = { [weak self] path in
      guard let self else { return }
      self.sendToBestAgent("@" + self.relativeToWorkspace(path) + " ")
    }
    windowModel.onOpenFileBeside = { path in
      NotificationCenter.default.post(
        name: .impulseOpenFile, object: nil, userInfo: ["path": path, "beside": true])
    }
    windowModel.onRefreshTree = { [weak self] in
      guard let self else { return }
      let root = self.fileTreeRootPath
      let showHidden = self.windowModel.showHiddenFiles
      guard !root.isEmpty else { return }
      // Collect expanded paths to restore after rebuild.
      let expandedPaths = Self.collectExpandedPaths(self.windowModel.fileTreeNodes)
      DispatchQueue.global(qos: .userInitiated).async {
        let nodes = FileTreeNode.buildTree(rootPath: root, showHidden: showHidden)
        // Restore expanded state and load children for expanded dirs.
        Self.restoreExpandedPaths(expandedPaths, in: nodes, showHidden: showHidden)
        FileTreeNode.refreshGitStatus(nodes: nodes, repoPath: root, dirPath: root)
        DispatchQueue.main.async { [weak self] in
          guard let self else { return }
          self.fileTreeData.showHidden = showHidden
          self.fileTreeData.updateTree(nodes: nodes, rootPath: root)
          self.windowModel.updateFileTree(nodes)
          self.fileTreeCacheInsert(key: root, nodes: nodes)
        }
      }
    }
    windowModel.onCollapseAll = { [weak self] in
      guard let self else { return }
      self.fileTreeData.collapseAll()
      self.windowModel.updateFileTree(self.fileTreeData.rootNodes, rootPath: self.fileTreeRootPath)
      self.fileTreeCacheInsert(key: self.fileTreeRootPath, nodes: self.fileTreeData.rootNodes)
    }
    windowModel.onFileTreeExpansionChanged = { [weak self] in
      guard let self else { return }
      self.fileTreeData.persistCurrentExpandedPaths()
      self.fileTreeCacheInsert(key: self.fileTreeRootPath, nodes: self.windowModel.fileTreeNodes)
    }
    windowModel.onToggleHidden = { [weak self] in
      guard let self else { return }
      self.windowModel.showHiddenFiles.toggle()
      let showHidden = self.windowModel.showHiddenFiles
      let root = self.fileTreeRootPath
      self.settings.sidebarShowHidden = showHidden
      guard !root.isEmpty else { return }
      DispatchQueue.global(qos: .userInitiated).async {
        let nodes = FileTreeNode.buildTree(rootPath: root, showHidden: showHidden)
        FileTreeNode.refreshGitStatus(nodes: nodes, repoPath: root, dirPath: root)
        DispatchQueue.main.async { [weak self] in
          guard let self else { return }
          self.fileTreeData.showHidden = showHidden
          self.fileTreeData.updateTree(nodes: nodes, rootPath: root)
          self.windowModel.updateFileTree(nodes)
          self.fileTreeCacheInsert(key: root, nodes: nodes)
        }
      }
    }
    windowModel.onShowProblems = { [weak self] in self?.showProblems() }
    windowModel.onOpenComposer = { [weak self] in self?.toggleAgentComposer() }
    windowModel.agentTurns = { id in
      AgentCheckpoints.shared.turns(terminalID: id).enumerated().reversed().map {
        AgentTurnItem(id: $0.offset, started: $0.element.start.date, finished: $0.element.end != nil)
      }
    }
    windowModel.onReviewAgentTurnAt = { [weak self] id, index in
      let turns = AgentCheckpoints.shared.turns(terminalID: id)
      guard let self, turns.indices.contains(index) else { return }
      let turn = turns[index]
      GitRepositoryStore.shared.resolve(directory: turn.repoRoot) { [weak self] state in
        guard let self, let state else { return }
        self.tabManager.addReviewTab(repository: state, scope: turn.scope, focusPath: nil, host: self)
      }
    }
    windowModel.onRestoreAgentTurn = { [weak self] id, index in self?.restoreAgentTurn(terminalID: id, index: index) }
    windowModel.onReplaceAll = { [weak self] in self?.replaceAllInProject() }
    windowModel.onOpenSettingsFile = { [weak self] in
      self?.openSettingsFile()
    }
    windowModel.onDismissSettingsWarning = { [weak self] in
      self?.windowModel.settingsLoadWarning = nil
    }
    windowModel.onNewFile = { [weak self] (dirPath: String) in
      guard let self, !dirPath.isEmpty else { return }
      NameInputDialog.show(
        title: "New File",
        message: "Enter a name for the new file:",
        placeholder: "untitled",
        defaultValue: ""
      ) { [weak self] name in
        guard let self, !name.isEmpty, !name.contains("/") else { return }
        let fullPath = (dirPath as NSString).appendingPathComponent(name)
        let resolvedPath = (fullPath as NSString).standardizingPath
        let resolvedDir = (dirPath as NSString).standardizingPath
        guard resolvedPath.hasPrefix(resolvedDir) else { return }
        guard FileManager.default.createFile(atPath: fullPath, contents: nil) else { return }
        self.windowModel.onRefreshTree?()
      }
    }
    windowModel.onNewFolder = { [weak self] (dirPath: String) in
      guard let self, !dirPath.isEmpty else { return }
      NameInputDialog.show(
        title: "New Folder",
        message: "Enter a name for the new folder:",
        placeholder: "untitled-folder",
        defaultValue: ""
      ) { [weak self] name in
        guard let self, !name.isEmpty, !name.contains("/") else { return }
        let fullPath = (dirPath as NSString).appendingPathComponent(name)
        let resolvedPath = (fullPath as NSString).standardizingPath
        let resolvedDir = (dirPath as NSString).standardizingPath
        guard resolvedPath.hasPrefix(resolvedDir) else { return }
        try? FileManager.default.createDirectory(
          atPath: fullPath, withIntermediateDirectories: false)
        self.windowModel.onRefreshTree?()
      }
    }
    // Sidebar action-bar shortcuts: create in the selected tree dir (or root).
    windowModel.onCreateFile = { [weak self] in self?.newFileAction(nil) }
    windowModel.onCreateFolder = { [weak self] in self?.newFolderAction(nil) }
    windowModel.onOpenDiffReview = { [weak self] in self?.openDiffReview() }

    windowModel.gitHost = self
    if let window {
      let model = windowModel
      windowModel.toasts.attach(to: window) { model.palette }
    }
    windowModel.onShowCommandPalette = { [weak self] in
      self?.showCommandPalette()
    }
    windowModel.onShowBranchSwitcher = { [weak self] in
      self?.showBranchSwitcher()
    }
    windowModel.onJoinTab = { [weak self] index, below in
      self?.tabManager.joinTab(at: index, axis: below ? .vertical : .horizontal)
    }
    windowModel.onPaneCommand = { [weak self] command in
      self?.performPaneCommand(command)
    }
    windowModel.onSelectWorkspace = { [weak self] id in
      self?.tabManager.activateWorkspace(id)
    }
    windowModel.onCloseWorkspace = { [weak self] id in
      self?.requestCloseWorkspace(id)
    }
    windowModel.onRenameWorkspace = { [weak self] id in
      self?.presentRenameWorkspace(id)
    }
    windowModel.onSetWorkspaceExpanded = { [weak self] id, expanded in
      self?.tabManager.setWorkspaceExpanded(id, expanded)
    }
    windowModel.onOpenWorkspace = { [weak self] in
      self?.presentOpenWorkspacePanel()
    }
    windowModel.onShowWorkspaceSwitcher = { [weak self] in
      self?.showPalette(prefix: "w:")
    }
    windowModel.onRevealTerminal = { [weak self] id in
      self?.revealTerminal(id: id)
    }
    windowModel.onReviewAgentTurn = { [weak self] id in
      self?.reviewLastAgentTurn(terminalID: id)
    }
    windowModel.onNewTask = { [weak self] in
      self?.presentNewTaskSheet()
    }
    windowModel.onShowAgentHooks = { [weak self] in
      self?.presentAgentHooksSheet()
    }
    windowModel.onComposerSend = { [weak self] text, submit in
      guard let terminal = self?.tabManager.selectedTerminal?.activeTerminal else { return }
      terminal.paste(text, submit: submit)
      if submit {
        self?.windowModel.composerVisible = false
        terminal.focus()
      }
    }
    windowModel.onCloseComposer = { [weak self] in
      self?.windowModel.composerVisible = false
      self?.tabManager.selectedTerminal?.activeTerminal?.focus()
    }
    startPortScanning()
    windowModel.onArchiveTask = { [weak self] id in
      self?.archiveTask(id)
    }

    // AppKit owns the layout (docks, dividers, focus); SwiftUI draws the
    // chrome inside hosting views. See WorkbenchView.
    let centerContent = ContentContainer(content: tabManager.contentView)
    // Hosting views here are sized by AppKit constraints: no SwiftUI-driven
    // min/max size (which clamps the window) and no safe-area padding (the
    // titlebar band would otherwise push the chrome down by its height).
    // The banner and input bar keep their intrinsic height.
    let inputHost = WorkbenchHosting.make(TerminalInputHost(model: windowModel), intrinsicHeight: true)
    inputBarHost = inputHost
    let workbench = WorkbenchView(
      model: windowModel,
      titlebar: WorkbenchHosting.make(ChromeBarView(model: windowModel)),
      banner: WorkbenchHosting.make(WorkbenchBanner(model: windowModel), intrinsicHeight: true),
      statusBar: WorkbenchHosting.make(WorkbenchStatusBar(model: windowModel)),
      leftDockContent: WorkbenchHosting.make(LeftDockView(model: windowModel)),
      rightDockContent: nil,
      bottomDockContent: nil
    )
    self.workbench = workbench
    workbench.translatesAutoresizingMaskIntoConstraints = false
    contentView.addSubview(workbench)
    NSLayoutConstraint.activate([
      workbench.topAnchor.constraint(equalTo: contentView.topAnchor),
      workbench.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
      workbench.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
      workbench.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
    ])

    // The terminal input bar lives inside whichever terminal has focus
    // (see attachInputBar), so the content fills the center column.
    let center = workbench.centerColumn
    centerContent.translatesAutoresizingMaskIntoConstraints = false
    center.addSubview(centerContent)
    NSLayoutConstraint.activate([
      centerContent.topAnchor.constraint(equalTo: center.topAnchor),
      centerContent.leadingAnchor.constraint(equalTo: center.leadingAnchor),
      centerContent.trailingAnchor.constraint(equalTo: center.trailingAnchor),
      centerContent.bottomAnchor.constraint(equalTo: center.bottomAnchor),
    ])
    attachInputBar()

    setupTerminalSearchBar()
  }
}
