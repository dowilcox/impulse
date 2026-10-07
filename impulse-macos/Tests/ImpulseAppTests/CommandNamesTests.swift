#if canImport(Testing)
  import AppKit
  @testable import ImpulseApp
  import ImpulseKit
  import Testing

  /// A command has one name: the menu bar, the Keyboard Shortcuts tab and
  /// the command palette all use the menu's title. On the main actor:
  /// building the menu bar sets NSApp's menus.
  @MainActor
  struct CommandNamesTests {
    private func menuTitles(_ menu: NSMenu) -> Set<String> {
      var titles: Set<String> = []
      for item in menu.items {
        titles.insert(item.title)
        if let submenu = item.submenu { titles.formUnion(menuTitles(submenu)) }
      }
      return titles
    }

    @Test func everyShortcutIsNamedAsInTheMenu() {
      _ = NSApplication.shared
      let titles = menuTitles(MenuBuilder.buildMainMenu())
      let missing = Keybindings.builtins.filter { !titles.contains($0.description) }.map(\.id)
      #expect(missing.isEmpty, "Keyboard Shortcuts names not in the menu bar: \(missing)")
    }

    @Test func paletteBuiltinsUseTheShortcutNames() {
      for item in CommandPalette.builtinItems() {
        guard let binding = Keybindings.builtins.first(where: { $0.id == item.id }) else { continue }
        #expect(item.title == binding.description, "\(item.id)")
      }
    }

    @Test func menuTitlesUseTheEllipsisCharacter() {
      _ = NSApplication.shared
      let dotted = menuTitles(MenuBuilder.buildMainMenu()).filter { $0.contains("...") }
      #expect(dotted.isEmpty, "\(dotted)")
    }

    @Test func helpOpensTheUserGuide() {
      _ = NSApplication.shared
      let help = MenuBuilder.buildMainMenu().items.last?.submenu?.item(withTitle: "Impulse Help")
      #expect(help?.action == #selector(MenuActions.menuShowHelp(_:)))
      #expect(MenuActions.helpURL.absoluteString.hasSuffix("/docs/README.md"))
    }
  }
#endif
