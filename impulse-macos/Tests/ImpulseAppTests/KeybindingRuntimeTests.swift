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

    @Test func metaInsertTextEncodingUsesEscapePrefixForOptionAscii() {
      let event = NSEvent.keyEvent(
        with: .keyDown,
        location: .zero,
        modifierFlags: [.option],
        timestamp: 0,
        windowNumber: 0,
        context: nil,
        characters: "f",
        charactersIgnoringModifiers: "f",
        isARepeat: false,
        keyCode: 3
      )

      #expect(KeyEncoder.encodeMetaForInsertText(text: "f", event: event) == Data([0x1B, 0x66]))
    }

    @Test func metaInsertTextLeavesComposedTextUnchanged() {
      let event = NSEvent.keyEvent(
        with: .keyDown,
        location: .zero,
        modifierFlags: [.option],
        timestamp: 0,
        windowNumber: 0,
        context: nil,
        characters: "é",
        charactersIgnoringModifiers: "e",
        isARepeat: false,
        keyCode: 14
      )

      #expect(KeyEncoder.encodeMetaForInsertText(text: "é", event: event) == nil)
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

    func testMetaInsertTextEncodingUsesEscapePrefixForOptionAscii() {
      guard
        let event = NSEvent.keyEvent(
          with: .keyDown,
          location: .zero,
          modifierFlags: [.option],
          timestamp: 0,
          windowNumber: 0,
          context: nil,
          characters: "f",
          charactersIgnoringModifiers: "f",
          isARepeat: false,
          keyCode: 3
        )
      else {
        return XCTFail("Expected NSEvent.keyEvent to create a key event")
      }

      XCTAssertEqual(
        KeyEncoder.encodeMetaForInsertText(text: "f", event: event), Data([0x1B, 0x66]))
    }

    func testMetaInsertTextLeavesComposedTextUnchanged() {
      guard
        let event = NSEvent.keyEvent(
          with: .keyDown,
          location: .zero,
          modifierFlags: [.option],
          timestamp: 0,
          windowNumber: 0,
          context: nil,
          characters: "é",
          charactersIgnoringModifiers: "e",
          isARepeat: false,
          keyCode: 14
        )
      else {
        return XCTFail("Expected NSEvent.keyEvent to create a key event")
      }

      XCTAssertNil(KeyEncoder.encodeMetaForInsertText(text: "é", event: event))
    }
  }
#endif