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
        id: "toggle_right_dock", title: "Toggle Right Panel", category: "View",
        keywords: ["dock", "panel", "review"], icon: .panelRight,
        keybindingId: "toggle_right_dock"
      ) { [weak controller] in controller?.toggleRightDock() },
      AppCommand(
        id: "show_changes", title: "Show Changes", category: "Git",
        keywords: ["git", "stage", "commit", "status"], icon: .gitBranch,
        keybindingId: "show_changes"
      ) { [weak controller] in controller?.showChangesPanel() },
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
        id: "pull_request", title: "Open or Create Pull Request", category: "Git",
        keywords: ["github", "gh", "pr", "review"], icon: .gitPullRequest
      ) { [weak controller] in controller?.openOrCreatePullRequest() },
      AppCommand(
        id: "git_history", title: "Show Git History", category: "Git",
        keywords: ["log", "commits", "graph", "blame"], icon: .history
      ) { [weak controller] in controller?.showHistory() },
      AppCommand(
        id: "file_history", title: "Show History of This File", category: "Git",
        keywords: ["log", "commits", "blame"], icon: .history
      ) { [weak controller] in
        guard let controller else { return }
        controller.showHistory(path: controller.tabManager.selectedEditor?.filePath)
      },
      AppCommand(
        id: "new_task", title: "New Task…", category: "Workspaces",
        keywords: ["worktree", "branch", "agent", "parallel"], icon: .gitBranchPlus
      ) { [weak controller] in controller?.presentNewTaskSheet() },
      AppCommand(
        id: "archive_task", title: "Archive This Task…", category: "Workspaces",
        keywords: ["worktree", "remove", "done"], icon: .archive
      ) { [weak controller] in
        guard let controller else { return }
        controller.archiveTask(controller.tabManager.activeWorkspaceID)
      },
      AppCommand(
        id: "review_agent_turn", title: "Review Last Agent Turn", category: "Agents",
        keywords: ["claude", "codex", "diff", "checkpoint", "changes"], icon: .fileDiff
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
        id: "next_agent", title: "Go to Next Agent Needing You", category: "Agents",
        keywords: ["claude", "codex", "inbox", "waiting"], icon: .bot, keybindingId: "next_agent"
      ) { [weak controller] in controller?.revealNextWaitingAgent() },
      AppCommand(
        id: "command_history", title: "Search Command History…", category: "Terminal",
        keywords: ["history", "recent", "ctrl-r"], icon: .history
      ) { [weak controller] in controller?.showPalette(prefix: "h:") },
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
