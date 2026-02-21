import Foundation

// MARK: - Centralized Notification Names

extension Notification.Name {

    // MARK: App Lifecycle & Settings

    /// Posted when the application-wide color theme changes.
    static let impulseThemeDidChange = Notification.Name("impulseThemeDidChange")
    /// Posted when settings are changed (e.g. from the settings window).
    static let impulseSettingsDidChange = Notification.Name("impulseSettingsDidChange")

    // MARK: Tab Management

    /// Posted when the active tab changes (used by status bar).
    static let impulseActiveTabDidChange = Notification.Name("impulseActiveTabDidChange")
    /// Requests a new terminal tab in the frontmost window.
    static let impulseNewTerminalTab = Notification.Name("impulseNewTerminalTab")
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

    // MARK: Terminal Commands

    /// Requests splitting the terminal horizontally.
    static let impulseSplitHorizontal = Notification.Name("impulseSplitHorizontal")
    /// Requests splitting the terminal vertically.
    static let impulseSplitVertical = Notification.Name("impulseSplitVertical")
    /// Requests moving focus to the previous terminal split pane.
    static let impulseFocusPrevSplit = Notification.Name("impulseFocusPrevSplit")
    /// Requests moving focus to the next terminal split pane.
    static let impulseFocusNextSplit = Notification.Name("impulseFocusNextSplit")

    // MARK: UI Commands

    /// Requests toggling the sidebar.
    static let impulseToggleSidebar = Notification.Name("impulseToggleSidebar")
    /// Requests showing the command palette.
    static let impulseShowCommandPalette = Notification.Name("impulseShowCommandPalette")
    /// Requests project-wide find.
    static let impulseFindInProject = Notification.Name("impulseFindInProject")

    // MARK: File Tree

    /// Posted when the user selects a file in the file tree. The `userInfo`
    /// dictionary contains `"path"` (String) and optionally `"line"` (Int).
    static let impulseOpenFile = Notification.Name("dev.impulse.openFile")
}
