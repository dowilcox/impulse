#if canImport(Testing)
  import AppKit
  @testable import ImpulseApp
  import Testing

  struct KeybindingRuntimeTests {
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
    }

    @Test func noneUnbindsACommandEverywhere() {
      _ = NSApplication.shared
      let overrides = ["review_changes": Keybindings.unbound]
      #expect(Keybindings.getKeybinding(id: "review_changes", overrides: overrides)?.keyEquivalent == "")
      #expect(Keybindings.shortcutDisplay(forId: "review_changes", overrides: overrides) == nil)
      let menu = MenuBuilder.buildMainMenu(overrides: overrides)
      let view = menu.items.first { $0.submenu?.title == "View" }?.submenu
      #expect(view?.item(withTitle: "Review Changes")?.keyEquivalent == "")
    }

    @Test func conflictsListSharedShortcuts() {
      let none = Keybindings.conflicts(overrides: [:])
      #expect(none.isEmpty, "built-in defaults don't collide: \(none)")
      let clash = Keybindings.conflicts(overrides: ["split_down": "Cmd+D"])
      #expect(Set(clash["⌘D"] ?? []) == ["split_right", "split_down"])
      let custom = Keybindings.conflicts(overrides: [:], extra: [("custom:Deploy", "Cmd+T")])
      #expect(Set(custom["⌘T"] ?? []) == ["new_tab", "custom:Deploy"])
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