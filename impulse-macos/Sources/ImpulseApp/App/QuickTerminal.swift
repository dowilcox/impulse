import AppKit
import Carbon.HIToolbox

/// A terminal that drops down from the top of the screen on a global
/// hotkey, over any app and full-screen space, and hides again on the same
/// key or when you click away. Off until enabled in Settings.
final class QuickTerminal {
  static let shared = QuickTerminal()

  private var panel: QuickTerminalPanel?
  private var container: TerminalContainer?
  private var hotKey: EventHotKeyRef?
  private var handler: EventHandlerRef?
  private var registered: (enabled: Bool, shortcut: String)?
  private var resignObserver: Any?

  /// Register or drop the hotkey for the current settings.
  func configure(enabled: Bool, shortcut: String) {
    guard registered.map({ $0.enabled != enabled || $0.shortcut != shortcut }) ?? true else { return }
    registered = (enabled, shortcut)
    if let hotKey { UnregisterEventHotKey(hotKey) }
    hotKey = nil
    guard enabled, let (keyCode, modifiers) = Self.carbonKey(for: shortcut) else {
      if !enabled { hide() }
      return
    }
    installHandlerOnce()
    var ref: EventHotKeyRef?
    let id = EventHotKeyID(signature: OSType(0x494D_5051), id: 1)  // "IMPQ"
    if RegisterEventHotKey(keyCode, modifiers, id, GetApplicationEventTarget(), 0, &ref) == noErr {
      hotKey = ref
    } else {
      NSLog("QuickTerminal: couldn't register the hotkey %@", shortcut)
    }
  }

  func toggle() {
    if let panel, panel.isVisible, panel.isKeyWindow {
      hide()
    } else {
      show()
    }
  }

  func applyTheme(_ theme: Theme) {
    container?.applyTheme(theme: Self.terminalTheme(theme), dividerColor: theme.bgHighlightColor)
    panel?.backgroundColor = theme.bgColor
  }

  // MARK: Showing

  private func show() {
    let screen = NSScreen.screens.first { NSMouseInRect(NSEvent.mouseLocation, $0.frame, false) } ?? NSScreen.main
    guard let screen else { return }
    let visible = screen.visibleFrame
    let height = (visible.height * 0.42).rounded()
    let frame = NSRect(x: visible.minX, y: visible.maxY - height, width: visible.width, height: height)

    let panel = self.panel ?? makePanel()
    if container == nil {
      let delegate = NSApp.delegate as? AppDelegate
      let theme = delegate?.theme ?? ThemeManager.theme(forName: "nord")
      var settings = SettingsStore.shared.settings.terminalSettings(directory: Self.directory())
      // No input bar here: type at the shell's own prompt.
      settings.terminalContextBar = false
      let container = TerminalContainer(
        frame: NSRect(origin: .zero, size: frame.size), settings: settings, theme: Self.terminalTheme(theme))
      container.applyTheme(theme: Self.terminalTheme(theme), dividerColor: theme.bgHighlightColor)
      container.autoresizingMask = [.width, .height]
      panel.contentView = container
      panel.backgroundColor = theme.bgColor
      self.container = container
    }

    let reduceMotion = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
    panel.setFrame(reduceMotion ? frame : frame.offsetBy(dx: 0, dy: height), display: false)
    panel.alphaValue = reduceMotion ? 1 : 0
    panel.makeKeyAndOrderFront(nil)
    if !reduceMotion {
      NSAnimationContext.runAnimationGroup { context in
        context.duration = 0.16
        context.timingFunction = CAMediaTimingFunction(name: .easeOut)
        panel.animator().setFrame(frame, display: true)
        panel.animator().alphaValue = 1
      }
    }
    container?.activeTerminal?.focus()
  }

  private func hide() {
    guard let panel, panel.isVisible else { return }
    panel.orderOut(nil)
  }

  private func makePanel() -> QuickTerminalPanel {
    let panel = QuickTerminalPanel(
      contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
    panel.level = .statusBar
    panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient]
    panel.isFloatingPanel = true
    panel.hidesOnDeactivate = false
    panel.hasShadow = true
    panel.isReleasedWhenClosed = false
    // Click away and it tucks itself back up.
    resignObserver = NotificationCenter.default.addObserver(
      forName: NSWindow.didResignKeyNotification, object: panel, queue: .main
    ) { [weak self] _ in self?.hide() }
    self.panel = panel
    return panel
  }

  /// The front window's current folder (its workspace, or the folder the
  /// file tree follows), else home.
  private static func directory() -> String {
    let controller = NSApp.orderedWindows.lazy.compactMap { $0.windowController as? MainWindowController }.first
    let root = controller?.fileTreeRootPath
    return root?.isEmpty == false ? root! : NSHomeDirectory()
  }

  private static func terminalTheme(_ theme: Theme) -> TerminalTheme {
    TerminalTheme(
      bg: theme.terminalBg, fg: theme.terminalFg, selection: theme.selection, cursor: theme.cursor,
      border: theme.border, fgMuted: theme.fgMuted, accent: theme.accent, red: theme.red,
      terminalPalette: theme.terminalPalette)
  }

  // MARK: Hotkey

  private func installHandlerOnce() {
    guard handler == nil else { return }
    var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
    InstallEventHandler(
      GetApplicationEventTarget(),
      { _, _, _ in
        DispatchQueue.main.async { QuickTerminal.shared.toggle() }
        return noErr
      }, 1, &spec, nil, &handler)
  }

  /// "Ctrl+`" → Carbon key code and modifiers (US key positions).
  static func carbonKey(for shortcut: String) -> (UInt32, UInt32)? {
    let parsed = Keybindings.parseShortcut(shortcut)
    guard !parsed.modifierFlags.intersection([.command, .control, .option]).isEmpty else { return nil }
    let codes: [String: Int] = [
      "a": kVK_ANSI_A, "b": kVK_ANSI_B, "c": kVK_ANSI_C, "d": kVK_ANSI_D, "e": kVK_ANSI_E, "f": kVK_ANSI_F,
      "g": kVK_ANSI_G, "h": kVK_ANSI_H, "i": kVK_ANSI_I, "j": kVK_ANSI_J, "k": kVK_ANSI_K, "l": kVK_ANSI_L,
      "m": kVK_ANSI_M, "n": kVK_ANSI_N, "o": kVK_ANSI_O, "p": kVK_ANSI_P, "q": kVK_ANSI_Q, "r": kVK_ANSI_R,
      "s": kVK_ANSI_S, "t": kVK_ANSI_T, "u": kVK_ANSI_U, "v": kVK_ANSI_V, "w": kVK_ANSI_W, "x": kVK_ANSI_X,
      "y": kVK_ANSI_Y, "z": kVK_ANSI_Z, "0": kVK_ANSI_0, "1": kVK_ANSI_1, "2": kVK_ANSI_2, "3": kVK_ANSI_3,
      "4": kVK_ANSI_4, "5": kVK_ANSI_5, "6": kVK_ANSI_6, "7": kVK_ANSI_7, "8": kVK_ANSI_8, "9": kVK_ANSI_9,
      "`": kVK_ANSI_Grave, "-": kVK_ANSI_Minus, "=": kVK_ANSI_Equal, "[": kVK_ANSI_LeftBracket,
      "]": kVK_ANSI_RightBracket, "\\": kVK_ANSI_Backslash, ";": kVK_ANSI_Semicolon, "'": kVK_ANSI_Quote,
      ",": kVK_ANSI_Comma, ".": kVK_ANSI_Period, "/": kVK_ANSI_Slash, " ": kVK_Space, "\t": kVK_Tab,
      "\r": kVK_Return,
    ]
    guard let code = codes[parsed.keyEquivalent.lowercased()] else { return nil }
    var modifiers = 0
    if parsed.modifierFlags.contains(.command) { modifiers |= cmdKey }
    if parsed.modifierFlags.contains(.control) { modifiers |= controlKey }
    if parsed.modifierFlags.contains(.option) { modifiers |= optionKey }
    if parsed.modifierFlags.contains(.shift) { modifiers |= shiftKey }
    return (UInt32(code), UInt32(modifiers))
  }
}

/// Borderless, but it takes keys (and doesn't drag the whole app forward).
final class QuickTerminalPanel: NSPanel {
  override var canBecomeKey: Bool { true }
  override var canBecomeMain: Bool { false }
}
