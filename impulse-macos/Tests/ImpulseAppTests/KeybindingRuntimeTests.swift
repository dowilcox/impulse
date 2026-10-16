#if canImport(Testing)
  import AppKit
  @testable import ImpulseApp
  import Testing

  struct KeybindingRuntimeTests {
    // Menu tests run on the main actor: building the menu bar sets NSApp's
    // Services, Window and Help menus, which isn't safe from parallel threads.
    @MainActor
    @Test func builtinOverrideChangesMenuShortcut() {
      _ = NSApplication.shared

      let menu = MenuBuilder.buildMainMenu(overrides: ["paste": "Cmd+Shift+V"])
      let editMenu = menu.items.first { $0.submenu?.title == "Edit" }?.submenu
      let pasteItem = editMenu?.item(withTitle: "Paste")

      #expect(pasteItem?.keyEquivalent.lowercased() == "v")
      #expect(
        pasteItem?.keyEquivalentModifierMask.intersection([.command, .control, .option, .shift])
          == [.command, .shift]
      )
    }

    @Test func terminalSettingsCarryKeybindingOverrides() {
      var settings = Settings.default
      settings.keybindingOverrides = ["copy": "Cmd+Shift+C"]

      let terminalSettings = settings.terminalSettings()

      #expect(terminalSettings.keybindingOverrides["copy"] == "Cmd+Shift+C")
    }

    /// A key event; `option` says which Option key is down, the way AppKit
    /// reports it (device-dependent bits beside `.option`).
    private func key(
      _ characters: String, ignoringModifiers: String, keyCode: UInt16, flags: NSEvent.ModifierFlags = [],
      option: KeyEncoder.OptionAsMeta = .off
    ) -> NSEvent {
      var modifiers = flags
      switch option {
      case .left: modifiers.formUnion(NSEvent.ModifierFlags(rawValue: NSEvent.ModifierFlags.option.rawValue | 0x20))
      case .right: modifiers.formUnion(NSEvent.ModifierFlags(rawValue: NSEvent.ModifierFlags.option.rawValue | 0x40))
      case .both, .off: break
      }
      return NSEvent.keyEvent(
        with: .keyDown, location: .zero, modifierFlags: modifiers, timestamp: 0, windowNumber: 0, context: nil,
        characters: characters, charactersIgnoringModifiers: ignoringModifiers, isARepeat: false, keyCode: keyCode)!
    }

    @Test func optionTypesItsCharacterUnlessItIsMeta() {
      let leftF = key("ƒ", ignoringModifiers: "f", keyCode: 3, option: .left)
      #expect(KeyEncoder.metaBytes(for: leftF, optionAsMeta: .off) == nil, "off: ƒ is typed")
      #expect(KeyEncoder.metaBytes(for: leftF, optionAsMeta: .left) == [0x1B, 0x66])
      #expect(KeyEncoder.metaBytes(for: leftF, optionAsMeta: .both) == [0x1B, 0x66])
      #expect(KeyEncoder.metaBytes(for: leftF, optionAsMeta: .right) == nil, "the other Option key")
      let rightF = key("ƒ", ignoringModifiers: "f", keyCode: 3, option: .right)
      #expect(KeyEncoder.metaBytes(for: rightF, optionAsMeta: .right) == [0x1B, 0x66])
      #expect(KeyEncoder.metaBytes(for: rightF, optionAsMeta: .left) == nil)
      // A German layout's ⌥L types @: as typed unless Option is Meta.
      let at = key("@", ignoringModifiers: "l", keyCode: 37, option: .left)
      #expect(KeyEncoder.metaBytes(for: at, optionAsMeta: .off) == nil)
      #expect(KeyEncoder.metaBytes(for: at, optionAsMeta: .left) == [0x1B, 0x6C])
      // Shift and Control still apply.
      let shifted = key("Ï", ignoringModifiers: "F", keyCode: 3, flags: .shift, option: .left)
      #expect(KeyEncoder.metaBytes(for: shifted, optionAsMeta: .left) == [0x1B, 0x46])
      let control = key("\u{6}", ignoringModifiers: "f", keyCode: 3, flags: .control, option: .left)
      #expect(KeyEncoder.metaBytes(for: control, optionAsMeta: .left) == [0x1B, 0x06])
      // Keys that type nothing go through encodeForDoCommand.
      let left = key("\u{F702}", ignoringModifiers: "\u{F702}", keyCode: 123, option: .left)
      #expect(KeyEncoder.metaBytes(for: left, optionAsMeta: .both) == nil)
    }

    @Test func optionDeleteAndShiftReturn() {
      let optionDelete = key("\u{7F}", ignoringModifiers: "\u{7F}", keyCode: 51, option: .left)
      #expect(KeyEncoder.encode(event: optionDelete, appCursor: false, appKeypad: false) == [0x7F])
      #expect(
        KeyEncoder.encode(event: optionDelete, appCursor: false, appKeypad: false, optionAsMeta: .left)
          == [0x1B, 0x7F])
      let optionReturn = key("\r", ignoringModifiers: "\r", keyCode: 36, option: .left)
      #expect(
        KeyEncoder.encode(event: optionReturn, appCursor: false, appKeypad: false, optionAsMeta: .both)
          == [0x1B, 0x0D])
      // Shells read the kitty form as text; programs that want it ask for it.
      let shiftReturn = key("\r", ignoringModifiers: "\r", keyCode: 36, flags: .shift)
      #expect(KeyEncoder.encode(event: shiftReturn, appCursor: false, appKeypad: false) == [0x0D])
      let controlC = key("\u{3}", ignoringModifiers: "c", keyCode: 8, flags: .control)
      #expect(KeyEncoder.encode(event: controlC, appCursor: false, appKeypad: false) == [0x03])
      // Option arrows keep their modifier whatever the setting.
      let optionLeft = key("\u{F702}", ignoringModifiers: "\u{F702}", keyCode: 123, option: .left)
      #expect(
        KeyEncoder.encode(event: optionLeft, appCursor: false, appKeypad: false)
          == Array("\u{1B}[1;3D".utf8))
    }

    @Test func kittyAltIsOptionOnlyWhenItIsMeta() {
      let optionF = key("ƒ", ignoringModifiers: "f", keyCode: 3, option: .left)
      #expect(KeyEncoder.kittyKey(for: optionF, optionAsMeta: .off) == nil, "typed as text")
      let meta = KeyEncoder.kittyKey(for: optionF, optionAsMeta: .left)
      #expect(meta?.modifiers == [.alt])
      #expect(meta?.text == nil)
      if case .text = meta?.key {} else { Issue.record("expected a text key, got \(String(describing: meta?.key))") }
      // Beside ⌃, Option types nothing: ⌃⌥X is C-M-x whatever the setting.
      let controlOption = key("\u{18}", ignoringModifiers: "x", keyCode: 7, flags: .control, option: .left)
      #expect(KeyEncoder.kittyKey(for: controlOption, optionAsMeta: .off)?.modifiers == [.control, .alt])
      #expect(KeyEncoder.kittyKey(for: controlOption, optionAsMeta: .left)?.modifiers == [.control, .alt])
      #expect(KeyEncoder.kittyKey(for: controlOption, optionAsMeta: .off)?.text == nil)
      let optionLeft = key("\u{F702}", ignoringModifiers: "\u{F702}", keyCode: 123, option: .left)
      #expect(KeyEncoder.kittyKey(for: optionLeft, optionAsMeta: .off)?.modifiers == [.alt])
    }

    @Test func optionAsMetaSettingParsesAndValidates() {
      #expect(KeyEncoder.OptionAsMeta(setting: "Left") == .left)
      #expect(KeyEncoder.OptionAsMeta(setting: "nonsense") == .off)
      var settings = Settings.default
      #expect(settings.terminalOptionAsMeta == "off")
      settings.terminalOptionAsMeta = "BOTH"
      settings.validate()
      #expect(settings.terminalOptionAsMeta == "both")
      #expect(settings.terminalSettings().terminalOptionAsMeta == "both")
    }

    @Test func recordedKeysBecomeShortcutStrings() {
      #expect(
        Keybindings.shortcutString(keyCode: 40, characters: "k", modifiers: [.command, .shift]) == "Shift+Cmd+K")
      #expect(
        Keybindings.shortcutString(keyCode: 123, characters: nil, modifiers: [.control, .command])
          == "Ctrl+Cmd+Left")
      #expect(Keybindings.shortcutString(keyCode: 49, characters: " ", modifiers: [.command]) == "Cmd+Space")
      #expect(Keybindings.shortcutString(keyCode: 24, characters: "=", modifiers: [.command]) == "Cmd+=")
      #expect(Keybindings.shortcutString(keyCode: 56, characters: "", modifiers: [.shift]) == nil, "a lone modifier")
      // Round trip through the parser.
      let parsed = Keybindings.parseShortcut("Shift+Cmd+K")
      #expect(parsed.keyEquivalent == "k")
      #expect(parsed.modifierFlags == [.command, .shift])
      // The + key: recorded as "Shift+Cmd++".
      let plus = Keybindings.parseShortcut("Shift+Cmd++")
      #expect(plus.keyEquivalent == "+")
      #expect(plus.modifierFlags == [.command, .shift])
    }

    @MainActor
    @Test func noneUnbindsACommandEverywhere() {
      _ = NSApplication.shared
      let overrides = ["review_changes": Keybindings.unbound]
      #expect(Keybindings.getKeybinding(id: "review_changes", overrides: overrides)?.keyEquivalent == "")
      #expect(Keybindings.shortcutDisplay(forId: "review_changes", overrides: overrides) == nil)
      let menu = MenuBuilder.buildMainMenu(overrides: overrides)
      let git = menu.items.first { $0.submenu?.title == "Git" }?.submenu
      #expect(git?.item(withTitle: "Review Changes") != nil)
      #expect(git?.item(withTitle: "Review Changes")?.keyEquivalent == "")
    }

    @Test func conflictsListSharedShortcuts() {
      let none = Keybindings.conflicts(overrides: [:])
      #expect(none.isEmpty, "built-in defaults don't collide: \(none)")
      let clash = Keybindings.conflicts(overrides: ["split_down": "Cmd+D"])
      #expect(Set(clash["⌘D"] ?? []) == ["split_right", "split_down"])
      let custom = Keybindings.conflicts(overrides: [:], extra: [("custom:Deploy", "Cmd+T")])
      #expect(Set(custom["⌘T"] ?? []) == ["new_tab", "custom:Deploy"])
    }

    @Test func quickTerminalShortcutsMapToCarbonKeys() {
      let grave = QuickTerminal.carbonKey(for: "Ctrl+`")
      #expect(grave?.0 == 50)  // kVK_ANSI_Grave
      #expect(grave?.1 == 4096)  // controlKey
      let space = QuickTerminal.carbonKey(for: "Alt+Space")
      #expect(space?.0 == 49)
      #expect(space?.1 == 2048)  // optionKey
      #expect(QuickTerminal.carbonKey(for: "Shift+K") == nil, "needs ⌘, ⌃ or ⌥ to be global")
      #expect(QuickTerminal.carbonKey(for: "Ctrl+F13") == nil, "unknown keys are refused")
    }

    /// Recording a shortcut takes the keyboard from a text field, so its
    /// keys (⌘V) stop being typing; otherwise the tab doesn't take it.
    @MainActor
    @Test func recordingAShortcutTakesTheKeyboard() {
      _ = NSApplication.shared
      let surface = KeybindingsSurface(palette: ChromePalette(theme: ThemeManager.theme(forName: "nord")))
      defer { surface.cleanupTool() }
      let window = NSWindow(
        contentRect: NSRect(x: 0, y: 0, width: 900, height: 600), styleMask: [.titled], backing: .buffered,
        defer: true)
      let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 100, height: 20))
      let content = NSView(frame: window.contentLayoutRect)
      content.addSubview(surface)
      content.addSubview(field)
      window.contentView = content
      window.makeFirstResponder(field)
      #expect(!surface.acceptsFirstResponder)
      surface.model.recording = "copy"
      #expect(window.firstResponder === surface)
      surface.model.recording = nil
      #expect(!surface.acceptsFirstResponder)
    }
  }
#elseif canImport(XCTest)
  import AppKit
  @testable import ImpulseApp
  import XCTest

  final class KeybindingRuntimeTests: XCTestCase {
    func testBuiltinOverrideChangesMenuShortcut() {
      _ = NSApplication.shared

      let menu = MenuBuilder.buildMainMenu(overrides: ["paste": "Cmd+Shift+V"])
      let editMenu = menu.items.first { $0.submenu?.title == "Edit" }?.submenu
      let pasteItem = editMenu?.item(withTitle: "Paste")

      XCTAssertEqual(pasteItem?.keyEquivalent.lowercased(), "v")
      XCTAssertEqual(
        pasteItem?.keyEquivalentModifierMask
          .intersection([.command, .control, .option, .shift]),
        [.command, .shift]
      )
    }

    func testTerminalSettingsCarryKeybindingOverrides() {
      var settings = Settings.default
      settings.keybindingOverrides = ["copy": "Cmd+Shift+C"]

      let terminalSettings = settings.terminalSettings()

      XCTAssertEqual(terminalSettings.keybindingOverrides["copy"], "Cmd+Shift+C")
    }

    func testOptionSendsMetaOnlyWhenTheSettingSaysSo() {
      guard
        let event = NSEvent.keyEvent(
          with: .keyDown,
          location: .zero,
          modifierFlags: [.option],
          timestamp: 0,
          windowNumber: 0,
          context: nil,
          characters: "ƒ",
          charactersIgnoringModifiers: "f",
          isARepeat: false,
          keyCode: 3
        )
      else {
        return XCTFail("Expected NSEvent.keyEvent to create a key event")
      }

      XCTAssertNil(KeyEncoder.metaBytes(for: event, optionAsMeta: .off))
      XCTAssertEqual(KeyEncoder.metaBytes(for: event, optionAsMeta: .both), [0x1B, 0x66])
    }
  }
#endif