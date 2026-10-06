import AppKit
import Foundation
import ImpulseKit

/// Every user-facing setting, described once: where it shows in the
/// Settings tab, how it's edited, its default, and its JSON Schema entry for
/// settings.json. Complex settings (lists, maps) are edited in settings.json
/// or their own panes and only appear in the schema.
struct SettingItem: Identifiable {
  enum Category: String, CaseIterable, Identifiable {
    case general = "General"
    case editor = "Editor"
    case terminal = "Terminal"
    case appearance = "Appearance"
    case git = "Git"
    case automation = "Automation"
    case languageServers = "Language Servers"
    case advanced = "Advanced"

    var id: String { rawValue }
    var icon: LucideIcon {
      switch self {
      case .general: return .settings
      case .editor: return .fileCode
      case .terminal: return .squareTerminal
      case .appearance: return .eye
      case .git: return .gitBranch
      case .automation: return .zap
      case .languageServers: return .plug
      case .advanced: return .slidersHorizontal
      }
    }
  }

  enum Control {
    case toggle(WritableKeyPath<Settings, Bool>)
    case integer(WritableKeyPath<Settings, Int>, range: ClosedRange<Int>, step: Int, zeroLabel: String?)
    case decimal(WritableKeyPath<Settings, Double>, range: ClosedRange<Double>, step: Double)
    case text(WritableKeyPath<Settings, String>, placeholder: String)
    case font(WritableKeyPath<Settings, String>)
    case choice(WritableKeyPath<Settings, String>, options: [(value: String, label: String)])
    case theme(WritableKeyPath<Settings, String>)
  }

  /// The settings.json key.
  let key: String
  let title: String
  let detail: String?
  let category: Category
  let section: String
  let control: Control
  let isModified: (Settings) -> Bool
  let reset: (inout Settings) -> Void
  /// JSON Schema for the value.
  let schema: [String: Any]

  var id: String { key }

  func matches(_ query: String) -> Bool {
    let q = query.lowercased()
    return title.lowercased().contains(q) || key.contains(q) || section.lowercased().contains(q)
      || (detail?.lowercased().contains(q) ?? false)
  }
}

enum SettingsCatalog {
  private static let defaults = Settings.default

  static let items: [SettingItem] = general + editor + terminal + appearance + git

  // MARK: Builders

  private static func toggle(
    _ key: String, _ title: String, _ kp: WritableKeyPath<Settings, Bool>, _ category: SettingItem.Category,
    _ section: String, detail: String? = nil
  ) -> SettingItem {
    SettingItem(
      key: key, title: title, detail: detail, category: category, section: section, control: .toggle(kp),
      isModified: { $0[keyPath: kp] != defaults[keyPath: kp] },
      reset: { $0[keyPath: kp] = defaults[keyPath: kp] },
      schema: ["type": "boolean", "default": defaults[keyPath: kp]])
  }

  private static func integer(
    _ key: String, _ title: String, _ kp: WritableKeyPath<Settings, Int>, _ category: SettingItem.Category,
    _ section: String, range: ClosedRange<Int>, step: Int = 1, zeroLabel: String? = nil, detail: String? = nil
  ) -> SettingItem {
    SettingItem(
      key: key, title: title, detail: detail, category: category, section: section,
      control: .integer(kp, range: range, step: step, zeroLabel: zeroLabel),
      isModified: { $0[keyPath: kp] != defaults[keyPath: kp] },
      reset: { $0[keyPath: kp] = defaults[keyPath: kp] },
      schema: [
        "type": "integer", "default": defaults[keyPath: kp], "minimum": range.lowerBound,
        "maximum": range.upperBound,
      ])
  }

  private static func decimal(
    _ key: String, _ title: String, _ kp: WritableKeyPath<Settings, Double>, _ category: SettingItem.Category,
    _ section: String, range: ClosedRange<Double>, step: Double, detail: String? = nil
  ) -> SettingItem {
    SettingItem(
      key: key, title: title, detail: detail, category: category, section: section,
      control: .decimal(kp, range: range, step: step),
      isModified: { $0[keyPath: kp] != defaults[keyPath: kp] },
      reset: { $0[keyPath: kp] = defaults[keyPath: kp] },
      schema: [
        "type": "number", "default": defaults[keyPath: kp], "minimum": range.lowerBound,
        "maximum": range.upperBound,
      ])
  }

  private static func choice(
    _ key: String, _ title: String, _ kp: WritableKeyPath<Settings, String>, _ category: SettingItem.Category,
    _ section: String, _ options: [(String, String)], detail: String? = nil
  ) -> SettingItem {
    SettingItem(
      key: key, title: title, detail: detail, category: category, section: section,
      control: .choice(kp, options: options.map { (value: $0.0, label: $0.1) }),
      isModified: { $0[keyPath: kp] != defaults[keyPath: kp] },
      reset: { $0[keyPath: kp] = defaults[keyPath: kp] },
      schema: ["type": "string", "default": defaults[keyPath: kp], "enum": options.map(\.0)])
  }

  private static func font(
    _ key: String, _ title: String, _ kp: WritableKeyPath<Settings, String>, _ category: SettingItem.Category,
    _ section: String, detail: String? = nil
  ) -> SettingItem {
    SettingItem(
      key: key, title: title, detail: detail, category: category, section: section, control: .font(kp),
      isModified: { $0[keyPath: kp] != defaults[keyPath: kp] },
      reset: { $0[keyPath: kp] = defaults[keyPath: kp] },
      schema: ["type": "string", "default": defaults[keyPath: kp]])
  }

  // MARK: Catalog

  private static let general: [SettingItem] = [
    toggle(
      "quick_terminal_enabled", "Quick terminal", \.quickTerminalEnabled, .general, "Quick terminal",
      detail: "A terminal that drops down from the top of the screen, over any app, on a global shortcut."),
    SettingItem(
      key: "quick_terminal_shortcut", title: "Quick terminal shortcut", detail: "For example Ctrl+` or Alt+Space.",
      category: .general, section: "Quick terminal", control: .text(\.quickTerminalShortcut, placeholder: "Ctrl+`"),
      isModified: { $0.quickTerminalShortcut != defaults.quickTerminalShortcut },
      reset: { $0.quickTerminalShortcut = defaults.quickTerminalShortcut },
      schema: ["type": "string", "default": defaults.quickTerminalShortcut]),
    toggle(
      "restore_session", "Restore previous session", \.restoreSession, .general, "Startup",
      detail: "Reopen workspaces, tabs, splits and terminal folders on launch."),
    toggle(
      "restore_scrollback", "Restore terminal scrollback", \.restoreScrollback, .general, "Startup",
      detail: "Bring back each terminal's output when the session is restored."),
    toggle("check_for_updates", "Check for updates on launch", \.checkForUpdates, .general, "Startup"),
    toggle(
      "confirm_close_warnings", "Warn before closing active work", \.confirmCloseWarnings, .general, "Window",
      detail: "Ask before closing unsaved files or terminals with running commands."),
    toggle("sidebar_show_hidden", "Show hidden files", \.sidebarShowHidden, .general, "Sidebar"),
  ]

  private static let editor: [SettingItem] = [
    font("font_family", "Font family", \.fontFamily, .editor, "Font"),
    integer("font_size", "Font size", \.fontSize, .editor, "Font", range: 6...72),
    toggle("font_ligatures", "Font ligatures", \.fontLigatures, .editor, "Font"),
    integer(
      "editor_line_height", "Line height", \.editorLineHeight, .editor, "Font", range: 0...50,
      zeroLabel: "Default", detail: "In points; 0 uses the font's own."),
    integer("tab_width", "Tab width", \.tabWidth, .editor, "Indentation", range: 1...16),
    toggle("use_spaces", "Insert spaces instead of tabs", \.useSpaces, .editor, "Indentation"),
    toggle("indent_guides", "Indent guides", \.indentGuides, .editor, "Indentation"),
    toggle("show_line_numbers", "Line numbers", \.showLineNumbers, .editor, "Display"),
    toggle("word_wrap", "Word wrap", \.wordWrap, .editor, "Display"),
    toggle("minimap_enabled", "Minimap", \.minimapEnabled, .editor, "Display"),
    toggle("highlight_current_line", "Highlight current line", \.highlightCurrentLine, .editor, "Display"),
    toggle("bracket_pair_colorization", "Bracket pair colors", \.bracketPairColorization, .editor, "Display"),
    choice(
      "render_whitespace", "Render whitespace", \.renderWhitespace, .editor, "Display",
      [("none", "None"), ("boundary", "Boundary"), ("selection", "Selection"), ("trailing", "Trailing"),
       ("all", "All")]),
    toggle("show_right_margin", "Right margin", \.showRightMargin, .editor, "Display"),
    integer("right_margin_position", "Right margin column", \.rightMarginPosition, .editor, "Display", range: 1...500),
    toggle("sticky_scroll", "Sticky scroll", \.stickyScroll, .editor, "Scrolling",
      detail: "Keep the enclosing scopes pinned at the top while scrolling."),
    toggle("scroll_beyond_last_line", "Scroll beyond last line", \.scrollBeyondLastLine, .editor, "Scrolling"),
    toggle("smooth_scrolling", "Smooth scrolling", \.smoothScrolling, .editor, "Scrolling"),
    integer(
      "editor_cursor_surrounding_lines", "Lines kept around the cursor", \.editorCursorSurroundingLines, .editor,
      "Scrolling", range: 0...20),
    choice(
      "editor_cursor_style", "Cursor style", \.editorCursorStyle, .editor, "Cursor",
      [("line", "Line"), ("block", "Block"), ("underline", "Underline"), ("line-thin", "Thin line"),
       ("block-outline", "Block outline"), ("underline-thin", "Thin underline")]),
    choice(
      "editor_cursor_blinking", "Cursor blinking", \.editorCursorBlinking, .editor, "Cursor",
      [("blink", "Blink"), ("smooth", "Smooth"), ("phase", "Phase"), ("expand", "Expand"), ("solid", "Solid")]),
    choice(
      "editor_auto_closing_brackets", "Auto-close brackets", \.editorAutoClosingBrackets, .editor, "Behavior",
      [("always", "Always"), ("languageDefined", "Language defined"), ("beforeWhitespace", "Before whitespace"),
       ("never", "Never")]),
    toggle("folding", "Code folding", \.folding, .editor, "Behavior"),
    toggle("auto_save", "Save when focus leaves the editor", \.autoSave, .editor, "Behavior"),
    toggle(
      "editor_vim_mode", "Vim keybindings", \.editorVimMode, .editor, "Behavior",
      detail: "Normal, insert and visual modes in the editor (monaco-vim); the mode shows at the bottom right."),
    toggle(
      "editor_preview_tabs", "Preview files from the file tree", \.editorPreviewTabs, .editor, "Behavior",
      detail: "A click shows a file in an italic tab the next click reuses; double-click or edit it to keep it."),
    toggle(
      "editor_selection_highlight", "Highlight matches of the selection", \.editorSelectionHighlight, .editor,
      "Behavior"),
    toggle(
      "editor_occurrences_highlight", "Highlight occurrences of the symbol", \.editorOccurrencesHighlight,
      .editor, "Behavior"),
    choice(
      "editor_inlay_hints", "Inlay hints", \.editorInlayHints, .editor, "Behavior",
      [("on", "On"), ("off", "Off"), ("offUnlessPressed", "While holding ⌃⌥"),
       ("onUnlessPressed", "Hidden while holding ⌃⌥")],
      detail: "Types and parameter names from the language server, shown inline."),
    choice(
      "editor_word_based_suggestions", "Word-based suggestions", \.editorWordBasedSuggestions, .editor,
      "Behavior",
      [("off", "Off"), ("currentDocument", "Current file"), ("matchingDocuments", "Files of the same language"),
       ("allDocuments", "All open files")]),
  ]

  private static let terminal: [SettingItem] = [
    font("terminal_font_family", "Font family", \.terminalFontFamily, .terminal, "Font"),
    integer("terminal_font_size", "Font size", \.terminalFontSize, .terminal, "Font", range: 6...72),
    toggle(
      "terminal_bold_is_bright", "Bold text uses bright colors", \.terminalBoldIsBright, .terminal, "Font"),
    decimal(
      "terminal_minimum_contrast", "Minimum contrast", \.terminalMinimumContrast, .terminal, "Font",
      range: 1...21, step: 0.5, detail: "Lift text colors to at least this contrast ratio (1 turns it off)."),
    choice(
      "terminal_cursor_shape", "Cursor shape", \.terminalCursorShape, .terminal, "Cursor",
      [("block", "Block"), ("underline", "Underline"), ("beam", "Beam")]),
    toggle("terminal_cursor_blink", "Blinking cursor", \.terminalCursorBlink, .terminal, "Cursor"),
    toggle(
      "terminal_blocks", "Command blocks", \.terminalBlocks, .terminal, "Blocks & input",
      detail: "Separate each command and its output, with status, actions and navigation."),
    toggle(
      "terminal_context_bar", "Input bar", \.terminalContextBar, .terminal, "Blocks & input",
      detail: "Type commands in Impulse's editor below the output instead of at the shell prompt."),
    toggle(
      "terminal_persistent_history", "Keep command history", \.terminalPersistentHistory, .terminal,
      "Blocks & input", detail: "Remember commands across sessions and terminals (stored locally)."),
    toggle(
      "terminal_shell_completions", "Ask the shell for completions", \.terminalShellCompletions, .terminal,
      "Blocks & input",
      detail: "Add fish's own completions (its completion scripts and descriptions) to the Tab menu."),
    toggle(
      "terminal_editor_integration", "Use Impulse as $EDITOR", \.terminalEditorIntegration, .terminal,
      "Blocks & input", detail: "git commit and friends open files in an Impulse tab."),
    toggle(
      "agent_composer_auto_show", "Open the composer when an agent needs input", \.agentComposerAutoShow,
      .terminal, "Agents", detail: "In the focused terminal, so you can answer without clicking into the agent."),
    toggle("terminal_copy_on_select", "Copy on select", \.terminalCopyOnSelect, .terminal, "Behavior"),
    toggle("terminal_scroll_on_output", "Scroll to bottom on output", \.terminalScrollOnOutput, .terminal, "Behavior"),
    toggle("terminal_allow_hyperlink", "Clickable links", \.terminalAllowHyperlink, .terminal, "Behavior"),
    toggle(
      "terminal_allow_osc52_write", "Programs may set the clipboard", \.terminalAllowOsc52Write, .terminal,
      "Clipboard", detail: "OSC 52 writes, as used by tmux, vim and ssh sessions."),
    toggle(
      "terminal_allow_osc52_read", "Programs may read the clipboard", \.terminalAllowOsc52Read, .terminal,
      "Clipboard"),
    toggle("terminal_bell", "Audible bell", \.terminalBell, .terminal, "Bell & notifications"),
    toggle("terminal_attention_on_bell", "Request attention on bell", \.terminalAttentionOnBell, .terminal,
      "Bell & notifications"),
    toggle(
      "terminal_allow_notifications", "Allow terminal notifications", \.terminalAllowNotifications, .terminal,
      "Bell & notifications", detail: "OSC 9 / 99 / 777 notifications from programs and agents."),
    toggle(
      "terminal_attention_on_long_command", "Notify when long commands finish",
      \.terminalAttentionOnLongCommand, .terminal, "Bell & notifications"),
    integer(
      "terminal_long_command_seconds", "Long command threshold", \.terminalLongCommandSeconds, .terminal,
      "Bell & notifications", range: 1...3600, detail: "Seconds."),
    integer(
      "terminal_scrollback", "Scrollback lines", \.terminalScrollback, .terminal, "Scrollback",
      range: 100...1_000_000, step: 1000),
  ]

  private static let git: [SettingItem] = [
    toggle(
      "git_commit_and_push", "Commit and push", \.gitCommitAndPush, .git, "Commit",
      detail: "The commit button and ⌘↩ push right after committing; ⇧⌘↩ only commits."),
    choice(
      "git_pull_mode", "When pulling", \.gitPullMode, .git, "Remote",
      [("ff-only", "Fast-forward only"), ("rebase", "Rebase local commits"), ("merge", "Merge")],
      detail: "Fast-forward only never makes a merge commit; it stops when the branch has diverged."),
    toggle(
      "git_push_follow_tags", "Push annotated tags with commits", \.gitPushFollowTags, .git, "Remote",
      detail: "Push uses --follow-tags: annotated tags on the commits being pushed go along."),
    integer(
      "git_auto_fetch_minutes", "Fetch in the background", \.gitAutoFetchMinutes, .git, "Remote",
      range: 0...120, step: 5, zeroLabel: "Off",
      detail: "Minutes between fetches of the repositories open in windows, so ahead/behind stays current. Never asks for credentials."),
    toggle(
      "git_push_tags_on_create", "Push new tags to the remote", \.gitPushTagsOnCreate, .git, "Tags",
      detail: "Creating a tag pushes it right away. The Create Tag sheet's checkbox starts from this."),
  ]

  private static let appearance: [SettingItem] = [
    SettingItem(
      key: "color_scheme", title: "Theme", detail: "Colors for the window, editor and terminal.",
      category: .appearance, section: "Theme", control: .theme(\.colorScheme),
      isModified: { $0.colorScheme != defaults.colorScheme },
      reset: { $0.colorScheme = defaults.colorScheme },
      schema: ["type": "string", "default": defaults.colorScheme, "enum": ThemeManager.availableThemes()])
  ]

  // MARK: JSON Schema

  /// A JSON Schema (draft-07) for settings.json: every catalog setting plus
  /// the list- and map-valued ones edited in the file itself.
  static func jsonSchema() -> [String: Any] {
    var properties: [String: Any] = [:]
    for item in items {
      var entry = item.schema
      entry["description"] = [item.title, item.detail].compactMap { $0 }.joined(separator: ". ")
      properties[item.key] = entry
    }
    let stringArray: [String: Any] = ["type": "array", "items": ["type": "string"]]
    properties["keybinding_overrides"] = [
      "type": "object", "description": "Shortcut per command id, e.g. \"split_right\": \"Cmd+D\" (\"none\" removes it).",
      "additionalProperties": ["type": "string"],
    ]
    properties["custom_keybindings"] = [
      "type": "array", "description": "Shortcuts that run a shell command in the active terminal.",
      "items": [
        "type": "object", "required": ["name", "key", "command"],
        "properties": [
          "name": ["type": "string"], "key": ["type": "string"], "command": ["type": "string"], "args": stringArray,
        ],
      ],
    ]
    properties["commands_on_save"] = [
      "type": "array", "description": "Commands run after saving matching files.",
      "items": [
        "type": "object", "required": ["name", "command", "file_pattern"],
        "properties": [
          "name": ["type": "string"], "command": ["type": "string"], "args": stringArray,
          "file_pattern": ["type": "string"], "reload_file": ["type": "boolean"],
        ],
      ],
    ]
    properties["file_type_overrides"] = [
      "type": "array", "description": "Indentation and formatting per file pattern.",
      "items": [
        "type": "object", "required": ["pattern"],
        "properties": [
          "pattern": ["type": "string"], "tab_width": ["type": "integer", "minimum": 1],
          "use_spaces": ["type": "boolean"],
          "format_on_save": [
            "type": "object", "properties": ["command": ["type": "string"], "args": stringArray],
          ],
        ],
      ],
    ]
    // Window state Impulse writes itself.
    for key in ["window_width", "window_height", "sidebar_width"] {
      properties[key] = ["type": "integer"]
    }
    properties["sidebar_visible"] = ["type": "boolean"]
    properties["tab_bar_position"] = ["type": "string", "enum": ["sidebar", "top"]]
    properties["last_directory"] = ["type": "string"]
    properties["open_files"] = stringArray
    return [
      "$schema": "http://json-schema.org/draft-07/schema#",
      "title": "Impulse settings",
      "type": "object",
      "properties": properties,
    ]
  }
}
