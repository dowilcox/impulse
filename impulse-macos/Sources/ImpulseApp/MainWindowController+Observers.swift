import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI
import os.log

extension MainWindowController {

  // MARK: - Notification Observers

  private func ownedEditor(from notification: Notification) -> EditorTab? {
    guard let editor = notification.object as? EditorTab,
      tabManager.ownsEditor(editor)
    else {
      return nil
    }
    return editor
  }

  private static func lineNumber(from userInfo: [AnyHashable: Any]?) -> UInt32? {
    if let line = userInfo?["line"] as? UInt32 {
      return line
    }
    if let line = userInfo?["line"] as? Int, line > 0 {
      return UInt32(line)
    }
    return nil
  }

  func openCommandPaletteSearchResult(path: String, line: UInt32?, column: UInt32? = nil) {
    let column = line == nil ? nil : (column ?? 1)
    tabManager.addEditorTab(
      path: path,
      projectDirectory: fileTreeRootPath,
      goToLine: line,
      goToColumn: column
    )
    if let editor = findEditorTab(forPath: path) {
      trackEditorTab(editor, forPath: path)
      lspDidOpenIfNeeded(path: path)
      if let line, let column {
        editor.goToPosition(line: line, column: column)
      }
    }
  }

  func setupNotificationObservers() {
    let nc = NotificationCenter.default

    notificationObservers.append(
      nc.addObserver(forName: .impulseToggleSidebar, object: nil, queue: .main) { [weak self] _ in
        guard self?.window?.isKeyWindow == true else { return }
        self?.toggleSidebar()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseNewTerminalTab, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.tabManager.addTerminalTab()
        // If a directory was specified (e.g. "Open in Terminal" from file tree),
        // navigate the new terminal to that directory.
        if let dir = notification.userInfo?["directory"] as? String,
          let container = self.tabManager.selectedTerminal,
          let terminal = container.activeTerminal
        {
          terminal.sendCommand("cd \(dir.shellEscaped)")
        }
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseNewFile, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        let cwd = self.getActiveCwd()
        self.tabManager.addUntitledEditorTab(cwd: cwd)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseCloseTab, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        let index = self.tabManager.selectedIndex
        guard index >= 0, index < self.tabManager.tabs.count else { return }
        self.requestCloseFocusedPane()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseReopenTab, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.tabManager.reopenLastClosedTab()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseActiveTabDidChange, object: self.tabManager, queue: .main) {
        [weak self] _ in
        guard let self else { return }
        // Close the window when the last tab is closed (covers tab bar X button,
        // context menu "Close Tab", etc.).
        if self.tabManager.tabs.isEmpty {
          self.window?.close()
          return
        }
        self.updateStatusBar()
        self.attachInputBar()
        // Rebuild the file tree when the active tab's directory context differs
        // from the current root. Terminal tabs use their CWD; editor tabs use
        // the parent directory of the open file.
        if let tab = self.tabManager.selectedTab?.focused {
          let dir: String?
          switch tab {
          case .terminal(let container):
            dir = container.activeTerminal?.currentWorkingDirectory
          case .editor(let editor):
            dir = editor.projectDirectory
          case .imagePreview, .split, .tool:
            dir = nil
          case .diffReview(let repoRoot, _), .history(let repoRoot, _):
            dir = repoRoot
          }
          if self.followsActiveDirectory, let dir, !dir.isEmpty, dir != self.fileTreeRootPath {
            self.switchFileTreeRoot(dir, updateStatusBar: false)
          }
        }
        // Refresh git diff decorations for the newly-active editor tab
        // (they may be stale after terminal git operations).
        if let editor = self.tabManager.selectedEditor {
          self.applyGitDiffDecorations(editor: editor)
        }
        // Hide terminal search bar when switching away from a terminal tab.
        if self.termSearchBarVisible {
          if self.tabManager.selectedTerminal == nil {
            self.hideTerminalSearch()
          } else {
            // Still on a terminal: the freshly-shown tab view was added above
            // the search bar, so re-raise the bar to keep it visible.
            self.termSearchBar.superview?.addSubview(
              self.termSearchBar, positioned: .above, relativeTo: nil)
          }
        }
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseOpenFile, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, self.window?.isKeyWindow == true else { return }
        if let path = notification.userInfo?["path"] as? String {
          var isDirectory: ObjCBool = false
          if FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory),
            isDirectory.boolValue
          {
            self.tabManager.openWorkspace(folder: path)
            return
          }
          let line = Self.lineNumber(from: notification.userInfo)
          let column = (notification.userInfo?["column"] as? Int).map { UInt32(max(1, $0)) }
          self.tabManager.addEditorTab(
            path: path,
            projectDirectory: self.fileTreeRootPath,
            goToLine: line,
            goToColumn: line == nil ? nil : (column ?? 1),
            beside: notification.userInfo?["beside"] as? Bool ?? false,
            preview: notification.userInfo?["preview"] as? Bool ?? false
          )
          // Navigate to specific line if provided (e.g. from search results).
          if let editor = self.findEditorTab(forPath: path) {
            self.trackEditorTab(editor, forPath: path)
            self.lspDidOpenIfNeeded(path: path)
            if let line {
              editor.goToPosition(line: line, column: column ?? 1)
            }
          }
        }
      }
    )
    // Apply git diff decorations once Monaco confirms it has processed
    // the OpenFile command and set up the model. This avoids the race
    // condition where decorations arrive before the model is ready.
    notificationObservers.append(
      nc.addObserver(forName: .editorFileOpened, object: nil, queue: .main) {
        [weak self] notification in
        guard let self else { return }
        if let editor = notification.object as? EditorTab {
          guard self.tabManager.ownsEditor(editor) else { return }
          // Send LSP didOpen now that the tab and Monaco model are ready.
          if let path = editor.filePath {
            self.trackEditorTab(editor, forPath: path)
            if self.lspOpenFiles.contains(self.filePathToUri(path)) {
              // A new model for a file the server already has (reloaded
              // from disk): send it the whole text again, or its copy and
              // every later edit would drift.
              self.lspDidChange(editor: editor)
            } else {
              self.lspDidOpenIfNeeded(path: path)
            }
          }
          self.applyGitDiffDecorations(editor: editor)
          if let path = editor.filePath, self.pendingDiffViewPaths.remove(path) != nil {
            self.setDiffView(editor, enabled: true)
          }
        }
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseReloadEditorFile, object: nil, queue: .main) {
        [weak self] notification in
        // Not gated on key window: the file changed on disk, so every window
        // with it open must reload.
        guard let self else { return }
        if let path = notification.userInfo?["path"] as? String {
          // Reload the open editor from disk. One with unsaved edits keeps
          // them and gets a "changed on disk" notice instead.
          self.findEditorTab(forPath: path)?.reloadFromDisk(force: false)
        }
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .terminalInsertIntoInputBar, object: nil, queue: .main) { [weak self] notification in
        guard let self, let terminal = notification.object as? TerminalTab, self.tabManager.ownsTerminal(terminal),
          let text = notification.userInfo?["text"] as? String
        else { return }
        if let location = self.tabManager.location(ofTerminal: terminal) { self.tabManager.reveal(location) }
        self.insertIntoInputBar(text, typed: notification.userInfo?["typed"] as? Bool ?? false)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .editorChangedOnDisk, object: nil, queue: .main) { [weak self] notification in
        guard let self, let editor = notification.object as? EditorTab, self.tabManager.ownsEditor(editor),
          let path = editor.filePath
        else { return }
        self.toasts.show(
          Toast(
            kind: .warning,
            message: "\((path as NSString).lastPathComponent) changed on disk while you have unsaved edits.",
            actionTitle: "Reload",
            action: { [weak editor] in editor?.reloadFromDisk(force: true) }, lifetime: 20))
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseFindInProject, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.toggleSidebarPanel(.search)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseActiveWorkspaceDidChange, object: tabManager, queue: .main) {
        [weak self] _ in
        self?.activeWorkspaceDidChange()
        self?.updateRestrictedIndicator()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseShowCommandHistory, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, let terminal = notification.object as? TerminalTab,
          self.tabManager.ownsTerminal(terminal)
        else { return }
        self.showPalette(prefix: "h:")
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseSwitchWorkspace, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.showPalette(prefix: "w:")
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseOpenWorkspace, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.presentOpenWorkspacePanel()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .terminalHintChosen, object: nil, queue: .main) { [weak self] notification in
        guard let self, let terminal = notification.object as? TerminalTab,
          self.tabManager.location(ofTerminal: terminal) != nil, let info = notification.userInfo
        else { return }
        self.performHint(info, terminal: terminal)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseBlockCommand, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, self.window?.isKeyWindow == true,
          let command = notification.userInfo?["command"] as? String,
          let terminal = self.tabManager.selectedTerminal?.activeTerminal
        else { return }
        terminal.performBlockCommand(command)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseRunCommand, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, self.window?.isKeyWindow == true,
          let id = notification.userInfo?["id"] as? String
        else { return }
        self.runCommand(id: id)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulsePaneCommand, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, self.window?.isKeyWindow == true,
          let command = notification.userInfo?["command"] as? String
        else { return }
        self.performPaneCommand(command)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseShowChanges, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.toggleSidebarPanel(.changes)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .taskOverlapsChanged, object: nil, queue: .main) { [weak self] notification in
        guard let self else { return }
        self.tabManager.syncToWindowModel()
        // A new pair of workspaces changing the same files: say so once,
        // in the key window, when it's one of this window's.
        guard SettingsStore.shared.settings.taskOverlapNotify, self.window?.isKeyWindow == true,
          let started = notification.userInfo?["newPairs"] as? [TaskOverlap.Pair]
        else { return }
        let roots = Set(self.tabManager.workspaces.map { TaskRegistry.canonical($0.root) })
        for pair in started where roots.contains(pair.a.path) || roots.contains(pair.b.path) {
          self.toasts.show(
            Toast(
              kind: .warning, message: OverlapMonitor.sentence(pair),
              detail: pair.files.prefix(3).joined(separator: ", ") + (pair.files.count > 3 ? ", …" : ""),
              lifetime: 12))
        }
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .reviewedTasksMerged, object: nil, queue: .main) { [weak self] notification in
        // A task Finish pushed for review was merged on the server: the
        // window that has it open offers to finish the job.
        guard let self, let paths = notification.userInfo?["paths"] as? [String] else { return }
        for path in paths {
          guard let workspace = self.tabManager.workspaces.first(where: { TaskRegistry.canonical($0.root) == path }),
            OverlapMonitor.shared.claimCleanUpOffer(path)
          else { continue }
          self.toasts.show(
            Toast(
              kind: .success, message: "\(workspace.name) was merged.",
              detail: "Finish can update the main checkout and archive the task.", actionTitle: "Clean Up…",
              action: { [weak self] in self?.openFinishTask(from: workspace.id) }, lifetime: 30,
              tag: "clean-up:\(path)"))
        }
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .gitHeadMoved, object: nil, queue: .main) { [weak self] notification in
        guard let info = notification.userInfo, let root = info["root"] as? String, let from = info["from"] as? String,
          let to = info["to"] as? String
        else { return }
        self?.offerDependencySteps(root: root, from: from, to: to)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseSwitchBranch, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.showBranchSwitcher()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseManageBranches, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.presentBranchManager()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .terminalCommandBlockChanged, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let tab = notification.object as? TerminalTab,
          self.tabManager.selectedTerminal?.activeTerminal === tab
        else { return }
        self.updateStatusBar()
        self.tabManager.syncToWindowModel()
      })
    notificationObservers.append(
      nc.addObserver(forName: .terminalInteractionModeChanged, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let tab = notification.object as? TerminalTab,
          self.tabManager.selectedTerminal?.activeTerminal === tab,
          let interactive = notification.userInfo?["interactive"] as? Bool
        else { return }
        self.windowModel.terminalDirectInteraction = interactive
        // Re-sync tab subtitles so the working folder appears alongside the
        // branch while a program/TUI owns the grid.
        self.tabManager.syncToWindowModel()
        // Leaving a TUI: the input bar reappears and should reclaim focus
        // (unless it's disabled, in which case the grid keeps the keyboard).
        if !interactive {
          if tab.wantsGridFocus {
            tab.focus()
          } else {
            self.windowModel.inputBarFocusToken += 1
          }
        }
      })
    notificationObservers.append(
      nc.addObserver(forName: .terminalRequestInputFocus, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let tab = notification.object as? TerminalTab,
          self.tabManager.selectedTerminal?.activeTerminal === tab
        else { return }
        self.windowModel.inputBarFocusToken += 1
      })
    notificationObservers.append(
      nc.addObserver(forName: .terminalPasswordInputChanged, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let tab = notification.object as? TerminalTab,
          self.tabManager.selectedTerminal?.activeTerminal === tab,
          let active = notification.userInfo?["active"] as? Bool
        else { return }
        self.windowModel.passwordInputActive = active
        // Focus is re-grabbed by the input bar itself: it must wait out the
        // plain↔secure field swap, so a token bump here would fire too early.
      })
    notificationObservers.append(
      nc.addObserver(forName: .terminalCwdChanged, object: nil, queue: .main) {
        [weak self] notification in
        guard let self else { return }
        guard let terminal = notification.object as? TerminalTab,
          self.tabManager.ownsTerminal(terminal)
        else { return }
        if let dir = notification.userInfo?["directory"] as? String {
          self.settings.lastDirectory = dir
          let isSelected = self.tabManager.selectedTerminal?.activeTerminal === terminal
          if dir == self.fileTreeRootPath || !self.followsActiveDirectory || !isSelected {
            // Same directory — just refresh git status (a command
            // may have changed git state without changing CWD).
            self.fileTreeData.refreshGitStatus()
            self.windowModel.repository?.refresh()
          } else {
            self.switchFileTreeRoot(dir)
          }
          // Push the new CWD/branch into the status bar right away
          // if this terminal is the selected one; otherwise the
          // status bar would lag until the next explicit refresh.
          if let selected = self.tabManager.selectedTerminal?.activeTerminal,
            selected === terminal
          {
            self.updateStatusBar()
          }
        }
      }
    )

    // Command palette
    notificationObservers.append(
      nc.addObserver(forName: .impulseShowCommandPalette, object: nil, queue: .main) {
        [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.showPalette(prefix: ">")
      }
    )

    notificationObservers.append(
      nc.addObserver(forName: .impulseRunInTerminal, object: nil, queue: .main) { [weak self] notification in
        guard let self, let editor = notification.object as? EditorTab, self.tabManager.ownsEditor(editor),
          let command = notification.userInfo?["command"] as? String
        else { return }
        self.runFromPreview(
          command, directory: notification.userInfo?["directory"] as? String ?? NSHomeDirectory(), editor: editor)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseGoToSymbol, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.showPalette(prefix: "@")
      }
    )
    // Go to File… (⌘P): the palette's file mode.
    notificationObservers.append(
      nc.addObserver(forName: .impulseQuickOpen, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.showPalette(prefix: "")
      }
    )

    // Update available — surface the updater on the visible SwiftUI status bar.
    notificationObservers.append(
      nc.addObserver(forName: .impulseUpdateAvailable, object: nil, queue: .main) {
        [weak self] notification in
        guard let self else { return }
        self.windowModel.updateAvailableVersion = notification.userInfo?["version"] as? String
        self.windowModel.updateCurrentVersion = notification.userInfo?["currentVersion"] as? String
        if let urlString = notification.userInfo?["url"] as? String {
          self.windowModel.updateURL = URL(string: urlString)
        } else {
          self.windowModel.updateURL = nil
        }
      }
    )

    // Install LSP — install managed web LSP servers
    notificationObservers.append(
      nc.addObserver(forName: .impulseInstallLsp, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        DispatchQueue.global(qos: .userInitiated).async {
          let result = ImpulseCore.lspInstall()
          DispatchQueue.main.async { [weak self] in
            switch result {
            case .success(let path):
              self?.toasts.show(
                Toast(kind: .success, message: "Language servers installed in \(TabManager.abbreviateHomePath(path))"))
            case .failure(let error):
              self?.toasts.show(Toast(kind: .warning, message: "Couldn't install language servers: \(error.message)", lifetime: 12))
            }
          }
        }
      }
    )

    // Save file — fired from menu Cmd+S or from EditorTab's SaveRequested event
    notificationObservers.append(
      nc.addObserver(forName: .impulseSaveFile, object: nil, queue: .main) {
        [weak self] notification in
        guard let self else { return }

        // If the notification came from an EditorTab (Monaco Cmd+S path),
        // save that specific editor directly — no key-window check needed
        // because the editor itself initiated the save.
        if notification.object is EditorTab {
          guard let sourceEditor = self.ownedEditor(from: notification) else { return }
          self.saveEditorTab(sourceEditor)
          return
        }

        // Menu path: save the currently selected editor tab.
        guard self.window?.isKeyWindow == true else { return }
        if let editor = self.tabManager.selectedEditor {
          self.saveEditorTab(editor)
        }
      }
    )

    // Find — editor: Monaco find widget; terminal: search bar toggle
    notificationObservers.append(
      nc.addObserver(forName: .impulseFind, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        guard let tab = self.tabManager.selectedTab?.focused else { return }
        switch tab {
        case .editor(let editor):
          editor.webView?.evaluateJavaScript(
            "editor.getAction('actions.find').run()",
            completionHandler: nil
          )
        case .terminal:
          self.toggleTerminalSearch()
        case .imagePreview, .diffReview, .history, .tool, .split:
          break
        }
      }
    )

    // Editor cursor position tracking for the status bar
    notificationObservers.append(
      nc.addObserver(forName: .editorCursorMoved, object: nil, queue: .main) {
        [weak self] notification in
        guard let self else { return }
        guard let line = notification.userInfo?["line"] as? UInt32,
          let col = notification.userInfo?["column"] as? UInt32
        else { return }
        // Only update if the notification came from the active editor tab
        guard let editor = self.ownedEditor(from: notification),
          editor === self.tabManager.selectedEditor
        else { return }
        let filePath = editor.filePath ?? ""
        let cwd =
          editor.projectDirectory
          ?? (filePath as NSString).deletingLastPathComponent
        // Sync to SwiftUI
        self.windowModel.cursorLine = Int(line)
        self.windowModel.cursorCol = Int(col)
        self.windowModel.currentCwd = cwd
        self.bindRepository(forDirectory: cwd)
        self.windowModel.currentLanguage = editor.language
      }
    )

    // Terminal title changed — update tab titles
    notificationObservers.append(
      nc.addObserver(forName: .terminalTitleChanged, object: nil, queue: .main) {
        [weak self] notification in
        guard let self else { return }
        guard let terminal = notification.object as? TerminalTab,
          self.tabManager.ownsTerminal(terminal)
        else { return }
        self.tabManager.refreshSegmentLabels()
      }
    )

    // Terminal attention changed — update tab indicators
    notificationObservers.append(
      nc.addObserver(forName: .terminalAttentionChanged, object: nil, queue: .main) {
        [weak self] notification in
        guard let self else { return }
        guard let terminal = notification.object as? TerminalTab,
          self.tabManager.ownsTerminal(terminal)
        else { return }
        if !terminal.needsAttention { DesktopNotifier.shared.clear(terminalID: terminal.id) }
        self.tabManager.refreshSegmentLabels()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .terminalWantsNotification, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, !NSApp.isActive,
          let terminal = notification.object as? TerminalTab,
          let location = self.tabManager.location(ofTerminal: terminal),
          let workspaceID = self.tabManager.workspaceID(ofTabAt: location.tabIndex)
        else { return }
        let workspace = self.tabManager.workspace(workspaceID)
        DesktopNotifier.shared.post(
          title: notification.userInfo?["title"] as? String ?? terminal.tabTitle,
          subtitle: [workspace?.name, terminal.tabTitle].compactMap { $0 }.joined(separator: " · "),
          body: notification.userInfo?["body"] as? String ?? "",
          terminalID: terminal.id, thread: workspaceID.uuidString)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: DesktopNotifier.revealTerminal, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, let id = (notification.userInfo?["terminal"] as? String).flatMap(UUID.init)
        else { return }
        self.revealTerminal(id: id)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .terminalAgentChanged, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, let terminal = notification.object as? TerminalTab,
          self.tabManager.ownsTerminal(terminal)
        else { return }
        self.tabManager.refreshSegmentLabels()
        self.announceAgentState(of: terminal)
        // Opt-in: the focused agent asks for input → open the composer.
        if self.settings.agentComposerAutoShow, terminal.agentState == .needsInput,
          terminal === self.tabManager.selectedTerminal?.activeTerminal, terminal.isDirectInteraction,
          !self.windowModel.composerVisible, self.window?.isKeyWindow == true
        {
          self.toggleAgentComposer()
        }
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .agentCheckpointsChanged, object: nil, queue: .main) { [weak self] _ in
        self?.tabManager.refreshSegmentLabels()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseSendToAgent, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, let text = notification.userInfo?["text"] as? String else { return }
        if let terminal = notification.object as? TerminalTab {
          guard self.tabManager.ownsTerminal(terminal) else { return }
          self.sendToBestAgent(text, excluding: terminal)
        } else if self.window?.isKeyWindow == true {
          self.sendToBestAgent(text)
        }
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseAgentComposer, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.toggleAgentComposer()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseNextAgent, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.revealNextWaitingAgent()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .editorGitAction, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, let editor = self.ownedEditor(from: notification),
          let action = notification.userInfo?["action"] as? String,
          let line = notification.userInfo?["line"] as? Int
        else { return }
        self.handleEditorGitAction(editor: editor, action: action, line: line)
      }
    )
    // Vim mode's :q, :wq and :x.
    notificationObservers.append(
      nc.addObserver(forName: .editorCloseRequested, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, let editor = self.ownedEditor(from: notification) else { return }
        let save = notification.userInfo?["save"] as? String ?? "never"
        guard save == "always" || (save == "modified" && editor.isModified) else {
          self.requestClose(editor: editor)
          return
        }
        self.saveEditorTab(editor) { [weak self, weak editor] saved in
          guard saved, let self, let editor else { return }
          self.requestClose(editor: editor)
        }
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .editorCodeActionChosen, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, self.ownedEditor(from: notification) != nil,
          let token = notification.userInfo?["token"] as? String
        else { return }
        self.runLspCodeAction(token: token)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .editorLspRequested, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, let editor = self.ownedEditor(from: notification),
          let requestId = notification.userInfo?["requestId"] as? UInt64,
          let method = notification.userInfo?["method"] as? String,
          let params = notification.userInfo?["params"] as? String
        else { return }
        self.handleLspPassthrough(editor: editor, requestId: requestId, method: method, params: params)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .editorDirtyStateChanged, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, let editor = notification.object as? EditorTab,
          self.tabManager.ownsEditor(editor)
        else { return }
        // An edit keeps a preview tab.
        if editor.isModified { self.tabManager.keepPreview(showing: editor) }
        self.tabManager.refreshSegmentLabels()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .terminalProgressChanged, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let terminal = notification.object as? TerminalTab,
          self.tabManager.ownsTerminal(terminal)
        else { return }
        self.tabManager.refreshSegmentLabels()
      }
    )

    // Terminal process terminated — close the tab.
    notificationObservers.append(
      nc.addObserver(forName: .terminalProcessTerminated, object: nil, queue: .main) {
        [weak self] notification in
        guard let self else { return }
        guard let terminalTab = notification.object as? TerminalTab,
          self.tabManager.ownsTerminal(terminalTab)
        else { return }
        if let location = self.tabManager.locate(where: {
          if case .terminal(let container) = $0 {
            return container.terminals.contains { $0 === terminalTab }
          }
          return false
        }) {
          self.tabManager.closePane(location.paneID, inTabAt: location.tabIndex)
        }
      }
    )

    // Editor content changed — refresh tab labels and notify LSP
    notificationObservers.append(
      nc.addObserver(forName: .editorContentChanged, object: nil, queue: .main) {
        [weak self] notification in
        guard let self else { return }
        guard let editor = self.ownedEditor(from: notification) else { return }
        self.tabManager.refreshSegmentLabels()
        let changes = notification.userInfo?["changes"] as? [MonacoContentChange] ?? []
        self.lspDidChange(editor: editor, changes: changes)
      }
    )

    // Editor focus changed — auto-save on focus loss if enabled
    notificationObservers.append(
      nc.addObserver(forName: .editorFocusChanged, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, self.settings.autoSave else { return }
        guard let editor = self.ownedEditor(from: notification) else { return }
        guard let focused = notification.userInfo?["focused"] as? Bool, !focused else { return }
        guard editor.isModified else { return }
        self.saveEditorTab(editor)
      }
    )

    // Custom keybinding command execution
    notificationObservers.append(
      nc.addObserver(forName: Notification.Name("impulseCustomCommand"), object: nil, queue: .main)
      { [weak self] notification in
        guard let self, self.window?.isKeyWindow == true else { return }
        guard let command = notification.userInfo?["command"] as? String,
          !command.isEmpty
        else { return }
        let args = notification.userInfo?["args"] as? [String] ?? []
        self.executeCustomCommand(command: command, args: args)
      }
    )

    // LSP: completion requested
    notificationObservers.append(
      nc.addObserver(forName: .editorCompletionRequested, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let editor = self.ownedEditor(from: notification),
          let requestId = notification.userInfo?["requestId"] as? UInt64,
          let line = notification.userInfo?["line"] as? UInt32,
          let character = notification.userInfo?["character"] as? UInt32
        else { return }
        self.handleCompletionRequest(
          editor: editor, requestId: requestId, line: line, character: character)
      }
    )

    // LSP: hover requested
    notificationObservers.append(
      nc.addObserver(forName: .editorHoverRequested, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let editor = self.ownedEditor(from: notification),
          let requestId = notification.userInfo?["requestId"] as? UInt64,
          let line = notification.userInfo?["line"] as? UInt32,
          let character = notification.userInfo?["character"] as? UInt32
        else { return }
        self.handleHoverRequest(
          editor: editor, requestId: requestId, line: line, character: character)
      }
    )

    // Go to line
    notificationObservers.append(
      nc.addObserver(forName: .impulseGoToLine, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.showGoToLineDialog()
      }
    )

    // Toggle Preview (Markdown / SVG)
    notificationObservers.append(
      nc.addObserver(forName: .impulseToggleMarkdownPreview, object: nil, queue: .main) {
        [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.togglePreview()
      }
    )

    // Review Changes — open the git diff review tab for the current workspace.
    notificationObservers.append(
      nc.addObserver(forName: .impulseReviewChanges, object: nil, queue: .main) {
        [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.openDiffReview()
      }
    )

    // Font size
    notificationObservers.append(
      nc.addObserver(forName: .impulseFontIncrease, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.changeFontSize(delta: 1)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseFontDecrease, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.changeFontSize(delta: -1)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseFontReset, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.resetFontSize()
      }
    )

    // Tab cycling
    notificationObservers.append(
      nc.addObserver(forName: .impulseNextTab, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.cycleTab(by: 1)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulsePrevTab, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.cycleTab(by: -1)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseSelectTab, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, self.window?.isKeyWindow == true else { return }
        guard let index = notification.userInfo?["index"] as? Int else { return }
        // ⌘1…⌘9 count tabs in the active workspace.
        let visible = self.tabManager.visibleTabIndices
        if visible.indices.contains(index) {
          self.tabManager.selectTab(index: visible[index])
        }
      }
    )

    // Settings changed (from SettingsWindow)
    notificationObservers.append(
      nc.addObserver(forName: .impulseSettingsDidChange, object: nil, queue: .main) {
        [weak self] notification in
        guard let self else { return }
        self.applyAllSettings()
        // settings.json may have been fixed (or broken) by hand.
        self.windowModel.settingsLoadWarning = Settings.loadWarning
        // Rebuild custom keybinding monitor so new/changed bindings take effect.
        self.setupCustomKeybindingMonitor()
      }
    )

    // LSP: go-to-definition requested
    notificationObservers.append(
      nc.addObserver(forName: .editorDefinitionRequested, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let editor = self.ownedEditor(from: notification),
          let requestId = notification.userInfo?["requestId"] as? UInt64,
          let line = notification.userInfo?["line"] as? UInt32,
          let character = notification.userInfo?["character"] as? UInt32
        else { return }
        self.handleDefinitionRequest(
          editor: editor, requestId: requestId, line: line, character: character)
      }
    )

    // Monaco: cross-file navigation (fired by registerEditorOpener on actual click)
    notificationObservers.append(
      nc.addObserver(forName: .editorOpenFileRequested, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          self.ownedEditor(from: notification) != nil,
          let uri = notification.userInfo?["uri"] as? String,
          let line = notification.userInfo?["line"] as? UInt32,
          let character = notification.userInfo?["character"] as? UInt32
        else { return }
        self.handleOpenFileRequested(uri: uri, line: line, character: character)
      }
    )

    // LSP: formatting requested
    notificationObservers.append(
      nc.addObserver(forName: .editorFormattingRequested, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let editor = self.ownedEditor(from: notification),
          let requestId = notification.userInfo?["requestId"] as? UInt64,
          let tabSize = notification.userInfo?["tabSize"] as? UInt32,
          let insertSpaces = notification.userInfo?["insertSpaces"] as? Bool
        else { return }
        self.handleFormattingRequest(
          editor: editor, requestId: requestId, tabSize: tabSize, insertSpaces: insertSpaces)
      }
    )

    // LSP: signature help requested
    notificationObservers.append(
      nc.addObserver(forName: .editorSignatureHelpRequested, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let editor = self.ownedEditor(from: notification),
          let requestId = notification.userInfo?["requestId"] as? UInt64,
          let line = notification.userInfo?["line"] as? UInt32,
          let character = notification.userInfo?["character"] as? UInt32
        else { return }
        self.handleSignatureHelpRequest(
          editor: editor, requestId: requestId, line: line, character: character)
      }
    )

    // LSP: references requested
    notificationObservers.append(
      nc.addObserver(forName: .editorReferencesRequested, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let editor = self.ownedEditor(from: notification),
          let requestId = notification.userInfo?["requestId"] as? UInt64,
          let line = notification.userInfo?["line"] as? UInt32,
          let character = notification.userInfo?["character"] as? UInt32
        else { return }
        self.handleReferencesRequest(
          editor: editor, requestId: requestId, line: line, character: character)
      }
    )

    // LSP: code action requested
    notificationObservers.append(
      nc.addObserver(forName: .editorCodeActionRequested, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let editor = self.ownedEditor(from: notification),
          let requestId = notification.userInfo?["requestId"] as? UInt64,
          let startLine = notification.userInfo?["startLine"] as? UInt32,
          let startColumn = notification.userInfo?["startColumn"] as? UInt32,
          let endLine = notification.userInfo?["endLine"] as? UInt32,
          let endColumn = notification.userInfo?["endColumn"] as? UInt32
        else { return }
        let diagnostics = notification.userInfo?["diagnostics"] as? [[String: Any]] ?? []
        self.handleCodeActionRequest(
          editor: editor, requestId: requestId, startLine: startLine, startColumn: startColumn,
          endLine: endLine, endColumn: endColumn, diagnostics: diagnostics)
      }
    )

    // LSP: rename requested
    notificationObservers.append(
      nc.addObserver(forName: .editorRenameRequested, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let editor = self.ownedEditor(from: notification),
          let requestId = notification.userInfo?["requestId"] as? UInt64,
          let line = notification.userInfo?["line"] as? UInt32,
          let character = notification.userInfo?["character"] as? UInt32,
          let newName = notification.userInfo?["newName"] as? String
        else { return }
        self.handleRenameRequest(
          editor: editor, requestId: requestId, line: line, character: character, newName: newName)
      }
    )

    // LSP: prepare rename requested
    notificationObservers.append(
      nc.addObserver(forName: .editorPrepareRenameRequested, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let editor = self.ownedEditor(from: notification),
          let requestId = notification.userInfo?["requestId"] as? UInt64,
          let line = notification.userInfo?["line"] as? UInt32,
          let character = notification.userInfo?["character"] as? UInt32
        else { return }
        self.handlePrepareRenameRequest(
          editor: editor, requestId: requestId, line: line, character: character)
      }
    )
  }
}
