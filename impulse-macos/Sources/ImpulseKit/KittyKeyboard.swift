// Kitty keyboard protocol (progressive enhancement) key encoding:
// https://sw.kovidgoyal.net/kitty/keyboard-protocol/
//
//   CSI key[:shifted] ; modifiers[:event] ; text u
//   CSI 1 ; modifiers[:event] {A B C D H F P Q S}
//   CSI number ; modifiers[:event] ~
//
// `encode` returns nil when the legacy encoding applies (plain typing with
// only "disambiguate" on, unmodified arrows, …), so the caller keeps its
// usual path for those, IME and dead keys included.

import Foundation

public enum KittyKeyboard {
  /// The enhancements a program asked for (CSI > flags u), in protocol order.
  public struct Flags: OptionSet, Sendable {
    public let rawValue: Int
    public init(rawValue: Int) { self.rawValue = rawValue }
    public static let disambiguate = Flags(rawValue: 1)
    public static let reportEventTypes = Flags(rawValue: 2)
    public static let reportAlternateKeys = Flags(rawValue: 4)
    public static let reportAllKeys = Flags(rawValue: 8)
    public static let reportText = Flags(rawValue: 16)
  }

  public struct Modifiers: OptionSet, Sendable {
    public let rawValue: Int
    public init(rawValue: Int) { self.rawValue = rawValue }
    public static let shift = Modifiers(rawValue: 1)
    public static let alt = Modifiers(rawValue: 2)
    public static let control = Modifiers(rawValue: 4)
    public static let superKey = Modifiers(rawValue: 8)
  }

  public enum Event: Int, Sendable {
    case press = 1, `repeat` = 2, release = 3
  }

  public enum Key: Equatable, Sendable {
    /// A key that types `base` without modifiers (lowercase); `shifted` is
    /// what it types with Shift.
    case text(base: Unicode.Scalar, shifted: Unicode.Scalar?)
    case escape, enter, tab, backspace
    case insert, delete, pageUp, pageDown
    case up, down, right, left, home, end
    /// F1…F12.
    case function(Int)
  }

  public static func encode(
    _ key: Key, modifiers: Modifiers, event: Event, text: String?, flags: Flags
  ) -> [UInt8]? {
    let allKeys = flags.contains(.reportAllKeys)
    let reportEvents = flags.contains(.reportEventTypes)
    let event = reportEvents ? event : (event == .release ? Event.release : .press)
    if event == .release && !reportEvents { return [] }
    let modifierValue = 1 + modifiers.rawValue
    let eventSuffix = event == .press ? "" : ":\(event.rawValue)"
    let plain = modifierValue == 1 && eventSuffix.isEmpty

    func csi(_ body: String) -> [UInt8] { [0x1B, 0x5B] + Array(body.utf8) }
    func withModifiers(_ prefix: String, _ final: String) -> [UInt8] {
      plain ? csi(prefix + final) : csi("\(prefix);\(modifierValue)\(eventSuffix)\(final)")
    }

    switch key {
    case .text(let base, let shifted):
      let commandModifiers = !modifiers.intersection([.alt, .control, .superKey]).isEmpty
      if !allKeys && !commandModifiers {
        // Typing stays text; its releases aren't reported.
        return event == .release ? [] : nil
      }
      var code = "\(base.value)"
      if flags.contains(.reportAlternateKeys), modifiers.contains(.shift), let shifted, shifted != base {
        code += ":\(shifted.value)"
      }
      var textField: String?
      if allKeys, flags.contains(.reportText), event != .release, let text, !text.isEmpty,
        !text.unicodeScalars.contains(where: { $0.value < 0x20 || $0.value == 0x7F })
      {
        textField = text.unicodeScalars.map { String($0.value) }.joined(separator: ":")
      }
      if let textField {
        return csi("\(code);\(modifierValue)\(eventSuffix);\(textField)u")
      }
      return withModifiers(code, "u")

    case .escape, .enter, .tab, .backspace:
      let code: Int
      switch key {
      case .escape: code = 27
      case .enter: code = 13
      case .tab: code = 9
      default: code = 127
      }
      // Enter, Tab and Backspace stay legacy unless modified (so `reset`
      // can still be typed after a program dies in this mode).
      if key != .escape, !allKeys, plain { return nil }
      if key != .escape, !allKeys, event == .release { return [] }
      return withModifiers("\(code)", "u")

    case .up, .down, .right, .left, .home, .end:
      if plain { return nil }
      let letter: String
      switch key {
      case .up: letter = "A"
      case .down: letter = "B"
      case .right: letter = "C"
      case .left: letter = "D"
      case .home: letter = "H"
      default: letter = "F"
      }
      return csi("1;\(modifierValue)\(eventSuffix)\(letter)")

    case .insert, .delete, .pageUp, .pageDown:
      if plain { return nil }
      let number: Int
      switch key {
      case .insert: number = 2
      case .delete: number = 3
      case .pageUp: number = 5
      default: number = 6
      }
      return csi("\(number);\(modifierValue)\(eventSuffix)~")

    case .function(let n):
      if plain { return nil }
      switch n {
      case 1: return csi("1;\(modifierValue)\(eventSuffix)P")
      case 2: return csi("1;\(modifierValue)\(eventSuffix)Q")
      case 4: return csi("1;\(modifierValue)\(eventSuffix)S")
      default:
        let numbers = [3: 13, 5: 15, 6: 17, 7: 18, 8: 19, 9: 20, 10: 21, 11: 23, 12: 24]
        guard let number = numbers[n] else { return nil }
        return csi("\(number);\(modifierValue)\(eventSuffix)~")
      }
    }
  }
}
