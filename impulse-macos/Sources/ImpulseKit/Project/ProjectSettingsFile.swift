// Writing project settings back as TOML. Project Setup saves into
// `.impulse/project.toml`, replacing only the sections it manages:
// everything else in the file (comments, keys Impulse doesn't know) stays
// as written. Finish Task remembers its answer in the local
// `.git/impulse/project.toml`, which Impulse writes whole while it's
// Impulse's own.

import Foundation

public enum ProjectSettingsFile {
  /// The section headers the screen manages.
  public static let managedHeaders: Set<String> = [
    "[scripts]", "[worktrees]", "[worktrees.ports]", "[worktrees.env]", "[worktrees.database]", "[on_change]",
    "[finish]", "[[actions]]",
  ]

  /// `config` as TOML, its sections in a fixed order; settings left at
  /// their defaults are left out. A local file written over a committed
  /// one (`clearing`) sets what it turns off explicitly (`setup = ""`,
  /// `clone = []`), since a key it left out would come from the committed
  /// file.
  public static func text(_ config: ProjectConfig, clearing committed: ProjectConfig? = nil) -> String {
    var sections: [String] = []
    var scripts: [String] = []
    func script(_ name: String, _ value: String?, _ earlier: String?) {
      if let value { scripts.append("\(name) = \(string(value))") } else if earlier != nil { scripts.append("\(name) = \"\"") }
    }
    script("setup", config.setupScript, committed?.setupScript)
    script("check", config.checkScript, committed?.checkScript)
    script("archive", config.archiveScript, committed?.archiveScript)
    if !scripts.isEmpty { sections.append((["[scripts]"] + scripts).joined(separator: "\n")) }

    var worktrees: [String] = []
    if !config.worktreeCopy.isEmpty || committed?.worktreeCopy.isEmpty == false {
      worktrees.append("copy = \(array(config.worktreeCopy))")
    }
    if !config.worktreeClone.isEmpty || committed?.worktreeClone.isEmpty == false {
      worktrees.append("clone = \(array(config.worktreeClone))")
    }
    if config.envFile != ProjectConfig().envFile { worktrees.append("env_file = \(string(config.envFile))") }
    if config.portOffset != ProjectConfig().portOffset { worktrees.append("port_offset = \(config.portOffset)") }
    if config.composeOverride || committed?.composeOverride == true {
      worktrees.append("compose_override = \(config.composeOverride)")
    }
    if !config.overlapIgnore.isEmpty || committed?.overlapIgnore.isEmpty == false {
      worktrees.append("overlap_ignore = \(array(config.overlapIgnore))")
    }
    if !worktrees.isEmpty { sections.append((["[worktrees]"] + worktrees).joined(separator: "\n")) }

    if !config.ports.isEmpty {
      sections.append(
        (["[worktrees.ports]"] + config.ports.keys.sorted().map { "\(key($0)) = \(config.ports[$0]!)" })
          .joined(separator: "\n"))
    }
    if !config.worktreeEnv.isEmpty {
      sections.append(
        (["[worktrees.env]"] + config.worktreeEnv.keys.sorted().map { "\(key($0)) = \(string(config.worktreeEnv[$0]!))" })
          .joined(separator: "\n"))
    }
    if let folder = config.databaseFolder {
      var lines = ["[worktrees.database]", "clone = \(string(folder))"]
      if let service = config.databaseService { lines.append("service = \(string(service))") }
      sections.append(lines.joined(separator: "\n"))
    } else if committed?.databaseFolder != nil {
      sections.append("[worktrees.database]\nclone = \"\"")
    }
    if !config.onChange.isEmpty {
      sections.append(
        (["[on_change]"] + config.onChange.keys.sorted().map { "\(key($0)) = \(string(config.onChange[$0]!))" })
          .joined(separator: "\n"))
    }
    if let landing = config.landing {
      sections.append("[finish]\nland = \(string(landing.rawValue))")
    } else if committed?.landing != nil {
      sections.append("[finish]\nland = \"\"")
    }
    for action in config.actions {
      var lines = ["[[actions]]", "name = \(string(action.name))", "command = \(string(action.command))"]
      if let cwd = action.cwd { lines.append("cwd = \(string(cwd))") }
      if let open = action.open { lines.append("open = \(string(open))") }
      sections.append(lines.joined(separator: "\n"))
    }
    return sections.isEmpty ? "" : sections.joined(separator: "\n\n") + "\n"
  }

  /// `existing` with the managed sections replaced by `config`'s, where
  /// the first of them was; the rest of the file stays as written. The
  /// same settings written another way go too (`["scripts"]`, or
  /// `scripts.setup = …` before any header), so a field emptied on the
  /// screen doesn't live on in them.
  public static func merging(_ config: ProjectConfig, into existing: String) -> String {
    let blocks = split(existing)
    var parts: [String] = []
    var insertAt: Int?
    for block in blocks {
      if let header = block.header, managedHeaders.contains(header) {
        if insertAt == nil { insertAt = parts.count }
        continue
      }
      let kept = (block.header == nil ? droppingManagedKeys(block.text) : block.text).trimmingCharacters(in: .newlines)
      // Not an empty start of the file: the sections would go after the
      // block that follows it.
      if !kept.isEmpty { parts.append(kept) }
    }
    let managed = text(config).trimmingCharacters(in: .newlines)
    if !managed.isEmpty {
      let index = min(insertAt ?? parts.count, parts.count)
      parts.insert(managed, at: index)
    }
    return parts.isEmpty ? "" : parts.joined(separator: "\n\n") + "\n"
  }

  /// Whether `text` reads as exactly `config`. Project Setup trusts a file
  /// it saved only then: `merging` keeps what it doesn't manage as
  /// written, and a setting there would otherwise be trusted unseen.
  public static func holdsOnly(_ config: ProjectConfig, in text: String) -> Bool {
    (try? ProjectConfig.parse(text).get()) == config
  }

  /// `existing` with `[finish] land` set to `landing`, for Finish Task
  /// remembering its answer. A file the screen wrote is written again the
  /// same way; in any other, only the `[finish]` section changes.
  public static func setting(_ landing: ProjectConfig.Landing, in existing: String) -> String {
    if case .success(var config) = ProjectConfig.parse(existing), text(config) == existing {
      config.landing = landing
      return text(config)
    }
    let section = ["[finish]", "land = \(string(landing.rawValue))"]
    var lines = existing.components(separatedBy: "\n")
    if let start = lines.firstIndex(where: { header(of: $0) == "[finish]" }) {
      var end = start + 1
      while end < lines.count, header(of: lines[end]) == nil { end += 1 }
      while end > start + 1, lines[end - 1].trimmingCharacters(in: .whitespaces).isEmpty { end -= 1 }
      lines.replaceSubrange(start..<end, with: section)
    } else if let actions = lines.firstIndex(where: { header(of: $0) == "[[actions]]" }) {
      lines.insert(contentsOf: section + [""], at: actions)
    } else {
      while lines.last?.trimmingCharacters(in: .whitespaces).isEmpty == true { lines.removeLast() }
      if !lines.isEmpty { lines.append("") }
      lines += section + [""]
    }
    return lines.joined(separator: "\n")
  }

  /// Whether `text` is a whole file as `text(_:clearing:)` writes it (by
  /// Project Setup, or Finish Task remembering its answer), so replacing
  /// it loses nothing written by hand. A local file may clear settings of
  /// the `committed` one.
  public static func isWrittenByImpulse(_ text: String, over committed: ProjectConfig? = nil) -> Bool {
    guard case .success(let config) = ProjectConfig.parse(text) else { return false }
    return Self.text(config) == text || Self.text(config, clearing: committed) == text
  }

  /// Whether the sections a rewrite replaces hold comments it would drop.
  public static func managedSectionsHaveComments(_ text: String) -> Bool {
    split(text).contains { block in
      guard let header = block.header, managedHeaders.contains(header) else { return false }
      return block.text.components(separatedBy: "\n").dropFirst().contains {
        $0.trimmingCharacters(in: .whitespaces).hasPrefix("#")
      }
    }
  }

  /// The keys each managed section can hold; nil for the tables of names
  /// (`[worktrees.ports]`), which hold any. Under `[worktrees]`, `ports`,
  /// `env` and `database` start dotted keys of its tables.
  private static let knownKeys: [String: Set<String>?] = [
    "[scripts]": ["setup", "archive", "check"],
    "[worktrees]": [
      "copy", "clone", "env_file", "port_offset", "compose_override", "overlap_ignore", "ports", "env", "database",
    ],
    "[worktrees.ports]": nil, "[worktrees.env]": nil, "[on_change]": nil,
    "[worktrees.database]": ["clone", "service"],
    "[finish]": ["land"],
    "[[actions]]": ["name", "command", "cwd", "open"],
  ]

  /// The keys in the managed sections of `text` that Project Setup doesn't
  /// know, which a rewrite drops (`scripts.lint`), in file order. That
  /// includes ones set before any header (`scripts.lint = …`).
  public static func unknownManagedKeys(_ text: String) -> [String] {
    var unknown: [String] = []
    for block in split(text) {
      let lines = block.text.components(separatedBy: "\n")
      var scanner = StatementScanner()
      for line in block.header == nil ? lines[...] : lines.dropFirst() {
        // A line of a multi-line string or array isn't a key.
        let startsStatement = scanner.isAtStatementStart
        scanner.scan(line)
        guard startsStatement, let path = keyPath(of: line) else { continue }
        var parts = path[...]
        let header: String
        if let table = block.header {
          header = table
        } else {
          // Before any header, only dotted keys of the screen's tables.
          guard let table = parts.first, managedTables.contains(table), parts.count > 1 else { continue }
          header = table == "actions" ? "[[actions]]" : "[\(table)]"
          parts = parts.dropFirst()
        }
        guard let known = knownKeys[header], let known, let key = parts.first, !known.contains(key) else { continue }
        let name = "\(header.trimmingCharacters(in: CharacterSet(charactersIn: "[]"))).\(key)"
        if !unknown.contains(name) { unknown.append(name) }
      }
    }
    return unknown
  }

  /// The tables the screen manages, by their first name.
  private static let managedTables = Set(
    managedHeaders.map { $0.trimmingCharacters(in: CharacterSet(charactersIn: "[]")).components(separatedBy: ".")[0] })

  /// `text` (the keys before any header) without the statements that set
  /// the screen's tables as dotted keys or inline values
  /// (`scripts.setup = …`, `actions = [ … ]`), however many lines each
  /// spans.
  private static func droppingManagedKeys(_ text: String) -> String {
    var kept: [String] = []
    var scanner = StatementScanner()
    var dropping = false
    for line in text.components(separatedBy: "\n") {
      if scanner.isAtStatementStart {
        dropping = keyPath(of: line).map { managedTables.contains($0[0]) } ?? false
      }
      scanner.scan(line)
      if !dropping { kept.append(line) }
    }
    return kept.joined(separator: "\n")
  }

  /// The parts of the key a line sets, unquoted (`worktrees."ports".APP = 1`
  /// → worktrees, ports, APP); nil for comments, headers and anything else.
  private static func keyPath(of line: String) -> [String]? {
    guard let key = keyPath(Substring(line)), key.rest.first == "=" else { return nil }
    return key.parts
  }

  /// A dotted key at the start of `text`, unquoted, and what follows it
  /// (from its first non-blank character); nil when it doesn't start with
  /// one.
  private static func keyPath(_ text: Substring) -> (parts: [String], rest: Substring)? {
    let blank = { (character: Character) in character == " " || character == "\t" }
    var rest = text.drop(while: blank)
    var parts: [String] = []
    while true {
      if let quote = rest.first, quote == "\"" || quote == "'" {
        guard let quoted = quotedKey(rest, quote: quote) else { return nil }
        parts.append(quoted.key)
        rest = quoted.rest
      } else {
        let bare = rest.prefix { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "_" || $0 == "-") }
        guard !bare.isEmpty else { return nil }
        parts.append(String(bare))
        rest = rest.dropFirst(bare.count)
      }
      rest = rest.drop(while: blank)
      guard rest.first == "." else { return (parts, rest) }
      rest = rest.dropFirst().drop(while: blank)
    }
  }

  /// A quoted key at the start of `text` and what follows it. A basic
  /// string's escapes are read as TOML reads them, so `"scripts"` is
  /// `scripts`.
  private static func quotedKey(_ text: Substring, quote: Character) -> (key: String, rest: Substring)? {
    var rest = text.dropFirst()
    var key = ""
    while let character = rest.first {
      rest = rest.dropFirst()
      if character == quote { return (key, rest) }
      guard quote == "\"", character == "\\", let escape = rest.first else {
        key.append(character)
        continue
      }
      rest = rest.dropFirst()
      switch escape {
      case "b": key.append("\u{8}")
      case "t": key.append("\t")
      case "n": key.append("\n")
      case "f": key.append("\u{C}")
      case "r": key.append("\r")
      case "u", "U":
        let digits = rest.prefix(escape == "u" ? 4 : 8)
        guard let value = UInt32(digits, radix: 16), let scalar = Unicode.Scalar(value) else { return nil }
        key.unicodeScalars.append(scalar)
        rest = rest.dropFirst(digits.count)
      default: key.append(escape)
      }
    }
    return nil
  }

  /// Where a TOML file's statements start: not inside a multi-line string
  /// or a value's brackets, which may span lines.
  private struct StatementScanner {
    private var multiline: String?
    private var depth = 0

    var isAtStatementStart: Bool { multiline == nil && depth == 0 }

    mutating func scan(_ line: String) {
      let characters = Array(line)
      var index = 0
      func starts(_ text: String) -> Bool { characters[index...].starts(with: text) }
      while index < characters.count {
        if let delimiter = multiline {
          if starts(delimiter) {
            multiline = nil
            index += 3
          } else {
            index += delimiter == "\"\"\"" && characters[index] == "\\" ? 2 : 1
          }
          continue
        }
        if starts("\"\"\"") || starts("'''") {
          multiline = String(characters[index..<index + 3])
          index += 3
          continue
        }
        switch characters[index] {
        case "#": return
        case "\"":
          index += 1
          while index < characters.count, characters[index] != "\"" { index += characters[index] == "\\" ? 2 : 1 }
        case "'":
          index += 1
          while index < characters.count, characters[index] != "'" { index += 1 }
        case "[", "{": depth += 1
        case "]", "}": depth = max(0, depth - 1)
        default: break
        }
        index += 1
      }
    }
  }

  /// The file in blocks: what comes before the first header, then each
  /// header with the lines under it. A line inside a multi-line value is
  /// never a header.
  private static func split(_ text: String) -> [(header: String?, text: String)] {
    var blocks: [(header: String?, text: String)] = [(nil, "")]
    var scanner = StatementScanner()
    for line in text.components(separatedBy: "\n") {
      if scanner.isAtStatementStart, let header = header(of: line) {
        blocks.append((header, line))
      } else {
        blocks[blocks.count - 1].text += (blocks[blocks.count - 1].text.isEmpty ? "" : "\n") + line
      }
      scanner.scan(line)
    }
    return blocks
  }

  /// The table header a line starts, as the screen writes it: `[scripts]`
  /// for `[ "scripts" ]`, `[[actions]]`; nil for any other line.
  private static func header(of line: String) -> String? {
    let trimmed = line.trimmingCharacters(in: .whitespaces)
    guard trimmed.hasPrefix("[") else { return nil }
    let isArray = trimmed.hasPrefix("[[")
    if let path = keyPath(trimmed.dropFirst(isArray ? 2 : 1)), path.rest.hasPrefix(isArray ? "]]" : "]") {
      let name = path.parts.map(key).joined(separator: ".")
      return isArray ? "[[\(name)]]" : "[\(name)]"
    }
    guard let close = trimmed.range(of: "]]") ?? trimmed.range(of: "]") else { return nil }
    return String(trimmed[..<close.upperBound]).replacingOccurrences(of: " ", with: "")
  }

  /// A TOML basic string. TOML forbids raw control characters in one (a
  /// pasted `\r\n`), so every one is escaped.
  static func string(_ value: String) -> String {
    var escaped = ""
    for scalar in value.unicodeScalars {
      switch scalar {
      case "\\": escaped += "\\\\"
      case "\"": escaped += "\\\""
      case "\n": escaped += "\\n"
      case "\t": escaped += "\\t"
      case "\r": escaped += "\\r"
      case _ where scalar.value < 0x20 || scalar.value == 0x7F:
        escaped += String(format: "\\u%04X", scalar.value)
      default: escaped.unicodeScalars.append(scalar)
      }
    }
    return "\"\(escaped)\""
  }

  static func array(_ values: [String]) -> String {
    "[" + values.map(string).joined(separator: ", ") + "]"
  }

  /// A key, quoted unless it's a bare key (`APP_PORT`, not `composer.lock`).
  static func key(_ name: String) -> String {
    name.allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "_" || $0 == "-") } && !name.isEmpty
      ? name : string(name)
  }
}
