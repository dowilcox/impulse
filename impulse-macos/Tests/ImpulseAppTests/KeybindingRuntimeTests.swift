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
