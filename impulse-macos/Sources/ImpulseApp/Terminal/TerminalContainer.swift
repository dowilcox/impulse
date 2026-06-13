import AppKit

/// Wraps a single `TerminalTab` as the content of a terminal tab. (Split
/// panes were removed; this stays a one-terminal container so the surrounding
/// tab/session code keeps a stable shape.)
class TerminalContainer: NSView {

  // MARK: Public Properties

  /// The terminals in this container — always exactly one. Kept as an array so
  /// call sites that iterate panes continue to work unchanged.
  private(set) var terminals: [TerminalTab] = []

  /// Always 0; retained for call-site compatibility.
  private(set) var activeTerminalIndex: Int = 0

  /// The container's terminal, or nil if it hasn't been created yet.
  var activeTerminal: TerminalTab? { terminals.first }

  /// Whether the terminal is requesting attention.
  var needsAttention: Bool { terminals.contains { $0.needsAttention } }

  // MARK: Private Properties

  private var currentSettings: TerminalSettings
  private var currentTheme: TerminalTheme

  // MARK: Initializer

  init(
    frame frameRect: NSRect, settings: TerminalSettings, theme: TerminalTheme,
    initialCommand: String? = nil
  ) {
    self.currentSettings = settings
    self.currentTheme = theme
    super.init(frame: frameRect)

    let terminal = createTerminal()
    terminals.append(terminal)
    addSubview(terminal)
    constrainChildToFill(terminal)

    // Defer shell spawning until after Auto Layout has resolved the
    // terminal view's frame, ensuring the PTY starts with the correct
    // column/row dimensions. Spawning synchronously here would use the
    // pre-layout frame (missing the 8px padding insets), causing a
    // COLUMNS mismatch that breaks line wrapping and cursor navigation.
    let dir = settings.lastDirectory.isEmpty ? nil : settings.lastDirectory
    DispatchQueue.main.async {
      terminal.spawnShell(initialDirectory: dir, initialCommand: initialCommand)
    }
  }

  init(
    frame frameRect: NSRect,
    settings: TerminalSettings,
    theme: TerminalTheme,
    sessionTab: SessionTabState
  ) {
    self.currentSettings = settings
    self.currentTheme = theme
    super.init(frame: frameRect)

    let terminal = createTerminal()
    terminals.append(terminal)
    addSubview(terminal)
    constrainChildToFill(terminal)

    // Restore the active pane's working directory (older sessions may have
    // stored multiple split panes; only the active one is restored now).
    let cwd = restoredCwd(for: sessionTab)
    DispatchQueue.main.async {
      terminal.spawnShell(initialDirectory: cwd)
      terminal.focus()
    }
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) is not supported")
  }

  // MARK: Process Lifecycle

  /// Terminate the shell process. Must be called before the container is
  /// removed from the tab list so the child process is cleaned up.
  func terminateAllProcesses() {
    for terminal in terminals {
      terminal.terminateProcess()
    }
  }

  func runningDescendantProcessCount() -> Int {
    terminals.reduce(0) { $0 + $1.runningDescendantProcessCount() }
  }

  func runningCloseRiskCommands() -> [CloseRiskCommand] {
    terminals.compactMap { $0.runningCloseRiskCommand() }
  }

  // MARK: Session State

  func sessionSnapshot(shellName: String) -> TerminalSessionSnapshot? {
    guard let terminal = terminals.first else { return nil }
    let cwd = terminal.currentWorkingDirectory.isEmpty
      ? NSHomeDirectory()
      : terminal.currentWorkingDirectory
    let pane = SessionTerminalPaneState(
      cwd: cwd,
      title: nonEmptySessionText(terminal.tabTitle),
      shell: nonEmptySessionText(shellName)
    )
    return TerminalSessionSnapshot(
      panes: [pane],
      activePaneIndex: 0,
      paneLayout: .pane(paneIndex: 0)
    )
  }

  private func restoredCwd(for tab: SessionTabState) -> String {
    if let panes = tab.panes, !panes.isEmpty {
      let index = tab.activePaneIndex ?? 0
      let pane = panes.indices.contains(index) ? panes[index] : panes[0]
      if !pane.cwd.isEmpty { return pane.cwd }
    }
    return nonEmptySessionText(tab.cwd) ?? NSHomeDirectory()
  }

  private func nonEmptySessionText(_ value: String?) -> String? {
    guard let value else { return nil }
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    return trimmed.isEmpty ? nil : trimmed
  }

  // MARK: Propagating Settings

  /// Apply a theme to the terminal. `dividerColor` is unused (no splits) but
  /// kept so existing call sites compile unchanged.
  func applyTheme(theme: TerminalTheme, dividerColor: NSColor? = nil) {
    currentTheme = theme
    for terminal in terminals {
      terminal.applyTheme(theme: theme)
    }
  }

  func applySettings(settings: TerminalSettings) {
    currentSettings = settings
    for terminal in terminals {
      terminal.configureTerminal(settings: settings, theme: currentTheme)
    }
  }

  // MARK: Helpers

  private func createTerminal() -> TerminalTab {
    let terminal = TerminalTab(frame: bounds)
    terminal.onFocused = { [weak self] _ in
      self?.activeTerminalIndex = 0
    }
    terminal.configureTerminal(settings: currentSettings, theme: currentTheme)
    return terminal
  }

  private func constrainChildToFill(_ child: NSView) {
    child.translatesAutoresizingMaskIntoConstraints = false
    NSLayoutConstraint.activate([
      child.topAnchor.constraint(equalTo: topAnchor),
      child.leadingAnchor.constraint(equalTo: leadingAnchor),
      child.trailingAnchor.constraint(equalTo: trailingAnchor),
      child.bottomAnchor.constraint(equalTo: bottomAnchor),
    ])
  }
}
