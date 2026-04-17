import AppKit

/// Translates NSEvent key events into terminal escape byte sequences.
struct KeyEncoder {

    /// Encode a key event triggered by an NSTextInputClient `doCommand(by:)` selector.
    /// Handles special keys (arrows, Home/End, PageUp/Down, F-keys, Tab/Return/etc.)
    /// and meta-prefix encoding for Option+ASCII combinations that reach doCommand.
    /// Returns an empty array for events that should be handled by the menu system (Cmd+).
    static func encodeForDoCommand(event: NSEvent, appCursor: Bool, appKeypad: Bool) -> [UInt8] {
        let flags = event.modifierFlags
        let hasCmd = flags.contains(.command)
        let hasCtrl = flags.contains(.control)
        let hasShift = flags.contains(.shift)
        let hasOption = flags.contains(.option)

        if hasCmd { return [] }

        let keyCode = event.keyCode
        let modParam = modifierParam(shift: hasShift, alt: hasOption, ctrl: hasCtrl)
        let anyMod = modParam > 1

        if (keyCode == 36 || keyCode == 76) && hasShift && !hasCtrl {
            return csiBytes("13;2u")
        }

        switch keyCode {
        case 36, 76: return [0x0D]
        case 51: return [0x7F]
        case 48:
            return hasShift ? csiBytes("Z") : [0x09]
        case 53: return [0x1B]

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

        if hasCtrl, let chars = event.charactersIgnoringModifiers?.lowercased(), chars.count == 1 {
            let c = chars.unicodeScalars.first!.value
            if c >= 0x61 && c <= 0x7A {
                return [UInt8(c - 0x60)]
            }
            switch c {
            case 0x5B: return [0x1B]
            case 0x5D: return [0x1D]
            case 0x5C: return [0x1C]
            default: break
            }
        }

        if hasOption,
           let chars = event.charactersIgnoringModifiers,
           chars.count == 1,
           let scalar = chars.unicodeScalars.first,
           scalar.value < 0x80 {
            return [0x1B] + Array(chars.utf8)
        }

        if let chars = event.characters, !chars.isEmpty {
            return Array(chars.utf8)
        }

        return []
    }

    /// Encode committed text arriving via NSTextInputClient `insertText(_:replacementRange:)`
    /// when a meta-prefix should be applied. Returns nil to signal the caller should write
    /// the text directly (composed dead keys, IME output, multi-character strings).
    ///
    /// Returns `\e` + ASCII byte only when Option was held AND the text is a single plain
    /// ASCII character — the shape readline/emacs expect for Meta-f, Meta-b, etc.
    static func encodeMetaForInsertText(text: String, event: NSEvent?) -> Data? {
        guard let event, event.modifierFlags.contains(.option) else { return nil }
        guard text.count == 1, let scalar = text.unicodeScalars.first, scalar.value < 0x80 else {
            return nil
        }
        var data = Data([0x1B])
        data.append(contentsOf: Array(text.utf8))
        return data
    }

    static func encode(event: NSEvent, appCursor: Bool, appKeypad: Bool) -> [UInt8] {
        encodeForDoCommand(event: event, appCursor: appCursor, appKeypad: appKeypad)
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
