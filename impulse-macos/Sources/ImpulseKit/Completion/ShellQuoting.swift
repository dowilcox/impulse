// Quoting for text Impulse types into a terminal on the user's behalf (file
// names from completion, pasted or dropped paths, shortcut arguments), so it
// reaches the program as one literal word in the shell that's running.

import Foundation

public enum ShellQuoting {
  /// `text` as one word for `shell`: as is when every character is safe
  /// bare, otherwise in single quotes. sh, bash and zsh close the quotes
  /// around a `'` (`'\''`); fish reads `\\` and `\'` as escapes inside single
  /// quotes, so those two get a backslash. Control characters other than tab
  /// and newline go outside the quotes, written as escapes (see
  /// `encodedControl`), so they never reach a line editor as keys.
  public static func quoted(_ text: String, for shell: ShellType) -> String {
    if text.isEmpty { return "''" }
    if text.unicodeScalars.allSatisfy(isSafe) { return text }
    var out = ""
    var run = String.UnicodeScalarView()
    func flush() {
      guard !run.isEmpty else { return }
      let literal = String(run)
      switch shell {
      case .fish:
        out += "'\(literal.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "'", with: "\\'"))'"
      case .bash, .zsh:
        out += "'\(literal.replacingOccurrences(of: "'", with: "'\\''"))'"
      }
      run = String.UnicodeScalarView()
    }
    for scalar in text.unicodeScalars {
      if isEncoded(scalar) {
        flush()
        out += encodedControl(scalar, for: shell)
      } else {
        run.append(scalar)
      }
    }
    flush()
    return out
  }

  /// `text` with a backslash before each character a shell reads specially,
  /// the way the shells' own Tab completion inserts a name (`my\ file`), so
  /// what was typed before it still reads the same. Newlines and tabs go in
  /// single quotes instead (a backslash before a newline continues the
  /// line), and other control characters as escapes for `shell`.
  public static func escaped(_ text: String, for shell: ShellType) -> String {
    var out = ""
    for scalar in text.unicodeScalars {
      if scalar == "\n" || scalar == "\t" {
        out += "'\(scalar)'"
      } else if isEncoded(scalar) {
        out += encodedControl(scalar, for: shell)
      } else {
        if !isSafe(scalar) { out += "\\" }
        out.unicodeScalars.append(scalar)
      }
    }
    return out
  }

  /// What replaces the token `typed` (as the user typed it, up to the
  /// cursor) for a completion the shell itself came up with (fish's
  /// `complete -C`), which arrives as the shell would print it: unescaped,
  /// with a leading `~/` or `$NAME/` left as typed. That head stays as typed
  /// (so it still expands) and the rest is escaped. A variable name the shell
  /// completed (`$HO` → `$HOME`) goes in as is.
  public static func completionText(_ value: String, typed: String, for shell: ShellType) -> String {
    if isVariable(typed, allowEmptyName: true), isVariable(value, allowEmptyName: false),
      value.utf8.starts(with: typed.utf8)
    {
      return value
    }
    let head = expansionHead(typed)
    if !head.isEmpty, value.utf8.starts(with: head.utf8) {
      let rest = String(decoding: Array(value.utf8.dropFirst(head.utf8.count)), as: UTF8.self)
      return head + escaped(rest, for: shell)
    }
    return escaped(value, for: shell)
  }

  /// A leading `~/` or `$NAME/` of `typed`, which the shell expands (empty
  /// when there's none).
  public static func expansionHead(_ typed: String) -> String {
    if typed.hasPrefix("~/") { return "~/" }
    guard typed.hasPrefix("$"), let slash = typed.firstIndex(of: "/") else { return "" }
    let head = String(typed[...slash])
    return isVariable(String(head.dropLast()), allowEmptyName: false) ? head : ""
  }

  /// `$NAME`: a dollar sign and a variable name (letters, digits and `_`, not
  /// starting with a digit).
  private static func isVariable(_ text: String, allowEmptyName: Bool) -> Bool {
    guard text.first == "$" else { return false }
    let name = text.unicodeScalars.dropFirst()
    guard let first = name.first else { return allowEmptyName }
    let word: (Unicode.Scalar) -> Bool = { $0 == "_" || ($0.isASCII && $0.properties.isAlphabetic) || ("0"..."9").contains($0) }
    return !("0"..."9").contains(first) && name.allSatisfy(word)
  }

  /// `text` without control characters, which a line editor would act on as
  /// keys (^U clears the line, ^M runs it). Tab and newline stay.
  public static func withoutControlCharacters(_ text: String) -> String {
    guard text.unicodeScalars.contains(where: isEncoded) else { return text }
    return String(String.UnicodeScalarView(text.unicodeScalars.filter { !isEncoded($0) }))
  }

  /// C0, DEL and C1.
  public static func isControl(_ scalar: Unicode.Scalar) -> Bool {
    scalar.value < 0x20 || (0x7F...0x9F).contains(scalar.value)
  }

  /// Control characters that can't go in quotes as they are: all but tab
  /// and newline.
  private static func isEncoded(_ scalar: Unicode.Scalar) -> Bool {
    isControl(scalar) && scalar != "\n" && scalar != "\t"
  }

  /// A control character written as an escape the shell turns back into it
  /// (`Icon\r`, the file behind a custom folder icon, keeps its `\r`): bash
  /// and zsh `$'\x0d'` (ANSI-C quotes, its UTF-8 bytes), fish `\u000d`
  /// outside quotes. A plain POSIX sh without `$'…'` reads it as `$` and
  /// some text, never as the control character itself.
  private static func encodedControl(_ scalar: Unicode.Scalar, for shell: ShellType) -> String {
    switch shell {
    case .fish:
      return String(format: "\\u%04x", scalar.value)
    case .bash, .zsh:
      return "$'" + String(scalar).utf8.map { String(format: "\\x%02x", $0) }.joined() + "'"
    }
  }

  /// Read literally, bare, by every supported shell anywhere in a word:
  /// ASCII letters and digits, `/ _ . - + , : @`, and anything beyond ASCII
  /// (none of the shells gives those a meaning).
  private static func isSafe(_ scalar: Unicode.Scalar) -> Bool {
    switch scalar.value {
    case 0x30...0x39, 0x41...0x5A, 0x61...0x7A: return true
    case 0x2B, 0x2C, 0x2D, 0x2E, 0x2F, 0x3A, 0x40, 0x5F: return true  // + , - . / : @ _
    default: return scalar.value > 0x9F
    }
  }
}
