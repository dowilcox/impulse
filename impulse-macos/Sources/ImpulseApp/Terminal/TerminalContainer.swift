import AppKit

/// A terminal surface: one `TerminalTab` (grid) above a slot for the
/// window's terminal input bar. The bar lives in whichever terminal has
/// focus; the others keep its space with a quiet stand-in so moving focus
/// between split panes never resizes their grids.
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

  /// Height of the input bar the last time it was measured, shared so new
  /// panes reserve the same space.
  private static var lastInputBarHeight: CGFloat = 0
  private let accessorySlot = AccessorySlot()
  private let placeholder = InputBarPlaceholder()
  private var reservedHeight: NSLayoutConstraint?
  private var interactionObserver: NSObjectProtocol?
  private var heightObserver: NSObjectProtocol?

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
    accessorySlot.translatesAutoresizingMaskIntoConstraints = false
    placeholder.translatesAutoresizingMaskIntoConstraints = false
    addSubview(accessorySlot)
    accessorySlot.addSubview(placeholder)
    // Collapses to zero when the slot is empty and nothing is reserved.
    let collapse = accessorySlot.heightAnchor.constraint(equalToConstant: 0)
    collapse.priority = NSLayoutConstraint.Priority(1)
    let reserved = accessorySlot.heightAnchor.constraint(equalToConstant: 0)
    reservedHeight = reserved
    NSLayoutConstraint.activate([
      child.topAnchor.constraint(equalTo: topAnchor),
      child.leadingAnchor.constraint(equalTo: leadingAnchor),
      child.trailingAnchor.constraint(equalTo: trailingAnchor),
      child.bottomAnchor.constraint(equalTo: accessorySlot.topAnchor),
      accessorySlot.leadingAnchor.constraint(equalTo: leadingAnchor),
      accessorySlot.trailingAnchor.constraint(equalTo: trailingAnchor),
      accessorySlot.bottomAnchor.constraint(equalTo: bottomAnchor),
      collapse,
      placeholder.topAnchor.constraint(equalTo: accessorySlot.topAnchor),
      placeholder.leadingAnchor.constraint(equalTo: accessorySlot.leadingAnchor),
      placeholder.trailingAnchor.constraint(equalTo: accessorySlot.trailingAnchor),
      placeholder.bottomAnchor.constraint(equalTo: accessorySlot.bottomAnchor),
    ])
    accessorySlot.onWillRemove = { [weak self] view in
      guard let self, view !== self.placeholder else { return }
      let height = self.accessorySlot.bounds.height
      if height > 0 { Self.lastInputBarHeight = height }
      // Let the removal finish before reserving space again.
      DispatchQueue.main.async { self.updatePlaceholder() }
    }
    interactionObserver = NotificationCenter.default.addObserver(
      forName: .terminalInteractionModeChanged, object: child, queue: .main
    ) { [weak self] _ in
      self?.updatePlaceholder()
    }
    heightObserver = NotificationCenter.default.addObserver(
      forName: Self.inputBarHeightChanged, object: nil, queue: .main
    ) { [weak self] _ in
      self?.updatePlaceholder()
    }
    accessorySlot.onLayout = { [weak self] height in
      guard let self, self.hasAccessory, height > 0, height != Self.lastInputBarHeight else {
        return
      }
      Self.lastInputBarHeight = height
      NotificationCenter.default.post(name: Self.inputBarHeightChanged, object: nil)
    }
    updatePlaceholder()
  }

  deinit {
    if let interactionObserver { NotificationCenter.default.removeObserver(interactionObserver) }
    if let heightObserver { NotificationCenter.default.removeObserver(heightObserver) }
  }

  /// The live bar's height changed (first layout, font change): unfocused
  /// terminals resize their stand-ins to match.
  private static let inputBarHeightChanged = Notification.Name("impulse.inputBarHeightChanged")

  // MARK: Input bar

  /// Whether the window's input bar is currently in this terminal.
  var hasAccessory: Bool { accessorySlot.subviews.contains { $0 !== placeholder } }

  /// Move the window's input bar into this terminal.
  func attachAccessory(_ view: NSView) {
    guard view.superview !== accessorySlot else { return }
    view.removeFromSuperview()
    view.translatesAutoresizingMaskIntoConstraints = false
    // The bar's own height decides the slot's.
    view.setContentCompressionResistancePriority(.required, for: .vertical)
    view.setContentHuggingPriority(.required, for: .vertical)
    accessorySlot.addSubview(view)
    NSLayoutConstraint.activate([
      view.topAnchor.constraint(equalTo: accessorySlot.topAnchor),
      view.leadingAnchor.constraint(equalTo: accessorySlot.leadingAnchor),
      view.trailingAnchor.constraint(equalTo: accessorySlot.trailingAnchor),
      view.bottomAnchor.constraint(equalTo: accessorySlot.bottomAnchor),
    ])
    updatePlaceholder()
  }

  func setInputBarColors(background: NSColor, border: NSColor) {
    placeholder.background = background
    placeholder.border = border
  }

  /// Reserve the bar's space while it's elsewhere — unless a TUI owns this
  /// grid or the bar is turned off, in which case the grid takes it all.
  private func updatePlaceholder() {
    let reserve = !hasAccessory && !(activeTerminal?.wantsGridFocus ?? true)
      && Self.lastInputBarHeight > 0
    placeholder.isHidden = !reserve
    reservedHeight?.constant = Self.lastInputBarHeight
    reservedHeight?.isActive = reserve
  }
}

/// Reports subviews leaving, so the container notices the input bar moving
/// to another pane.
private final class AccessorySlot: NSView {
  var onWillRemove: ((NSView) -> Void)?
  var onLayout: ((CGFloat) -> Void)?

  override func layout() {
    super.layout()
    onLayout?(bounds.height)
  }

  override func willRemoveSubview(_ subview: NSView) {
    super.willRemoveSubview(subview)
    onWillRemove?(subview)
  }
}

/// The stand-in drawn where the input bar sits in an unfocused terminal: the
/// bar's background and top border, with a faint prompt mark.
private final class InputBarPlaceholder: NSView {
  var background: NSColor = .windowBackgroundColor { didSet { needsDisplay = true } }
  var border: NSColor = .separatorColor { didSet { needsDisplay = true } }

  override var isFlipped: Bool { true }

  override func draw(_ dirtyRect: NSRect) {
    background.setFill()
    bounds.fill()
    border.setFill()
    NSRect(x: 0, y: 0, width: bounds.width, height: 1).fill()
    let attributes: [NSAttributedString.Key: Any] = [
      .font: NSFont.monospacedSystemFont(ofSize: 13, weight: .regular),
      .foregroundColor: border.blended(withFraction: 0.35, of: .gray) ?? border,
    ]
    // Where the live bar's prompt chevron sits (input row, bottom half).
    let mark = NSAttributedString(string: "›", attributes: attributes)
    let size = mark.size()
    mark.draw(at: NSPoint(x: 26, y: bounds.height - 25 - size.height / 2))
  }
}
