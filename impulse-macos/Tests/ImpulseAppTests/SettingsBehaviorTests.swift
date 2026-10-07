#if canImport(Testing)
  import AppKit
  import Foundation
  @testable import ImpulseApp
  import Testing

  struct SettingsValidationTests {
    @Test func numbersKeepToTheSettingsTabRanges() {
      var settings = Settings.default
      settings.editorCursorSurroundingLines = -5
      settings.gitAutoFetchMinutes = 999
      settings.terminalLongCommandSeconds = 50_000
      settings.editorLineHeight = 80
      settings.terminalMinimumContrast = 40
      settings.fontSize = 2
      settings.validate()
      #expect(settings.editorCursorSurroundingLines == 0)
      #expect(settings.gitAutoFetchMinutes == 120)
      #expect(settings.terminalLongCommandSeconds == 50_000)
      #expect(settings.editorLineHeight == 80)
      #expect(settings.terminalMinimumContrast == 21)
      #expect(settings.fontSize == 6)
    }

    @Test func defaultsAreInRange() {
      var settings = Settings.default
      settings.validate()
      for item in SettingsCatalog.items {
        #expect(!item.isModified(settings), "\(item.key)")
      }
    }
  }

  struct CustomShortcutTests {
    @Test func commandLineKeepsShellSyntaxAndQuotesExtraArguments() {
      #expect(CustomKeybinding(command: "npm run lint && npm test").commandLine == "npm run lint && npm test")
      #expect(CustomKeybinding(command: "npm", args: ["test"]).commandLine == "npm test")
      #expect(CustomKeybinding(command: "echo", args: ["a b", "it's"]).commandLine == "echo 'a b' 'it'\\''s'")
      #expect(CustomKeybinding(command: "  ").commandLine == "")
    }

    @Test func argumentsMayBeLeftOut() throws {
      let json = #"[{"name": "Tests", "key": "Ctrl+Alt+T", "command": "npm test | tee out.log"}]"#
      let decoded = try JSONDecoder().decode([CustomKeybinding].self, from: Data(json.utf8))
      #expect(decoded.first?.commandLine == "npm test | tee out.log")
    }
  }

  struct FixedShortcutTests {
    private func conflicts(_ overrides: [String: String]) -> [String: [String]] {
      Keybindings.conflicts(
        overrides: overrides,
        extra: Keybindings.fixedShortcuts.enumerated().map { ("fixed:\($0.offset)", $0.element.shortcut) })
    }

    private func items(_ menu: NSMenu) -> [NSMenuItem] {
      menu.items.flatMap { [$0] + ($0.submenu.map(items) ?? []) }
    }

    /// A menu item's shortcut as the Keyboard Shortcuts tab writes it (an
    /// uppercase key or "?" implies ⇧).
    private func symbol(_ item: NSMenuItem) -> String {
      var modifiers = item.keyEquivalentModifierMask.intersection([.command, .control, .option, .shift])
      if item.keyEquivalent != item.keyEquivalent.lowercased() || item.keyEquivalent == "?" {
        modifiers.insert(.shift)
      }
      return Keybindings.modifierSymbols(modifiers) + Keybindings.keySymbol(item.keyEquivalent)
    }

    @MainActor  // building the menu bar sets NSApp's menus
    @Test func listMatchesTheMenuBar() throws {
      _ = NSApplication.shared
      let menuItems = items(MenuBuilder.buildMainMenu()).filter { !$0.keyEquivalent.isEmpty }
      for fixed in Keybindings.fixedShortcuts {
        let item = menuItems.first { $0.title.replacingOccurrences(of: "...", with: "…") == fixed.name }
        let shown = try #require(item, "no menu item \(fixed.name)")
        #expect(symbol(shown) == Keybindings.symbolDisplay(shortcut: fixed.shortcut), "\(fixed.name)")
      }
    }

    @Test func defaultShortcutsAvoidThem() {
      let clashes = conflicts([:]).filter { $0.value.contains { $0.hasPrefix("fixed:") } }
      #expect(clashes.isEmpty, "\(clashes)")
    }

    @Test func aCommandOnAFixedShortcutIsFlagged() {
      let shared = conflicts(["split_right": "Cmd+Q"])["⌘Q"] ?? []
      #expect(shared.contains("split_right"))
      #expect(shared.contains { $0.hasPrefix("fixed:") })
    }
  }

  struct EditorKeyTests {
    private func key(_ characters: String, _ modifiers: NSEvent.ModifierFlags) -> NSEvent {
      NSEvent.keyEvent(
        with: .keyDown, location: .zero, modifierFlags: modifiers, timestamp: 0, windowNumber: 0, context: nil,
        characters: characters, charactersIgnoringModifiers: characters, isARepeat: false, keyCode: 0)!
    }

    @Test func impulseShortcutsWinOverMonacos() {
      #expect(EditorWebView.appCommandWins(key("d", [.command])))  // Split Right, not add next occurrence
      #expect(EditorWebView.appCommandWins(key("g", [.command])))  // Go to Line…, not find next
      #expect(EditorWebView.appCommandWins(key("G", [.command, .shift])))  // Review Changes
      #expect(EditorWebView.appCommandWins(key("\r", [.command, .shift])))  // Zoom Pane
    }

    @Test func editorKeepsItsOwnKeysAndTerminalOnes() {
      #expect(!EditorWebView.appCommandWins(key("c", [.command])))
      #expect(!EditorWebView.appCommandWins(key("f", [.command])))
      #expect(!EditorWebView.appCommandWins(key("s", [.command])))
      #expect(!EditorWebView.appCommandWins(key("K", [.command, .shift])))  // Bookmark Block; Monaco deletes the line
      #expect(!EditorWebView.appCommandWins(key("i", [.command])))  // the agent composer; Monaco suggests
      #expect(!EditorWebView.appCommandWins(key("l", [.command])))  // not an Impulse shortcut
    }
  }
#endif
