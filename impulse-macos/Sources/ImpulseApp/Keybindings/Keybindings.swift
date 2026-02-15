import AppKit

// MARK: - Built-in Keybinding

/// A built-in keyboard shortcut with its default key equivalent and modifiers,
/// mirroring the Rust `BuiltinKeybinding` from the Linux frontend but using
/// macOS conventions (Cmd instead of Ctrl).
struct BuiltinKeybinding {
    let id: String
    let description: String
    let category: String
    /// Human-readable shortcut string, e.g. "Cmd+T".
    let defaultShortcut: String
    /// The key equivalent for NSMenuItem / NSEvent, e.g. "t".
    let keyEquivalent: String
    /// The modifier flags for the shortcut.
    let modifierFlags: NSEvent.ModifierFlags
}

// MARK: - Built-in Keybinding Registry

/// All built-in keybindings, using macOS conventions (Cmd instead of Ctrl).
enum Keybindings {

    static let builtins: [BuiltinKeybinding] = [
        // -- Tabs --
        BuiltinKeybinding(
            id: "new_tab",
            description: "New Terminal Tab",
            category: "Tabs",
            defaultShortcut: "Cmd+T",
            keyEquivalent: "t",
            modifierFlags: [.command]
        ),
        BuiltinKeybinding(
            id: "close_tab",
            description: "Close Tab",
            category: "Tabs",
            defaultShortcut: "Cmd+W",
            keyEquivalent: "w",
            modifierFlags: [.command]
        ),
        BuiltinKeybinding(
            id: "reopen_tab",
            description: "Reopen Closed Tab",
            category: "Tabs",
            defaultShortcut: "Cmd+Shift+T",
            keyEquivalent: "T",
            modifierFlags: [.command, .shift]
        ),
        BuiltinKeybinding(
            id: "next_tab",
            description: "Next Tab",
            category: "Tabs",
            defaultShortcut: "Ctrl+Tab",
            keyEquivalent: "\t",
            modifierFlags: [.control]
        ),
        BuiltinKeybinding(
            id: "prev_tab",
            description: "Previous Tab",
            category: "Tabs",
            defaultShortcut: "Ctrl+Shift+Tab",
            keyEquivalent: "\u{0019}", // backtab
            modifierFlags: [.control, .shift]
        ),
        // -- Terminal --
        BuiltinKeybinding(
            id: "copy",
            description: "Copy",
            category: "Terminal",
            defaultShortcut: "Cmd+C",
            keyEquivalent: "c",
            modifierFlags: [.command]
        ),
        BuiltinKeybinding(
            id: "paste",
            description: "Paste",
            category: "Terminal",
            defaultShortcut: "Cmd+V",
            keyEquivalent: "v",
            modifierFlags: [.command]
        ),
        BuiltinKeybinding(
            id: "split_horizontal",
            description: "Split Horizontally",
            category: "Terminal",
            defaultShortcut: "Cmd+Shift+E",
            keyEquivalent: "E",
            modifierFlags: [.command, .shift]
        ),
        BuiltinKeybinding(
            id: "split_vertical",
            description: "Split Vertically",
            category: "Terminal",
            defaultShortcut: "Cmd+Shift+O",
            keyEquivalent: "O",
            modifierFlags: [.command, .shift]
        ),
        BuiltinKeybinding(
            id: "focus_prev_split",
            description: "Focus Previous Split",
            category: "Terminal",
            defaultShortcut: "Alt+Left",
            keyEquivalent: String(Character(UnicodeScalar(NSLeftArrowFunctionKey)!)),
            modifierFlags: [.option]
        ),
        BuiltinKeybinding(
            id: "focus_next_split",
            description: "Focus Next Split",
            category: "Terminal",
            defaultShortcut: "Alt+Right",
            keyEquivalent: String(Character(UnicodeScalar(NSRightArrowFunctionKey)!)),
            modifierFlags: [.option]
        ),
        // -- Editor --
        BuiltinKeybinding(
            id: "save",
            description: "Save File",
            category: "Editor",
            defaultShortcut: "Cmd+S",
            keyEquivalent: "s",
            modifierFlags: [.command]
        ),
        BuiltinKeybinding(
            id: "find",
            description: "Find",
            category: "Editor",
            defaultShortcut: "Cmd+F",
            keyEquivalent: "f",
            modifierFlags: [.command]
        ),
        BuiltinKeybinding(
            id: "go_to_line",
            description: "Go to Line",
            category: "Editor",
            defaultShortcut: "Cmd+G",
            keyEquivalent: "g",
            modifierFlags: [.command]
        ),
        // -- Navigation --
        BuiltinKeybinding(
            id: "toggle_sidebar",
            description: "Toggle Sidebar",
            category: "Navigation",
            defaultShortcut: "Cmd+Shift+B",
            keyEquivalent: "B",
            modifierFlags: [.command, .shift]
        ),
        BuiltinKeybinding(
            id: "project_search",
            description: "Find in Project",
            category: "Navigation",
            defaultShortcut: "Cmd+Shift+F",
            keyEquivalent: "F",
            modifierFlags: [.command, .shift]
        ),
        BuiltinKeybinding(
            id: "command_palette",
            description: "Command Palette",
            category: "Navigation",
            defaultShortcut: "Cmd+Shift+P",
            keyEquivalent: "P",
            modifierFlags: [.command, .shift]
        ),
        BuiltinKeybinding(
            id: "open_settings",
            description: "Open Settings",
            category: "Navigation",
            defaultShortcut: "Cmd+,",
            keyEquivalent: ",",
            modifierFlags: [.command]
        ),
        // -- Font --
        BuiltinKeybinding(
            id: "font_increase",
            description: "Increase Font Size",
            category: "Font",
            defaultShortcut: "Cmd+=",
            keyEquivalent: "=",
            modifierFlags: [.command]
        ),
        BuiltinKeybinding(
            id: "font_decrease",
            description: "Decrease Font Size",
            category: "Font",
            defaultShortcut: "Cmd+-",
            keyEquivalent: "-",
            modifierFlags: [.command]
        ),
        BuiltinKeybinding(
            id: "font_reset",
            description: "Reset Font Size",
            category: "Font",
            defaultShortcut: "Cmd+0",
            keyEquivalent: "0",
            modifierFlags: [.command]
        ),
        // -- App --
        BuiltinKeybinding(
            id: "new_window",
            description: "New Window",
            category: "App",
            defaultShortcut: "Cmd+Shift+N",
            keyEquivalent: "N",
            modifierFlags: [.command, .shift]
        ),
        BuiltinKeybinding(
            id: "fullscreen",
            description: "Toggle Fullscreen",
            category: "App",
            defaultShortcut: "Ctrl+Cmd+F",
            keyEquivalent: "f",
            modifierFlags: [.control, .command]
        ),
    ]

    // MARK: Keybinding Lookup Cache

    /// Cached dictionary mapping "modifierFlags.rawValue-keyEquivalent" to keybinding ID
    /// for O(1) lookup on key events. Invalidated when overrides change.
    private static var lookupCache: [String: String]?
    private static var lastOverrides: [String: String]?

    /// Builds or returns the cached lookup dictionary for the given overrides.
    private static func buildLookup(overrides: [String: String]) -> [String: String] {
        if let cache = lookupCache, lastOverrides == overrides {
            return cache
        }
        var dict: [String: String] = [:]
        for builtin in builtins {
            let effective = getKeybinding(id: builtin.id, overrides: overrides) ?? builtin
            let relevantMask: NSEvent.ModifierFlags = [.command, .control, .option, .shift]
            let mods = effective.modifierFlags.intersection(relevantMask)
            let key = "\(mods.rawValue)-\(effective.keyEquivalent.lowercased())"
            dict[key] = effective.id
        }
        lookupCache = dict
        lastOverrides = overrides
        return dict
    }

    // MARK: Lookup

    /// Returns the keybinding for the given ID, applying any user override for
    /// the shortcut. If an override exists, the `keyEquivalent` and
    /// `modifierFlags` reflect the overridden shortcut.
    static func getKeybinding(id: String, overrides: [String: String] = [:]) -> BuiltinKeybinding? {
        guard let builtin = builtins.first(where: { $0.id == id }) else {
            return nil
        }
        if let overrideStr = overrides[id], !overrideStr.isEmpty {
            let parsed = parseShortcut(overrideStr)
            return BuiltinKeybinding(
                id: builtin.id,
                description: builtin.description,
                category: builtin.category,
                defaultShortcut: overrideStr,
                keyEquivalent: parsed.keyEquivalent,
                modifierFlags: parsed.modifierFlags
            )
        }
        return builtin
    }

    /// Ordered list of keybinding categories for display purposes.
    static func categories() -> [String] {
        ["Tabs", "Terminal", "Editor", "Navigation", "Font", "App"]
    }

    // MARK: Shortcut String Parsing

    /// Parses a human-readable shortcut string like "Cmd+Shift+B" into a
    /// `(keyEquivalent, modifierFlags)` tuple suitable for NSMenuItem or event
    /// matching.
    ///
    /// Recognized modifier names (case-insensitive):
    /// - Cmd, Command
    /// - Ctrl, Control
    /// - Shift
    /// - Alt, Option, Opt
    ///
    /// Special key names: Tab, Left, Right, Up, Down, Space, Return, Enter,
    /// Escape, Delete, Backspace, F1-F20.
    static func parseShortcut(_ shortcut: String) -> (keyEquivalent: String, modifierFlags: NSEvent.ModifierFlags) {
        let parts = shortcut.split(separator: "+").map { $0.trimmingCharacters(in: .whitespaces) }
        guard !parts.isEmpty else { return ("", []) }

        var flags: NSEvent.ModifierFlags = []
        var keyPart = ""

        for (index, part) in parts.enumerated() {
            if index < parts.count - 1 {
                switch part.lowercased() {
                case "cmd", "command":   flags.insert(.command)
                case "ctrl", "control":  flags.insert(.control)
                case "shift":            flags.insert(.shift)
                case "alt", "option", "opt": flags.insert(.option)
                default: break
                }
            } else {
                keyPart = part
            }
        }

        let keyEquivalent = keyNameToEquivalent(keyPart)
        return (keyEquivalent, flags)
    }

    // MARK: Display Helpers

    /// Returns a human-readable shortcut string for a keybinding, like
    /// "Cmd+Shift+B". Takes into account user overrides from settings.
    static func shortcutDisplay(forId id: String, overrides: [String: String] = [:]) -> String? {
        if let override_ = overrides[id], !override_.isEmpty {
            return override_
        }
        guard let binding = builtins.first(where: { $0.id == id }) else { return nil }
        return binding.defaultShortcut
    }

    /// Converts modifier flags to a display string using standard names.
    static func modifierDisplay(_ mask: NSEvent.ModifierFlags) -> String {
        var parts: [String] = []
        if mask.contains(.control) { parts.append("Ctrl") }
        if mask.contains(.option)  { parts.append("Alt") }
        if mask.contains(.shift)   { parts.append("Shift") }
        if mask.contains(.command) { parts.append("Cmd") }
        return parts.joined(separator: "+")
    }

    /// Converts modifier flags to macOS symbol notation.
    static func modifierSymbols(_ mask: NSEvent.ModifierFlags) -> String {
        var parts: [String] = []
        if mask.contains(.control) { parts.append("\u{2303}") }
        if mask.contains(.option)  { parts.append("\u{2325}") }
        if mask.contains(.shift)   { parts.append("\u{21E7}") }
        if mask.contains(.command) { parts.append("\u{2318}") }
        return parts.joined()
    }

    /// Converts a key equivalent character to a display-friendly name.
    static func keyDisplay(_ key: String) -> String {
        switch key {
        case "\t":       return "Tab"
        case "\u{0019}": return "Tab"
        case "\r":       return "Return"
        case " ":        return "Space"
        case ",":        return ","
        case ".":        return "."
        case "=":        return "="
        case "-":        return "-"
        default:
            if key.unicodeScalars.count == 1 {
                let scalar = key.unicodeScalars.first!
                switch Int(scalar.value) {
                case 27:                      return "Escape"
                case 127:                     return "Delete"
                case NSLeftArrowFunctionKey:  return "Left"
                case NSRightArrowFunctionKey: return "Right"
                case NSUpArrowFunctionKey:    return "Up"
                case NSDownArrowFunctionKey:  return "Down"
                case NSF1FunctionKey:  return "F1"
                case NSF2FunctionKey:  return "F2"
                case NSF3FunctionKey:  return "F3"
                case NSF4FunctionKey:  return "F4"
                case NSF5FunctionKey:  return "F5"
                case NSF6FunctionKey:  return "F6"
                case NSF7FunctionKey:  return "F7"
                case NSF8FunctionKey:  return "F8"
                case NSF9FunctionKey:  return "F9"
                case NSF10FunctionKey: return "F10"
                case NSF11FunctionKey: return "F11"
                case NSF12FunctionKey: return "F12"
                default: break
                }
            }
            return key.uppercased()
        }
    }

    // MARK: Event Matching

    /// Checks whether an NSEvent matches a given keybinding.
    static func eventMatches(_ event: NSEvent, keybinding: BuiltinKeybinding) -> Bool {
        let relevantMask: NSEvent.ModifierFlags = [.command, .control, .option, .shift]
        let eventMods = event.modifierFlags.intersection(relevantMask)
        let bindingMods = keybinding.modifierFlags.intersection(relevantMask)
        guard eventMods == bindingMods else { return false }

        guard let chars = event.charactersIgnoringModifiers?.lowercased() else { return false }
        let expected = keybinding.keyEquivalent.lowercased()

        if chars == expected { return true }

        if chars.unicodeScalars.count == 1 && expected.unicodeScalars.count == 1 {
            return chars.unicodeScalars.first!.value == expected.unicodeScalars.first!.value
        }

        return false
    }

    /// Checks whether an NSEvent matches a given key equivalent and modifier flags.
    /// Reusable for custom keybindings that are not `BuiltinKeybinding` instances.
    static func eventMatchesShortcut(_ event: NSEvent, keyEquivalent: String, modifierFlags: NSEvent.ModifierFlags) -> Bool {
        let relevantMask: NSEvent.ModifierFlags = [.command, .control, .option, .shift]
        let eventMods = event.modifierFlags.intersection(relevantMask)
        let bindingMods = modifierFlags.intersection(relevantMask)
        guard eventMods == bindingMods else { return false }

        guard let chars = event.charactersIgnoringModifiers?.lowercased() else { return false }
        let expected = keyEquivalent.lowercased()

        if chars == expected { return true }

        if chars.unicodeScalars.count == 1 && expected.unicodeScalars.count == 1 {
            return chars.unicodeScalars.first!.value == expected.unicodeScalars.first!.value
        }

        return false
    }

    /// Finds the first keybinding matching the given event, with override support.
    /// Uses a cached dictionary for O(1) lookup instead of scanning all keybindings.
    static func matchingKeybinding(for event: NSEvent, overrides: [String: String] = [:]) -> BuiltinKeybinding? {
        let relevantMask: NSEvent.ModifierFlags = [.command, .control, .option, .shift]
        let eventMods = event.modifierFlags.intersection(relevantMask)
        guard let chars = event.charactersIgnoringModifiers?.lowercased() else { return nil }

        let lookupKey = "\(eventMods.rawValue)-\(chars)"
        let dict = buildLookup(overrides: overrides)

        if let id = dict[lookupKey] {
            return getKeybinding(id: id, overrides: overrides)
        }

        // Fallback: check unicode scalar values for function keys etc.
        if chars.unicodeScalars.count == 1 {
            for builtin in builtins {
                let effective = getKeybinding(id: builtin.id, overrides: overrides) ?? builtin
                if eventMatches(event, keybinding: effective) {
                    return effective
                }
            }
        }

        return nil
    }

    // MARK: Private

    /// Converts a display-friendly key name to an NSEvent key equivalent string.
    private static func keyNameToEquivalent(_ name: String) -> String {
        switch name.lowercased() {
        case "tab":        return "\t"
        case "return", "enter": return "\r"
        case "space":      return " "
        case "escape", "esc": return String(Character(UnicodeScalar(27)))
        case "delete", "backspace": return String(Character(UnicodeScalar(127)))
        case "left":       return String(Character(UnicodeScalar(NSLeftArrowFunctionKey)!))
        case "right":      return String(Character(UnicodeScalar(NSRightArrowFunctionKey)!))
        case "up":         return String(Character(UnicodeScalar(NSUpArrowFunctionKey)!))
        case "down":       return String(Character(UnicodeScalar(NSDownArrowFunctionKey)!))
        case "f1":  return String(Character(UnicodeScalar(NSF1FunctionKey)!))
        case "f2":  return String(Character(UnicodeScalar(NSF2FunctionKey)!))
        case "f3":  return String(Character(UnicodeScalar(NSF3FunctionKey)!))
        case "f4":  return String(Character(UnicodeScalar(NSF4FunctionKey)!))
        case "f5":  return String(Character(UnicodeScalar(NSF5FunctionKey)!))
        case "f6":  return String(Character(UnicodeScalar(NSF6FunctionKey)!))
        case "f7":  return String(Character(UnicodeScalar(NSF7FunctionKey)!))
        case "f8":  return String(Character(UnicodeScalar(NSF8FunctionKey)!))
        case "f9":  return String(Character(UnicodeScalar(NSF9FunctionKey)!))
        case "f10": return String(Character(UnicodeScalar(NSF10FunctionKey)!))
        case "f11": return String(Character(UnicodeScalar(NSF11FunctionKey)!))
        case "f12": return String(Character(UnicodeScalar(NSF12FunctionKey)!))
        case "f13": return String(Character(UnicodeScalar(NSF13FunctionKey)!))
        case "f14": return String(Character(UnicodeScalar(NSF14FunctionKey)!))
        case "f15": return String(Character(UnicodeScalar(NSF15FunctionKey)!))
        case "f16": return String(Character(UnicodeScalar(NSF16FunctionKey)!))
        case "f17": return String(Character(UnicodeScalar(NSF17FunctionKey)!))
        case "f18": return String(Character(UnicodeScalar(NSF18FunctionKey)!))
        case "f19": return String(Character(UnicodeScalar(NSF19FunctionKey)!))
        case "f20": return String(Character(UnicodeScalar(NSF20FunctionKey)!))
        case ",":   return ","
        case ".":   return "."
        case "=":   return "="
        case "-":   return "-"
        case "0":   return "0"
        default:
            if name.count == 1 { return name.lowercased() }
            return name
        }
    }
}
