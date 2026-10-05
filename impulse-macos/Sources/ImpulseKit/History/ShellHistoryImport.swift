// Readers for the shells' own history files, so Impulse's history can start
// with what the user already typed elsewhere.

import Foundation

public enum ShellHistoryImport {
  /// zsh: plain lines, or EXTENDED_HISTORY `: <start>:<elapsed>;<command>`.
  /// Multi-line commands continue with a trailing backslash. `data` is the
  /// raw file (zsh "metafies" non-ASCII bytes; they're restored here).
  public static func zsh(_ data: Data) -> [HistoryEntry] {
    let text = String(decoding: unmetafy(data), as: UTF8.self)
    var entries: [HistoryEntry] = []
    var pending: (command: String, date: Date?)?

    func flush() {
      if let pending, HistoryEntry.shouldRecord(pending.command) {
        entries.append(HistoryEntry(command: pending.command, startedAt: pending.date ?? .distantPast))
      }
      pending = nil
    }

    for line in text.split(separator: "\n", omittingEmptySubsequences: false) {
      var line = String(line)
      if let current = pending {
        // Continuation of a multi-line command.
        let continues = line.hasSuffix("\\")
        if continues { line.removeLast() }
        pending = (current.command + "\n" + line, current.date)
        if !continues { flush() }
        continue
      }
      var date: Date?
      if line.hasPrefix(": "), let semicolon = line.firstIndex(of: ";") {
        let meta = line[line.index(line.startIndex, offsetBy: 2)..<semicolon]
        if let seconds = meta.split(separator: ":").first.flatMap({ Double($0) }) {
          date = Date(timeIntervalSince1970: seconds)
        }
        line = String(line[line.index(after: semicolon)...])
      }
      if line.hasSuffix("\\") {
        line.removeLast()
        pending = (line, date)
      } else {
        pending = (line, date)
        flush()
      }
    }
    flush()
    return entries
  }

  /// bash: one command per line, optionally preceded by `#<timestamp>`
  /// lines (HISTTIMEFORMAT).
  public static func bash(_ text: String) -> [HistoryEntry] {
    var entries: [HistoryEntry] = []
    var date: Date?
    for line in text.split(separator: "\n") {
      if line.hasPrefix("#"), let seconds = Double(line.dropFirst()) {
        date = Date(timeIntervalSince1970: seconds)
        continue
      }
      let command = String(line)
      if HistoryEntry.shouldRecord(command) {
        entries.append(HistoryEntry(command: command, startedAt: date ?? .distantPast))
      }
      date = nil
    }
    return entries
  }

  /// fish: YAML-like `- cmd: <command>` / `  when: <seconds>` records, with
  /// `\n` and `\\` escapes in the command.
  public static func fish(_ text: String) -> [HistoryEntry] {
    var entries: [HistoryEntry] = []
    var command: String?
    var date: Date?

    func flush() {
      if let command, HistoryEntry.shouldRecord(command) {
        entries.append(HistoryEntry(command: command, startedAt: date ?? .distantPast))
      }
      command = nil
      date = nil
    }

    for line in text.split(separator: "\n") {
      if line.hasPrefix("- cmd: ") {
        flush()
        command = unescapeFish(String(line.dropFirst(7)))
      } else if line.hasPrefix("  when: "), let seconds = Double(line.dropFirst(8)) {
        date = Date(timeIntervalSince1970: seconds)
      }
    }
    flush()
    return entries
  }

  private static func unescapeFish(_ text: String) -> String {
    var result = ""
    var escaping = false
    for character in text {
      if escaping {
        result.append(character == "n" ? "\n" : character)
        escaping = false
      } else if character == "\\" {
        escaping = true
      } else {
        result.append(character)
      }
    }
    if escaping { result.append("\\") }
    return result
  }

  /// Undo zsh's metafication: 0x83 marks that the next byte was XORed with 32.
  static func unmetafy(_ data: Data) -> Data {
    guard data.contains(0x83) else { return data }
    var out = Data(capacity: data.count)
    var iterator = data.makeIterator()
    while let byte = iterator.next() {
      if byte == 0x83, let next = iterator.next() {
        out.append(next ^ 32)
      } else {
        out.append(byte)
      }
    }
    return out
  }
}
