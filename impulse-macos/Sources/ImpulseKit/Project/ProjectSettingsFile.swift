// Writing project settings back as TOML, for the project setup screen. The
// local file (`.git/impulse/project.toml`) is the screen's: it's written
// whole. In a committed `.impulse/project.toml`, only the sections the
// screen manages are replaced, and everything else in the file (comments,
// keys Impulse doesn't know) stays as written.

import Foundation

public enum ProjectSettingsFile {
  /// The section headers the screen manages.
  public static let managedHeaders: Set<String> = [
    "[scripts]", "[worktrees]", "[worktrees.ports]", "[worktrees.env]", "[worktrees.database]", "[on_change]",
    "[[actions]]",
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
    for action in config.actions {
      var lines = ["[[actions]]", "name = \(string(action.name))", "command = \(string(action.command))"]
      if let cwd = action.cwd { lines.append("cwd = \(string(cwd))") }
      if let open = action.open { lines.append("open = \(string(open))") }
      sections.append(lines.joined(separator: "\n"))
    }
    return sections.isEmpty ? "" : sections.joined(separator: "\n\n") + "\n"
  }

  /// `existing` with the managed sections replaced by `config`'s, where
  /// the first of them was; the rest of the file stays as written.
  public static func merging(_ config: ProjectConfig, into existing: String) -> String {
    let blocks = split(existing)
    var kept: [String] = []
    var insertAt: Int?
    for block in blocks {
      if let header = block.header, managedHeaders.contains(header) {
        if insertAt == nil { insertAt = kept.count }
      } else {
        kept.append(block.text.trimmingCharacters(in: .newlines))
      }
    }
    let managed = text(config).trimmingCharacters(in: .newlines)
    var parts = kept.filter { !$0.isEmpty }
    if !managed.isEmpty {
      let index = min(insertAt ?? parts.count, parts.count)
      parts.insert(managed, at: index)
    }
    return parts.isEmpty ? "" : parts.joined(separator: "\n\n") + "\n"
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

  /// The file in blocks: what comes before the first header, then each
  /// header with the lines under it.
  private static func split(_ text: String) -> [(header: String?, text: String)] {
    var blocks: [(header: String?, text: String)] = [(nil, "")]
    for line in text.components(separatedBy: "\n") {
      let trimmed = line.trimmingCharacters(in: .whitespaces)
      if trimmed.hasPrefix("["), let close = trimmed.range(of: "]]") ?? trimmed.range(of: "]") {
        let header = String(trimmed[..<close.upperBound]).replacingOccurrences(of: " ", with: "")
        blocks.append((header, line))
      } else {
        blocks[blocks.count - 1].text += (blocks[blocks.count - 1].text.isEmpty ? "" : "\n") + line
      }
    }
    return blocks
  }

  /// A TOML basic string.
  static func string(_ value: String) -> String {
    var escaped = ""
    for character in value {
      switch character {
      case "\\": escaped += "\\\\"
      case "\"": escaped += "\\\""
      case "\n": escaped += "\\n"
      case "\t": escaped += "\\t"
      default: escaped.append(character)
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
