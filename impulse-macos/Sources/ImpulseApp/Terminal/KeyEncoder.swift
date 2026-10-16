import AppKit
import ImpulseKit

/// Translates NSEvent key events into terminal escape byte sequences.
struct KeyEncoder {

    /// Which Option keys act as Meta (`terminal_option_as_meta`): they send
    /// Esc + the key, as readline's Meta-f and Meta-b expect, instead of
    /// typing the character macOS gives them (ƒ, or @ on a German layout).
    enum OptionAsMeta: String, CaseIterable {
        case off, left, right, both

        init(setting: String) {
            self = OptionAsMeta(rawValue: setting.lowercased()) ?? .off
        }

        /// Device-dependent bits of NSEvent.ModifierFlags that tell the two
        /// Option keys apart (NX_DEVICELALTKEYMASK, NX_DEVICERALTKEYMASK).
        private static let leftOptionBit: UInt = 0x20
        private static let rightOptionBit: UInt = 0x40

        /// Whether an Option key held in `flags` is Meta.
        func applies(to flags: NSEvent.ModifierFlags) -> Bool {
            guard flags.contains(.option), self != .off else { return false }
            let left = flags.rawValue & Self.leftOptionBit != 0
            let right = flags.rawValue & Self.rightOptionBit != 0
            switch self {
            case .off: return false
            case .both: return true
            // An event that doesn't say which side (a synthesized one)
            // follows the setting.
            case .left: return left || !right
            case .right: return right || !left
            }
        }
    }

    /// Esc + the key's own character when its Option key is Meta (⌥F sends
    /// Esc f on every layout; ⌥⇧F Esc F; ⌃⌥F Esc ⌃F), or nil for keys that
    /// don't type a character (arrows, Return, Delete: see
    /// `encodeForDoCommand`) and when Option isn't Meta.
    static func metaBytes(for event: NSEvent, optionAsMeta: OptionAsMeta) -> [UInt8]? {
        let flags = event.modifierFlags
        guard !flags.contains(.command), optionAsMeta.applies(to: flags),
            let characters = event.charactersIgnoringModifiers, characters.unicodeScalars.count == 1,
            let scalar = characters.unicodeScalars.first,
            scalar.value >= 0x20, scalar.value != 0x7F, !(0xF700...0xF8FF).contains(scalar.value)
        else { return nil }
        if flags.contains(.control) {
            guard let control = controlByte(for: scalar) else { return nil }
            return [0x1B, control]
        }
        return [0x1B] + Array(String(scalar).utf8)
    }

    /// Encode a key event triggered by an NSTextInputClient `doCommand(by:)` selector.
    /// Handles special keys (arrows, Home/End, PageUp/Down, F-keys, Tab/Return/etc.)
    /// and ⌃ keys; Return, Tab, Delete and Esc get an Esc in front when their
    /// Option key is Meta.
    /// Returns an empty array for events that should be handled by the menu system (Cmd+).
    static func encodeForDoCommand(
        event: NSEvent, appCursor: Bool, appKeypad: Bool, optionAsMeta: OptionAsMeta = .off
    ) -> [UInt8] {
        let flags = event.modifierFlags
        let hasCmd = flags.contains(.command)
        let hasCtrl = flags.contains(.control)
        let hasShift = flags.contains(.shift)
        let hasOption = flags.contains(.option)

        if hasCmd { return [] }

        let keyCode = event.keyCode
        let modParam = modifierParam(shift: hasShift, alt: hasOption, ctrl: hasCtrl)
        let anyMod = modParam > 1
        let meta: [UInt8] = optionAsMeta.applies(to: flags) ? [0x1B] : []

        // Shift+Return is a plain Return: shells read the kitty form
        // (CSI 13;2u) as text. Programs that want it apart ask for the
        // kitty protocol, which keyDown handles before this.
        switch keyCode {
        case 36, 76: return meta + [0x0D]
        case 51: return meta + [0x7F]
        case 48:
            return hasShift ? csiBytes("Z") : meta + [0x09]
        case 53: return meta + [0x1B]

        case 126: return arrowBytes(letter: "A", appCursor: appCursor, modParam: modParam, anyMod: anyMod)
        case 125: return arrowBytes(letter: "B", appCursor: appCursor, modParam: modParam, anyMod: anyMod)
        case 124: return arrowBytes(letter: "C", appCursor: appCursor, modParam: modParam, anyMod: anyMod)
        case 123: return arrowBytes(letter: "D", appCursor: appCursor, modParam: modParam, anyMod: anyMod)

        case 115: return navBytes(letter: "H", modParam: modParam, anyMod: anyMod)
        case 119: return navBytes(letter: "F", modParam: modParam, anyMod: anyMod)
        case 116: return tildeBytes(number: 5, modParam: modParam, anyMod: anyMod)
        case 121: return tildeBytes(number: 6, modParam: modParam, anyMod: anyMod)
        case 117: return tildeBytes(number: 3, modParam: modParam, anyMod: anyMod)

        case 122: return fkeySS3Bytes(letter: "P", modParam: modParam, anyMod: anyMod)
        case 120: return fkeySS3Bytes(letter: "Q", modParam: modParam, anyMod: anyMod)
        case 99:  return fkeySS3Bytes(letter: "R", modParam: modParam, anyMod: anyMod)
        case 118: return fkeySS3Bytes(letter: "S", modParam: modParam, anyMod: anyMod)
        case 96:  return tildeBytes(number: 15, modParam: modParam, anyMod: anyMod)
        case 97:  return tildeBytes(number: 17, modParam: modParam, anyMod: anyMod)
        case 98:  return tildeBytes(number: 18, modParam: modParam, anyMod: anyMod)
        case 100: return tildeBytes(number: 19, modParam: modParam, anyMod: anyMod)
        case 101: return tildeBytes(number: 20, modParam: modParam, anyMod: anyMod)
        case 109: return tildeBytes(number: 21, modParam: modParam, anyMod: anyMod)
        case 103: return tildeBytes(number: 23, modParam: modParam, anyMod: anyMod)
        case 111: return tildeBytes(number: 24, modParam: modParam, anyMod: anyMod)

        default: break
        }

        if hasCtrl, let characters = event.charactersIgnoringModifiers, characters.unicodeScalars.count == 1,
            let scalar = characters.unicodeScalars.first, let control = controlByte(for: scalar)
        {
            return meta + [control]
        }

        if let bytes = metaBytes(for: event, optionAsMeta: optionAsMeta) {
            return bytes
        }

        if let chars = event.characters, !chars.isEmpty {
            return Array(chars.utf8)
        }

        return []
    }

    static func encode(
        event: NSEvent, appCursor: Bool, appKeypad: Bool, optionAsMeta: OptionAsMeta = .off
    ) -> [UInt8] {
        encodeForDoCommand(event: event, appCursor: appCursor, appKeypad: appKeypad, optionAsMeta: optionAsMeta)
    }

    /// The control character ⌃ makes of a key: ⌃A–⌃Z, ⌃[ (Esc), ⌃\ and ⌃].
    private static func controlByte(for scalar: Unicode.Scalar) -> UInt8? {
        var c = scalar.value
        if c >= 0x41 && c <= 0x5A { c += 0x20 }
        switch c {
        case 0x61...0x7A: return UInt8(c - 0x60)
        case 0x5B: return 0x1B
        case 0x5C: return 0x1C
        case 0x5D: return 0x1D
        default: return nil
        }
    }

    /// The key, modifiers and typed text of `event` for the kitty keyboard
    /// protocol (nil for keys it doesn't describe, which keep the legacy path).
    /// Option is the protocol's Alt where it's Meta, or beside ⌃ or ⌘;
    /// otherwise it types characters, which go through the input manager as
    /// text.
    static func kittyKey(for event: NSEvent, optionAsMeta: OptionAsMeta = .off) -> (
        key: KittyKeyboard.Key, modifiers: KittyKeyboard.Modifiers, text: String?
    )? {
        let flags = event.modifierFlags
        let optionIsMeta = optionAsMeta.applies(to: flags)
        var modifiers: KittyKeyboard.Modifiers = []
        if flags.contains(.shift) { modifiers.insert(.shift) }
        if flags.contains(.option) { modifiers.insert(.alt) }
        if flags.contains(.control) { modifiers.insert(.control) }
        if flags.contains(.command) { modifiers.insert(.superKey) }

        let functional: [UInt16: KittyKeyboard.Key] = [
            53: .escape, 36: .enter, 76: .enter, 48: .tab, 51: .backspace,
            114: .insert, 117: .delete, 116: .pageUp, 121: .pageDown,
            126: .up, 125: .down, 124: .right, 123: .left, 115: .home, 119: .end,
            122: .function(1), 120: .function(2), 99: .function(3), 118: .function(4),
            96: .function(5), 97: .function(6), 98: .function(7), 100: .function(8),
            101: .function(9), 109: .function(10), 103: .function(11), 111: .function(12),
        ]
        if let key = functional[event.keyCode] {
            return (key, modifiers, nil)
        }
        // Option alone types a character here (å, or @ on a German layout)
        // or starts a dead key (´): the legacy path composes it and sends the
        // text. Beside ⌃ it can't type anything, so it's Alt (⌃⌥F is C-M-f).
        if flags.contains(.option), !optionIsMeta, modifiers.isDisjoint(with: [.control, .superKey]) {
            return nil
        }
        // The key's own character without modifiers, and with Shift.
        guard let unmodified = event.characters(byApplyingModifiers: []),
            unmodified.unicodeScalars.count == 1,
            let base = unmodified.lowercased().unicodeScalars.first, base.value >= 0x20, base.value != 0x7F
        else { return nil }
        let shifted = event.characters(byApplyingModifiers: .shift)?.unicodeScalars.first
        // What Option makes of the key (ƒ) isn't text it types.
        return (.text(base: base, shifted: shifted), modifiers, flags.contains(.option) ? nil : event.characters)
    }

    private static func modifierParam(shift: Bool, alt: Bool, ctrl: Bool) -> Int {
        1 + (shift ? 1 : 0) + (alt ? 2 : 0) + (ctrl ? 4 : 0)
    }

    private static func csiBytes(_ tail: String) -> [UInt8] {
        [0x1B, 0x5B] + Array(tail.utf8)
    }

    private static func arrowBytes(letter: Character, appCursor: Bool, modParam: Int, anyMod: Bool) -> [UInt8] {
        if anyMod {
            return csiBytes("1;\(modParam)\(letter)")
        }
        let intro: UInt8 = appCursor ? 0x4F : 0x5B
        return [0x1B, intro, UInt8(letter.asciiValue!)]
    }

    private static func navBytes(letter: Character, modParam: Int, anyMod: Bool) -> [UInt8] {
        if anyMod {
            return csiBytes("1;\(modParam)\(letter)")
        }
        return [0x1B, 0x5B, UInt8(letter.asciiValue!)]
    }

    private static func tildeBytes(number: Int, modParam: Int, anyMod: Bool) -> [UInt8] {
        if anyMod {
            return csiBytes("\(number);\(modParam)~")
        }
        return csiBytes("\(number)~")
    }

    private static func fkeySS3Bytes(letter: Character, modParam: Int, anyMod: Bool) -> [UInt8] {
        if anyMod {
            return csiBytes("1;\(modParam)\(letter)")
        }
        return [0x1B, 0x4F, UInt8(letter.asciiValue!)]
    }
}
