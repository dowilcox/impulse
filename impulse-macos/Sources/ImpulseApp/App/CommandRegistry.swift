import AppKit
import ImpulseKit

/// A command that can be run from the palette (and, through its keybinding,
/// from the menu bar).
struct AppCommand: Identifiable {
  let id: String
  let title: String
  let category: String
  var keywords: [String] = []
  var icon: LucideIcon? = nil
  /// Keybinding id whose shortcut is shown next to the command.
  var keybindingId: String? = nil
  /// Literal shortcut text (custom commands), used when `keybindingId` is nil.
  var shortcut: String? = nil
  let action: () -> Void

  /// Text the palette matches against: title first, then category/keywords.
  var searchText: String { title }
}

/// The catalog of commands for a window. Legacy built-ins keep their ids,
/// titles and keywords from `ImpulseKit.CommandPalette` (covered by golden
/// fixtures); workbench commands are added here.
enum CommandRegistry {
  static func commands(
    for controller: MainWindowController, customKeybindings: [CustomKeybinding]
  ) -> [AppCommand] {
    var result: [AppCommand] = []

    for item in ImpulseKit.CommandPalette.builtinItems() {
      result.append(
        AppCommand(
          id: item.id, title: item.title, category: item.category, keywords: item.keywords,
          icon: icon(forBuiltin: item.id), keybindingId: keybindingId(forBuiltin: item.id),
          action: builtinAction(item.id, controller: controller)))
    }

    result += [
      AppCommand(
        id: "show_changes", title: "Show Changes", category: "Git",
        keywords: ["git", "stage", "commit", "status"], icon: .gitBranch,
        keybindingId: "show_changes"
      ) { [weak controller] in controller?.toggleSidebarPanel(.changes) },
      AppCommand(
        id: "show_files", title: "Show Files", category: "Navigation",
        keywords: ["explorer", "file tree", "sidebar", "folders"], icon: .folderTree,
        keybindingId: "show_files"
      ) { [weak controller] in controller?.toggleSidebarPanel(.files) },
      AppCommand(
        id: "switch_branch", title: "Switch Branch…", category: "Git",
        keywords: ["checkout", "git", "create branch"], icon: .gitBranch,
        keybindingId: "switch_branch"
      ) { [weak controller] in controller?.showBranchSwitcher() },
      AppCommand(
        id: "agent_hooks", title: "Install Agent Hooks…", category: "Agents",
        keywords: ["claude", "codex", "hooks", "status", "setup"], icon: .plug
      ) { [weak controller] in controller?.presentAgentHooksSheet() },
      AppCommand(
        id: "editor_integration", title: "Use Impulse as $EDITOR in Terminals", category: "Terminal",
        keywords: ["git commit", "visual", "editor", "wait"], icon: .pencil
      ) { [weak controller] in controller?.toggleEditorIntegration() },
      AppCommand(
        id: "manage_branches", title: "Manage Branches…", category: "Git",
        keywords: ["delete", "rename", "publish", "merged", "stale"], icon: .gitBranch,
        keybindingId: "manage_branches"
      ) { [weak controller] in controller?.presentBranchManager() },
      AppCommand(
        id: "git_fetch", title: "Fetch", category: "Git",
        keywords: ["git", "remote", "update", "sync"], icon: .refreshCw, keybindingId: "git_fetch"
      ) { [weak controller] in controller?.repositoryActions()?.fetch() },
      AppCommand(
        id: "git_fetch_all", title: "Fetch All Remotes", category: "Git",
        keywords: ["git", "remote", "update", "sync", "upstream"], icon: .refreshCw,
        keybindingId: "git_fetch_all"
      ) { [weak controller] in controller?.repositoryActions()?.fetch(allRemotes: true) },
      AppCommand(
        id: "git_pull", title: "Pull", category: "Git",
        keywords: ["git", "update", "sync", "download", "merge"], icon: .arrowDown, keybindingId: "git_pull"
      ) { [weak controller] in controller?.repositoryActions()?.pull() },
      AppCommand(
        id: "git_pull_rebase", title: "Pull (Rebase)", category: "Git",
        keywords: ["git", "update", "sync", "rebase"], icon: .arrowDown, keybindingId: "git_pull_rebase"
      ) { [weak controller] in controller?.repositoryActions()?.pull(mode: .rebase) },
      AppCommand(
        id: "git_push", title: "Push", category: "Git",
        keywords: ["git", "upload", "sync", "publish"], icon: .arrowUp, keybindingId: "git_push"
      ) { [weak controller] in controller?.repositoryActions()?.push() },
      AppCommand(
        id: "git_force_push", title: "Force Push (With Lease)…", category: "Git",
        keywords: ["git", "overwrite", "rebase", "force-with-lease"], icon: .arrowUp,
        keybindingId: "git_force_push"
      ) { [weak controller] in controller?.repositoryActions()?.forcePush() },
      AppCommand(
        id: "git_create_tag", title: "Create Tag…", category: "Git",
        keywords: ["git", "release", "version", "annotated"], icon: .tag, keybindingId: "git_create_tag"
      ) { [weak controller] in controller?.createTagAtHead() },
      AppCommand(
        id: "git_push_tags", title: "Push All Tags", category: "Git",
        keywords: ["git", "release", "version", "upload"], icon: .tag, keybindingId: "git_push_tags"
      ) { [weak controller] in controller?.repositoryActions()?.pushAllTags() },
      AppCommand(
        id: "git_stash", title: "Stash All Changes", category: "Git",
        keywords: ["git", "save", "shelve", "wip"], icon: .archive, keybindingId: "git_stash"
      ) { [weak controller] in controller?.repositoryActions()?.stashAll() },
      AppCommand(
        id: "git_pop_stash", title: "Pop Latest Stash", category: "Git",
        keywords: ["git", "restore", "unshelve", "apply"], icon: .archive, keybindingId: "git_pop_stash"
      ) { [weak controller] in controller?.popLatestStash() },
      AppCommand(
        id: "git_undo_commit", title: "Undo Last Commit", category: "Git",
        keywords: ["git", "uncommit", "reset", "soft"], icon: .undo2, keybindingId: "git_undo_commit"
      ) { [weak controller] in controller?.repositoryActions()?.undoLastCommit() },
      AppCommand(
        id: "git_open_remote", title: "Open Repository in Browser", category: "Git",
        keywords: ["git", "web", "remote", "url"], icon: .externalLink, keybindingId: "git_open_remote"
      ) { [weak controller] in controller?.repositoryActions()?.openRepositoryInBrowser() },
      AppCommand(
        id: "git_copy_remote_url", title: "Copy Remote URL", category: "Git",
        keywords: ["git", "remote", "origin", "clone", "url"], icon: .copy, keybindingId: "git_copy_remote_url"
      ) { [weak controller] in controller?.repositoryActions()?.copyRemoteURL() },
      AppCommand(
        id: "git_history", title: "Show Git History", category: "Git",
        keywords: ["log", "commits", "graph", "blame"], icon: .history, keybindingId: "git_history"
      ) { [weak controller] in controller?.showHistory() },
      AppCommand(
        id: "file_history", title: "Show History of This File", category: "Git",
        keywords: ["log", "commits", "blame"], icon: .history, keybindingId: "file_history"
      ) { [weak controller] in
        guard let controller else { return }
        controller.showHistory(path: controller.tabManager.selectedEditor?.filePath)
      },
      AppCommand(
        id: "new_task", title: "New Task…", category: "Workspaces",
        keywords: ["worktree", "branch", "agent", "parallel"], icon: .gitBranchPlus, keybindingId: "new_task"
      ) { [weak controller] in controller?.presentNewTaskSheet() },
      AppCommand(
        id: "project_setup", title: "Project Setup…", category: "Workspaces",
        keywords: ["task", "worktree", "ports", "env", "clone", "setup script", "docker", "project.toml"],
        icon: .settings, keybindingId: "project_setup"
      ) { [weak controller] in controller?.openProjectSetup() },
      AppCommand(
        id: "new_task_from_branch", title: "New Task from Branch…", category: "Workspaces",
        keywords: ["worktree", "branch", "checkout", "review", "existing", "remote"], icon: .gitBranch,
        keybindingId: "new_task_from_branch"
      ) { [weak controller] in controller?.showPalette(prefix: "task:") },
      AppCommand(
        id: "finish_task", title: "Finish Task…", category: "Workspaces",
        keywords: ["worktree", "merge", "land", "push", "review", "done", "check"], icon: .gitMerge,
        keybindingId: "finish_task"
      ) { [weak controller] in controller?.openFinishTask() },
      AppCommand(
        id: "archive_task", title: "Archive Task…", category: "Workspaces",
        keywords: ["worktree", "remove", "done"], icon: .archive
      ) { [weak controller] in
        guard let controller else { return }
        controller.archiveTask(controller.tabManager.activeWorkspaceID)
      },
      AppCommand(
        id: "review_agent_turn", title: "Review Last Agent Turn", category: "Agents",
        keywords: ["claude", "codex", "diff", "checkpoint", "changes"], icon: .fileDiff,
        keybindingId: "review_agent_turn"
      ) { [weak controller] in controller?.reviewLastAgentTurn() },
      AppCommand(
        id: "send_selection_to_agent", title: "Send Selection to Agent", category: "Agents",
        keywords: ["claude", "codex", "code", "ask"], icon: .messageSquarePlus
      ) { [weak controller] in controller?.sendEditorSelectionToAgent() },
      AppCommand(
        id: "agent_composer", title: "Compose Message to Agent", category: "Agents",
        keywords: ["prompt", "claude", "codex", "write"], icon: .messageSquare,
        keybindingId: "agent_composer"
      ) { [weak controller] in controller?.toggleAgentComposer() },
      AppCommand(
        id: "next_agent", title: "Next Agent Needing You", category: "Agents",
        keywords: ["claude", "codex", "inbox", "waiting"], icon: .bot, keybindingId: "next_agent"
      ) { [weak controller] in controller?.revealNextWaitingAgent() },
      AppCommand(
        id: "command_history", title: "Command History…", category: "Terminal",
        keywords: ["history", "recent", "ctrl-r"], icon: .history
      ) { [weak controller] in controller?.showPalette(prefix: "h:") },
      AppCommand(
        id: "preview_beside", title: "Open Preview to the Side", category: "Editor",
        keywords: ["markdown", "svg", "split", "live"], icon: .columns2
      ) { [weak controller] in controller?.togglePreviewBeside() },
      AppCommand(
        id: "diff_view", title: "Toggle Diff View", category: "Editor",
        keywords: ["changes", "git", "compare", "staged", "side by side"], icon: .fileDiff,
        keybindingId: "diff_view"
      ) { [weak controller] in controller?.toggleDiffView() },
      AppCommand(
        id: "go_to_symbol", title: "Go to Symbol in File…", category: "Editor",
        keywords: ["outline", "function", "class", "@"], icon: .code, keybindingId: "go_to_symbol"
      ) { [weak controller] in controller?.showPalette(prefix: "@") },
      AppCommand(
        id: "go_to_project_symbol", title: "Go to Symbol in Project…", category: "Editor",
        keywords: ["workspace", "function", "class", "#"], icon: .code, keybindingId: "go_to_project_symbol"
      ) { [weak controller] in controller?.showPalette(prefix: "#") },
      AppCommand(
        id: "project_actions", title: "Run Project Action…", category: "Workspaces",
        keywords: ["task", "script", "run", "project.toml", "a:"], icon: .play,
        keybindingId: "project_actions"
      ) { [weak controller] in controller?.showPalette(prefix: "a:") },
      AppCommand(
        id: "trust_folder", title: "Trust This Folder…", category: "Workspaces",
        keywords: ["workspace trust", "restricted", "security", "language servers"], icon: .shieldCheck
      ) { [weak controller] in controller?.trustActiveFolder() },
      AppCommand(
        id: "restrict_folder", title: "Restrict This Folder", category: "Workspaces",
        keywords: ["workspace trust", "untrust", "security", "language servers"], icon: .shieldAlert
      ) { [weak controller] in controller?.restrictActiveFolder() },
      AppCommand(
        id: "forget_trusted_folders", title: "Forget Trusted Folders", category: "Workspaces",
        keywords: ["workspace trust", "restricted", "security", "reset"], icon: .shieldAlert
      ) { [weak controller] in controller?.forgetTrustedFolders() },
      AppCommand(
        id: "edit_project_config", title: "Edit Project Actions", category: "Workspaces",
        keywords: ["project.toml", "scripts", "worktree"], icon: .pencil
      ) { [weak controller] in controller?.editProjectConfig() },
      AppCommand(
        id: "show_problems", title: "Show Problems", category: "Editor",
        keywords: ["diagnostics", "errors", "warnings", "lint"], icon: .triangleAlert,
        keybindingId: "show_problems"
      ) { [weak controller] in controller?.showProblems() },
      AppCommand(
        id: "quick_terminal", title: "Toggle Quick Terminal", category: "Terminal",
        keywords: ["dropdown", "hotkey", "quake", "global"], icon: .squareTerminal
      ) { QuickTerminal.shared.toggle() },
      AppCommand(
        id: "open_keybindings", title: "Keyboard Shortcuts…", category: "Impulse",
        keywords: ["keybindings", "shortcuts", "keys", "hotkeys"], icon: .keyboard,
        keybindingId: "open_keybindings"
      ) { [weak controller] in controller?.openKeybindings() },
      AppCommand(
        id: "open_settings_json", title: "Open settings.json", category: "Impulse",
        keywords: ["preferences", "config", "json"], icon: .fileCode
      ) { [weak controller] in controller?.openSettingsFile() },
      AppCommand(
        id: "find_setting", title: "Find a Setting…", category: "Impulse",
        keywords: ["preferences", "search"], icon: .settings
      ) { [weak controller] in controller?.showPalette(prefix: "set:") },
      AppCommand(
        id: "terminal_hints", title: "Show Hints", category: "Terminal",
        keywords: ["link", "url", "path", "sha", "port", "open", "copy"], icon: .keyboard,
        keybindingId: "terminal_hints"
      ) { [weak controller] in
        controller?.tabManager.selectedTerminal?.activeTerminal?.performBlockCommand("terminal_hints")
      },
      AppCommand(
        id: "import_shell_history", title: "Import Shell History", category: "Terminal",
        keywords: ["zsh", "bash", "fish", "history"], icon: .history
      ) { [weak controller] in controller?.importShellHistory() },
      AppCommand(
        id: "switch_workspace", title: "Switch Workspace…", category: "Workspaces",
        keywords: ["project", "folder", "recent"], icon: .folderGit2,
        keybindingId: "switch_workspace"
      ) { [weak controller] in controller?.showPalette(prefix: "w:") },
      AppCommand(
        id: "open_workspace", title: "Open Folder as Workspace…", category: "Workspaces",
        keywords: ["project", "folder", "open"], icon: .folderOpen
      ) { [weak controller] in controller?.presentOpenWorkspacePanel() },
      AppCommand(
        id: "rename_workspace", title: "Rename Workspace…", category: "Workspaces",
        keywords: ["project", "name"], icon: .pencil
      ) { [weak controller] in
        guard let controller else { return }
        controller.presentRenameWorkspace(controller.tabManager.activeWorkspaceID)
      },
      AppCommand(
        id: "close_workspace", title: "Close Workspace", category: "Workspaces",
        keywords: ["project", "folder"], icon: .x
      ) { [weak controller] in
        guard let controller else { return }
        controller.requestCloseWorkspace(controller.tabManager.activeWorkspaceID)
      },
      AppCommand(
        id: "switch_tab", title: "Switch Tab…", category: "Tabs",
        keywords: ["go to tab"], icon: .layers
      ) { [weak controller] in controller?.showPalette(prefix: "t:") },
      AppCommand(
        id: "search_text", title: "Search Text in Project…", category: "Navigation",
        keywords: ["grep", "find"], icon: .search
      ) { [weak controller] in controller?.showPalette(prefix: "%") },
    ]

    let paneCommands: [(id: String, title: String, keywords: [String], icon: LucideIcon)] = [
      ("split_right", "Split Right", ["pane", "vertical split", "side by side"], .columns2),
      ("split_down", "Split Down", ["pane", "horizontal split", "stack"], .rows2),
      ("focus_pane_left", "Focus Pane Left", ["pane", "move"], .chevronLeft),
      ("focus_pane_right", "Focus Pane Right", ["pane", "move"], .chevronRight),
      ("focus_pane_up", "Focus Pane Above", ["pane", "move"], .chevronUp),
      ("focus_pane_down", "Focus Pane Below", ["pane", "move"], .chevronDown),
      ("next_pane", "Next Pane", ["pane", "cycle"], .chevronRight),
      ("prev_pane", "Previous Pane", ["pane", "cycle"], .chevronLeft),
      ("resize_pane_left", "Grow Pane Left", ["pane", "resize", "wider"], .chevronLeft),
      ("resize_pane_right", "Grow Pane Right", ["pane", "resize", "wider"], .chevronRight),
      ("resize_pane_up", "Grow Pane Up", ["pane", "resize", "taller"], .chevronUp),
      ("resize_pane_down", "Grow Pane Down", ["pane", "resize", "taller"], .chevronDown),
      ("zoom_pane", "Zoom Pane", ["pane", "maximize", "fullscreen"], .maximize2),
      ("equalize_panes", "Even Out Panes", ["pane", "equalize", "balance"], .columns2),
      ("move_pane_to_tab", "Move Pane to New Tab", ["pane", "pop out", "detach"], .externalLink),
    ]
    for command in paneCommands {
      let id = command.id
      result.append(
        AppCommand(
          id: id, title: command.title, category: "Panes", keywords: command.keywords,
          icon: command.icon, keybindingId: id
        ) { [weak controller] in controller?.performPaneCommand(id) })
    }

    // View ▸ Command Blocks (Show Hints is above), on the focused terminal.
    let blockCommands: [(id: String, title: String, keywords: [String], icon: LucideIcon)] = [
      ("select_blocks", "Select Blocks", ["block", "copy", "output"], .squareTerminal),
      ("previous_block", "Previous Block", ["block", "command", "scroll"], .chevronUp),
      ("next_block", "Next Block", ["block", "command", "scroll"], .chevronDown),
      ("last_failed_block", "Last Failed Block", ["block", "error", "exit"], .triangleAlert),
      ("toggle_block_bookmark", "Bookmark Block", ["block", "mark", "pin"], .bookmark),
      ("previous_block_bookmark", "Previous Bookmark", ["block", "mark"], .chevronUp),
      ("next_block_bookmark", "Next Bookmark", ["block", "mark"], .chevronDown),
    ]
    for command in blockCommands {
      let id = command.id
      result.append(
        AppCommand(
          id: id, title: command.title, category: "Terminal", keywords: command.keywords,
          icon: command.icon, keybindingId: id
        ) { [weak controller] in
          controller?.tabManager.selectedTerminal?.activeTerminal?.performBlockCommand(id)
        })
    }

    if AppState.isDev {
      result.append(
        AppCommand(
          id: "component_gallery", title: "Component Gallery", category: "Developer",
          keywords: ["themes", "design", "debug"], icon: .eye
        ) { ComponentGalleryWindowController.show() })
    }

    for custom in customKeybindings where !custom.name.isEmpty {
      let command = custom.command
      let args = custom.args
      result.append(
        AppCommand(
          id: "custom_\(custom.name)", title: custom.name, category: "Custom",
          keywords: [command], icon: .squareTerminal,
          shortcut: custom.key.isEmpty ? nil : Keybindings.symbolDisplay(shortcut: custom.key)
        ) {
          NotificationCenter.default.post(
            name: Notification.Name("impulseCustomCommand"), object: nil,
            userInfo: ["command": command, "args": args])
        })
    }
    return result
  }

  private static func keybindingId(forBuiltin id: String) -> String? {
    Keybindings.builtins.contains(where: { $0.id == id }) ? id : nil
  }

  private static func icon(forBuiltin id: String) -> LucideIcon? {
    switch id {
    case "new_tab": return .plus
    case "close_tab": return .x
    case "reopen_tab": return .undo2
    case "next_tab": return .chevronRight
    case "prev_tab": return .chevronLeft
    case "copy": return .copy
    case "paste": return .clipboard
    case "review_changes": return .fileDiff
    case "new_file": return .filePlus
    case "save": return .check
    case "find": return .search
    case "go_to_line": return .cornerDownLeft
    case "toggle_markdown_preview": return .eye
    case "toggle_sidebar": return .panelLeft
    case "quick_open": return .file
    case "project_search": return .search
    case "command_palette": return .command
    case "open_settings": return .settings
    case "font_increase", "font_decrease", "font_reset": return .slidersHorizontal
    case "new_window": return .externalLink
    case "fullscreen": return .maximize2
    case "install_lsp": return .package
    default: return nil
    }
  }

  private static func builtinAction(_ id: String, controller: MainWindowController) -> () -> Void
  {
    let notificationMap: [String: Notification.Name] = [
      "new_tab": .impulseNewTerminalTab,
      "close_tab": .impulseCloseTab,
      "reopen_tab": .impulseReopenTab,
      "next_tab": .impulseNextTab,
      "prev_tab": .impulsePrevTab,
      "new_file": .impulseNewFile,
      "save": .impulseSaveFile,
      "find": .impulseFind,
      "go_to_line": .impulseGoToLine,
      "toggle_markdown_preview": .impulseToggleMarkdownPreview,
      "toggle_sidebar": .impulseToggleSidebar,
      "project_search": .impulseFindInProject,
      "install_lsp": .impulseInstallLsp,
      "font_increase": .impulseFontIncrease,
      "font_decrease": .impulseFontDecrease,
      "font_reset": .impulseFontReset,
      "review_changes": .impulseReviewChanges,
    ]
    if let name = notificationMap[id] {
      return { NotificationCenter.default.post(name: name, object: nil) }
    }
    switch id {
    case "quick_open":
      return { [weak controller] in controller?.showPalette(prefix: "") }
    case "command_palette":
      return { [weak controller] in controller?.showPalette(prefix: ">") }
    case "copy":
      return { NSApp.sendAction(#selector(NSText.copy(_:)), to: nil, from: nil) }
    case "paste":
      return { NSApp.sendAction(#selector(NSText.paste(_:)), to: nil, from: nil) }
    case "open_settings":
      return { (NSApp.delegate as? AppDelegate)?.showPreferences(nil) }
    case "new_window":
      return { (NSApp.delegate as? AppDelegate)?.newWindow(nil) }
    case "fullscreen":
      return { NSApp.keyWindow?.toggleFullScreen(nil) }
    default:
      return { NSSound.beep() }
    }
  }
}
