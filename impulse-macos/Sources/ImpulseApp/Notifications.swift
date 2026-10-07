import Foundation

// MARK: - Centralized Notification Names

extension Notification.Name {

    // MARK: App Lifecycle & Settings
    /// Posted when settings are changed (e.g. from the settings window).
    static let impulseSettingsDidChange = Notification.Name("impulseSettingsDidChange")

    // MARK: Tab Management

    /// Posted when the active tab changes (used by status bar).
    static let impulseActiveTabDidChange = Notification.Name("impulseActiveTabDidChange")
    /// Requests a new terminal tab in the frontmost window.
    static let impulseNewTerminalTab = Notification.Name("impulseNewTerminalTab")
    /// Requests a new untitled editor tab in the frontmost window.
    static let impulseNewFile = Notification.Name("impulseNewFile")
    /// Requests closing the current tab in the frontmost window.
    static let impulseCloseTab = Notification.Name("impulseCloseTab")
    /// Requests reopening the most recently closed tab.
    static let impulseReopenTab = Notification.Name("impulseReopenTab")
    /// Requests switching to the next tab.
    static let impulseNextTab = Notification.Name("impulseNextTab")
    /// Requests switching to the previous tab.
    static let impulsePrevTab = Notification.Name("impulsePrevTab")
    /// Requests switching to a specific tab by index (0-based in userInfo "index").
    static let impulseSelectTab = Notification.Name("impulseSelectTab")

    // MARK: Editor Events

    /// Posted when the cursor position changes. The `userInfo` dictionary contains
    /// `"line"` and `"column"` as `UInt32` values.
    static let editorCursorMoved = Notification.Name("impulse.editorCursorMoved")
    /// Posted when the editor content is modified. The `userInfo` dictionary contains
    /// `"filePath"` as a `String`.
    static let editorContentChanged = Notification.Name("impulse.editorContentChanged")
    /// Text pasted or dropped on a terminal whose input bar owns input
    /// (object: TerminalTab, userInfo "text").
    static let terminalInsertIntoInputBar = Notification.Name("impulse.terminalInsertIntoInputBar")
    /// An editor's file changed on disk while it had unsaved edits (object: EditorTab).
    static let editorChangedOnDisk = Notification.Name("impulse.editorChangedOnDisk")
    /// Posted when a completion request is received from Monaco. The `userInfo`
    /// dictionary contains `"requestId"`, `"line"`, and `"character"`.
    static let editorCompletionRequested = Notification.Name("impulse.editorCompletionRequested")
    /// Posted when a hover request is received from Monaco. The `userInfo`
    /// dictionary contains `"requestId"`, `"line"`, and `"character"`.
    static let editorHoverRequested = Notification.Name("impulse.editorHoverRequested")
    /// Posted when a go-to-definition request is received. The `userInfo`
    /// dictionary contains `"line"` and `"character"`.
    static let editorDefinitionRequested = Notification.Name("impulse.editorDefinitionRequested")
    /// Posted when Monaco wants to open a different file (cross-file definition).
    static let editorOpenFileRequested = Notification.Name("impulse.editorOpenFileRequested")
    /// Posted when the editor focus state changes. The `userInfo` dictionary
    /// contains `"focused"` as a `Bool`.
    static let editorFocusChanged = Notification.Name("impulse.editorFocusChanged")
    /// Posted after Monaco finishes processing an `OpenFile` command, meaning
    /// the new model is set up and ready for decorations. The `object` is the
    /// `EditorTab` and the `userInfo` dictionary contains `"filePath"`.
    static let editorFileOpened = Notification.Name("impulse.editorFileOpened")
    /// Posted when a formatting request is received from Monaco. The `userInfo`
    /// dictionary contains `"requestId"`, `"tabSize"`, and `"insertSpaces"`.
    static let editorFormattingRequested = Notification.Name("impulse.editorFormattingRequested")
    /// Posted when a signature help request is received from Monaco. The `userInfo`
    /// dictionary contains `"requestId"`, `"line"`, and `"character"`.
    static let editorSignatureHelpRequested = Notification.Name("impulse.editorSignatureHelpRequested")
    /// Posted when a references request is received from Monaco. The `userInfo`
    /// dictionary contains `"requestId"`, `"line"`, and `"character"`.
    static let editorReferencesRequested = Notification.Name("impulse.editorReferencesRequested")
    /// Posted when a code action request is received from Monaco. The `userInfo`
    /// dictionary contains `"requestId"`, `"startLine"`, `"startColumn"`, `"endLine"`,
    /// `"endColumn"`, and `"diagnostics"`.
    static let editorCodeActionRequested = Notification.Name("impulse.editorCodeActionRequested")
    /// Posted when a rename request is received from Monaco. The `userInfo`
    /// dictionary contains `"requestId"`, `"line"`, `"character"`, and `"newName"`.
    static let editorRenameRequested = Notification.Name("impulse.editorRenameRequested")
    /// Posted when a prepare rename request is received from Monaco. The `userInfo`
    /// dictionary contains `"requestId"`, `"line"`, and `"character"`.
    static let editorPrepareRenameRequested = Notification.Name("impulse.editorPrepareRenameRequested")

    // MARK: Editor Commands

    /// Requests saving the current editor tab.
    static let impulseSaveFile = Notification.Name("impulseSaveFile")
    /// Requests toggling find in the terminal or editor.
    static let impulseFind = Notification.Name("impulseFind")
    /// Requests showing the go-to-line dialog.
    static let impulseGoToLine = Notification.Name("impulseGoToLine")
    /// Requests reloading an editor tab from disk (e.g. after discarding git changes).
    /// The `userInfo` dictionary contains `"path"` (String).
    static let impulseReloadEditorFile = Notification.Name("impulseReloadEditorFile")

    // MARK: Editor Font

    /// Requests increasing the editor and terminal font size.
    static let impulseFontIncrease = Notification.Name("impulseFontIncrease")
    /// Requests decreasing the editor and terminal font size.
    static let impulseFontDecrease = Notification.Name("impulseFontDecrease")
    /// Requests resetting the editor and terminal font size to defaults.
    static let impulseFontReset = Notification.Name("impulseFontReset")

    // MARK: Terminal Events

    /// Posted when the terminal title changes.
    static let terminalTitleChanged = Notification.Name("impulse.terminalTitleChanged")
    /// Posted when the terminal working directory changes.
    static let terminalCwdChanged = Notification.Name("impulse.terminalCwdChanged")
    /// Posted when a terminal process terminates.
    static let terminalProcessTerminated = Notification.Name("impulse.terminalProcessTerminated")
    /// Posted when a terminal tab's attention state changes.
    static let terminalAttentionChanged = Notification.Name("impulse.terminalAttentionChanged")
    /// Posted by a TerminalTab when its OSC 9;4 progress report changes.
    static let terminalProgressChanged = Notification.Name("impulse.terminalProgressChanged")
    /// Posted by an EditorTab when its unsaved-changes state flips.
    static let editorDirtyStateChanged = Notification.Name("impulse.editorDirtyStateChanged")
    /// Posted by an EditorTab when the git peek widget asks to stage a hunk or
    /// open the review (userInfo: action, line).
    static let editorGitAction = Notification.Name("impulse.editorGitAction")
    /// Run a palette command by id in the key window (menu items for
    /// commands without a dedicated action; userInfo["id"]).
    static let impulseRunCommand = Notification.Name("impulseRunCommand")
    static let editorCodeActionChosen = Notification.Name("impulse.editorCodeActionChosen")
    static let editorLspRequested = Notification.Name("impulse.editorLspRequested")
    /// Posted when a command block starts or ends in a terminal
    /// (userInfo["block"]: TerminalCommandBlock).
    static let terminalCommandBlockChanged = Notification.Name("impulse.terminalCommandBlockChanged")
    /// Posted when a terminal enters or leaves direct-interaction mode — a
    /// full-screen/raw TUI (alternate screen, or a running command that turned
    /// on bracketed-paste/mouse reporting) (userInfo["interactive"]: Bool).
    static let terminalInteractionModeChanged = Notification.Name(
      "impulse.terminalInteractionModeChanged")
    /// Posted when the read-only terminal grid is clicked and keyboard focus
    /// should move to the input bar.
    static let terminalRequestInputFocus = Notification.Name("impulse.terminalRequestInputFocus")
    /// A hint was picked in hints mode (object: TerminalTab; userInfo: kind,
    /// text, action, and for paths path/line/column; cwd).
    static let terminalHintChosen = Notification.Name("impulse.terminalHintChosen")
    /// Posted when the foreground program toggles password-style input on the
    /// PTY (termios ECHO off) and the input bar should mask what's typed
    /// (userInfo["active"]: Bool).
    static let terminalPasswordInputChanged = Notification.Name(
      "impulse.terminalPasswordInputChanged")

    // MARK: UI Commands

    /// Requests toggling the sidebar.
    static let impulseToggleSidebar = Notification.Name("impulseToggleSidebar")
    /// Requests showing the command palette.
    static let impulseShowCommandPalette = Notification.Name("impulseShowCommandPalette")
    /// Toggles the right dock in the frontmost window.
    /// A pane command (split, focus, zoom…); userInfo["command"] is its keybinding id.
    static let impulsePaneCommand = Notification.Name("impulsePaneCommand")
    /// The window switched workspaces (object: the TabManager).
    static let impulseActiveWorkspaceDidChange = Notification.Name("impulseActiveWorkspaceDidChange")
    /// Ask the key window to pick a folder to open as a workspace.
    static let impulseOpenWorkspace = Notification.Name("impulseOpenWorkspace")
    static let impulseSwitchWorkspace = Notification.Name("impulseSwitchWorkspace")
    /// A terminal asked for the history panel (object: the TerminalTab).
    static let impulseShowCommandHistory = Notification.Name("impulseShowCommandHistory")
    /// A terminal wants a desktop notification (object: the TerminalTab;
    /// userInfo "title", "body").
    static let terminalWantsNotification = Notification.Name("impulse.terminalWantsNotification")
    /// A terminal's coding agent appeared, changed state or left (object:
    /// the TerminalTab).
    static let terminalAgentChanged = Notification.Name("impulse.terminalAgentChanged")
    /// Go to the next agent waiting for the user.
    static let impulseNextAgent = Notification.Name("impulseNextAgent")
    /// Toggle ⌘I's prompt editor over the focused terminal's program.
    static let impulseAgentComposer = Notification.Name("impulseAgentComposer")
    /// Hand text to the most relevant running agent (userInfo "text"; the
    /// object, when a terminal, is never the target).
    static let impulseSendToAgent = Notification.Name("impulseSendToAgent")
    /// An editor tab closed (userInfo "path"); `impulse edit` waits for it.
    static let impulseEditorClosed = Notification.Name("impulseEditorClosed")
    /// Shows the git Changes panel in the frontmost window.
    static let impulseShowChanges = Notification.Name("impulseShowChanges")
    /// Opens the branch switcher in the frontmost window.
    static let impulseSwitchBranch = Notification.Name("impulseSwitchBranch")
    static let impulseManageBranches = Notification.Name("impulseManageBranches")
    static let impulseGoToSymbol = Notification.Name("impulseGoToSymbol")
    /// A preview's Run button (object: EditorTab; userInfo: command, directory).
    static let impulseRunInTerminal = Notification.Name("impulseRunInTerminal")
    /// A Command Blocks menu item (userInfo["command"]) for the focused terminal.
    static let impulseBlockCommand = Notification.Name("impulseBlockCommand")
    /// A pull request's checks finished (object: GitRepositoryState).
    static let pullRequestChecksFinished = Notification.Name("impulsePullRequestChecksFinished")
    /// Requests project-wide find.
    static let impulseFindInProject = Notification.Name("impulseFindInProject")
    /// Requests toggling markdown preview in the active editor tab.
    static let impulseToggleMarkdownPreview = Notification.Name("impulseToggleMarkdownPreview")
    /// Requests opening the Review Changes tab for the current workspace.
    static let impulseReviewChanges = Notification.Name("impulseReviewChanges")

    // MARK: File Tree

    /// Posted when the user selects a file in the file tree. The `userInfo`
    /// dictionary contains `"path"` (String) and optionally `"line"` (Int).
    static let impulseOpenFile = Notification.Name("dev.impulse.openFile")
    /// Posted when the file tree contents change (refresh, create, delete,
    /// rename). Observers like the search panel can re-run queries so results
    /// don't go stale.
    static let impulseFileTreeChanged = Notification.Name("impulse.fileTreeChanged")

    // MARK: Command Palette Actions

    /// Requests Go to File… (the palette's file mode).
    static let impulseQuickOpen = Notification.Name("impulseQuickOpen")
    /// Requests installing managed web LSP servers.
    static let impulseInstallLsp = Notification.Name("impulseInstallLsp")

    // MARK: Update Notifications

    /// Posted when a newer version is available. The `userInfo` dictionary
    /// contains `"version"` (String) and `"url"` (String).
    static let impulseUpdateAvailable = Notification.Name("impulseUpdateAvailable")
}
