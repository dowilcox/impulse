import AppKit

/// Translates NSEvent key events into terminal escape byte sequences.
struct KeyEncoder {

    /// Encode a key event into terminal bytes, respecting the terminal's current mode.
    /// Returns an empty array for events that should be handled by the menu system (Cmd+).
    static func encode(event: NSEvent, appCursor: Bool, appKeypad: Bool) -> [UInt8] {
        let flags = event.modifierFlags
        let hasCmd = flags.contains(.command)
        let hasCtrl = flags.contains(.control)
        let hasShift = flags.contains(.shift)
        let hasOption = flags.contains(.option)

        // Cmd+ combinations are menu shortcuts — don't send to terminal.
        if hasCmd { return [] }

        let keyCode = event.keyCode

        // Shift+Enter → CSI u sequence for multi-line input (Claude Code etc.)
        if (keyCode == 36 || keyCode == 76) && hasShift && !hasCtrl {
            return [0x1B, 0x5B, 0x31, 0x33, 0x3B, 0x32, 0x75] // \e[13;2u
        }

        // Special keys.
        switch keyCode {
        case 36, 76: return [0x0D] // Return / Enter
        case 51: return [0x7F]     // Backspace
        case 48: return [0x09]     // Tab
        case 53: return [0x1B]     // Escape

        // Arrow keys — respect application cursor mode.
        case 126: return appCursor ? [0x1B, 0x4F, 0x41] : [0x1B, 0x5B, 0x41] // Up
        case 125: return appCursor ? [0x1B, 0x4F, 0x42] : [0x1B, 0x5B, 0x42] // Down
        case 124: return appCursor ? [0x1B, 0x4F, 0x43] : [0x1B, 0x5B, 0x43] // Right
        case 123: return appCursor ? [0x1B, 0x4F, 0x44] : [0x1B, 0x5B, 0x44] // Left

        // Navigation keys.
        case 115: return [0x1B, 0x5B, 0x48]             // Home
        case 119: return [0x1B, 0x5B, 0x46]             // End
        case 116: return [0x1B, 0x5B, 0x35, 0x7E]       // Page Up
        case 121: return [0x1B, 0x5B, 0x36, 0x7E]       // Page Down
        case 117: return [0x1B, 0x5B, 0x33, 0x7E]       // Delete (forward)

        // Function keys F1-F12.
        case 122: return [0x1B, 0x4F, 0x50]                         // F1
        case 120: return [0x1B, 0x4F, 0x51]                         // F2
        case 99:  return [0x1B, 0x4F, 0x52]                         // F3
        case 118: return [0x1B, 0x4F, 0x53]                         // F4
        case 96:  return [0x1B, 0x5B, 0x31, 0x35, 0x7E]             // F5
        case 97:  return [0x1B, 0x5B, 0x31, 0x37, 0x7E]             // F6
        case 98:  return [0x1B, 0x5B, 0x31, 0x38, 0x7E]             // F7
        case 100: return [0x1B, 0x5B, 0x31, 0x39, 0x7E]             // F8
        case 101: return [0x1B, 0x5B, 0x32, 0x30, 0x7E]             // F9
        case 109: return [0x1B, 0x5B, 0x32, 0x31, 0x7E]             // F10
        case 103: return [0x1B, 0x5B, 0x32, 0x33, 0x7E]             // F11
        case 111: return [0x1B, 0x5B, 0x32, 0x34, 0x7E]             // F12

        default: break
        }

        // Ctrl+letter → control codes (0x01-0x1A).
        if hasCtrl, let chars = event.charactersIgnoringModifiers?.lowercased(), chars.count == 1 {
            let c = chars.unicodeScalars.first!.value
            if c >= 0x61 && c <= 0x7A { // a-z
                return [UInt8(c - 0x60)]
            }
            switch c {
            case 0x5B: return [0x1B] // Ctrl+[ → ESC
            case 0x5D: return [0x1D] // Ctrl+] → GS
            case 0x5C: return [0x1C] // Ctrl+\ → FS
            default: break
            }
        }

        // Option+key → ESC prefix (meta encoding).
        if hasOption, let chars = event.charactersIgnoringModifiers, chars.count == 1 {
            return [0x1B] + Array(chars.utf8)
        }

        // Regular character input.
        if let chars = event.characters, !chars.isEmpty {
            return Array(chars.utf8)
        }

        return []
    }
}
