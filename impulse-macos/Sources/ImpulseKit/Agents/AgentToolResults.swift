// What Impulse answers agents with, beyond the task listing: the JSON the
// `impulse` commands print with --json and `impulse mcp` returns as a tool's
// structured result, and the text form of each for people and for agents
// that only read text. The app fills them in; they're kept here so their
// shape and wording are tested headless.

import Foundation

/// Encoding shared by every answer: sorted keys, ISO 8601 dates.
public enum AgentToolJSON {
  public static func data<T: Encodable>(_ value: T, pretty: Bool = false) -> Data {
    let encoder = JSONEncoder()
    encoder.outputFormatting = pretty ? [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes] : [.sortedKeys, .withoutEscapingSlashes]
    encoder.dateEncodingStrategy = .iso8601
    return (try? encoder.encode(value)) ?? Data("{}".utf8)
  }

  public static func string<T: Encodable>(_ value: T) -> String {
    String(decoding: data(value, pretty: true), as: UTF8.self)
  }
}

// MARK: - One task

public struct TaskDetail: Codable, Equatable, Sendable {
  public struct Commit: Codable, Equatable, Sendable {
    public var id: String
    public var subject: String
    public init(id: String, subject: String) {
      self.id = id
      self.subject = subject
    }
  }

  public var workspace: TaskListing.Workspace
  /// Files its branch changes against its base.
  public var changedFiles: [String]
  /// Files with uncommitted changes in its folder.
  public var uncommittedFiles: [String]
  /// Its commits that its base lacks, newest first (at most `commitLimit`).
  public var commits: [Commit]
  /// More commits than listed.
  public var moreCommits: Int
  /// Merged into its base (squash merges included); nil for the main
  /// checkout, or when unknown.
  public var merged: Bool?
  public var slot: Int?
  /// Its own port values (`APP_PORT` = 8100), when the project gives tasks
  /// values.
  public var ports: [String: Int]
  /// What it was started with, when an agent's request made it.
  public var brief: String?

  public static let commitLimit = 50

  public init(
    workspace: TaskListing.Workspace, changedFiles: [String] = [], uncommittedFiles: [String] = [],
    commits: [Commit] = [], moreCommits: Int = 0, merged: Bool? = nil, slot: Int? = nil, ports: [String: Int] = [:],
    brief: String? = nil
  ) {
    self.workspace = workspace
    self.changedFiles = changedFiles
    self.uncommittedFiles = uncommittedFiles
    self.commits = commits
    self.moreCommits = moreCommits
    self.merged = merged
    self.slot = slot
    self.ports = ports
    self.brief = brief
  }

  public func text() -> String {
    var lines = TaskListing(repository: "", workspaces: [workspace]).text().components(separatedBy: "\n")
    // Drop the listing's "repo: 1 workspace" header and blank line.
    lines.removeFirst(min(2, lines.count))
    if let merged { lines.append(merged ? "  merged into its base" : "  not merged into its base yet") }
    if let slot {
      var line = "  slot \(slot)"
      if !ports.isEmpty { line += ": " + ports.keys.sorted().map { "\($0) \(ports[$0]!)" }.joined(separator: ", ") }
      lines.append(line)
    }
    if !commits.isEmpty {
      lines.append("")
      lines.append("Commits ahead of its base:")
      lines += commits.map { "  \($0.id.prefix(7)) \($0.subject)" }
      if moreCommits > 0 { lines.append("  …and \(moreCommits) more") }
    }
    if !changedFiles.isEmpty {
      lines.append("")
      lines.append("Changed against its base:")
      lines += changedFiles.map { "  " + $0 }
    }
    if !uncommittedFiles.isEmpty {
      lines.append("")
      lines.append("Uncommitted:")
      lines += uncommittedFiles.map { "  " + $0 }
    }
    if let brief, !brief.isEmpty {
      lines.append("")
      lines.append("Brief:")
      lines += brief.components(separatedBy: "\n").map { "  " + $0 }
    }
    return lines.joined(separator: "\n")
  }
}

// MARK: - Turns

public struct TurnList: Codable, Equatable, Sendable {
  public struct Turn: Codable, Equatable, Sendable {
    public var agent: String
    public var started: Date
    public var ended: Date?
    /// Checkpoint refs: `git diff <start> <end>` is what the turn changed.
    public var start: String
    public var end: String?
    public init(agent: String, started: Date, ended: Date? = nil, start: String, end: String? = nil) {
      self.agent = agent
      self.started = started
      self.ended = ended
      self.start = start
      self.end = end
    }
  }

  public var turns: [Turn]
  public init(turns: [Turn]) { self.turns = turns }

  public func text() -> String {
    guard !turns.isEmpty else { return "No agent turns recorded in this terminal yet." }
    return turns.enumerated().map { index, turn in
      let span = turn.end.map { "\(turn.start) \($0)" } ?? "\(turn.start) (still working)"
      return "\(index + 1). \(turn.agent), \(Self.time(turn.started)): \(span)"
    }.joined(separator: "\n")
  }

  static func time(_ date: Date) -> String {
    let formatter = ISO8601DateFormatter()
    formatter.formatOptions = [.withInternetDateTime]
    return formatter.string(from: date)
  }
}

// MARK: - Review comments

public struct CommentList: Codable, Equatable, Sendable {
  public struct Entry: Codable, Equatable, Sendable {
    public var id: String
    public var path: String
    /// "src/a.ts:12-14".
    public var location: String
    public var side: ReviewComment.Side
    public var line: Int
    public var endLine: Int
    public var snippet: String
    public var text: String
    /// Its lines read differently now (or nil when that can't be told:
    /// a comment on removed lines).
    public var outdated: Bool?
    public var resolution: ReviewComment.Resolution?

    public init(_ comment: ReviewComment, outdated: Bool?) {
      id = comment.id
      path = comment.path
      location = comment.location
      side = comment.side
      line = comment.line
      endLine = comment.endLine
      snippet = comment.snippet
      text = comment.text
      self.outdated = outdated
      resolution = comment.resolution
    }
  }

  public var repository: String
  public var comments: [Entry]

  public init(repository: String, comments: [Entry]) {
    self.repository = repository
    self.comments = comments
  }

  public func text() -> String {
    guard !comments.isEmpty else { return "No review comments in \(repository)." }
    var out = "\(comments.count) review comment\(comments.count == 1 ? "" : "s") in \(repository):\n"
    for comment in comments {
      var title = "\n[\(comment.id)] \(comment.location)"
      if comment.side == .old { title += " (removed lines)" }
      if comment.outdated == true { title += " (outdated)" }
      if let resolution = comment.resolution { title += " (addressed by \(resolution.by))" }
      out += title + "\n"
      if !comment.snippet.isEmpty {
        let fence = ReviewCommentAnchoring.codeFence(for: comment.snippet)
        out += "\(fence)\n\(comment.snippet)\n\(fence)\n"
      }
      out += comment.text.trimmingCharacters(in: .whitespacesAndNewlines) + "\n"
      if let resolution = comment.resolution { out += "Addressed: \(resolution.note)\n" }
    }
    return out
  }
}

/// Whether a comment's lines still read as they did, against a file's
/// current text: only for comments on the new side (the working tree's
/// lines); nil for removed lines.
public enum CommentFreshness {
  public static func isOutdated(_ comment: ReviewComment, currentText: String?) -> Bool? {
    guard comment.side == .new else { return nil }
    guard let currentText else { return true }
    var lines: [Int: String] = [:]
    for (index, line) in currentText.components(separatedBy: "\n").enumerated() { lines[index + 1] = line }
    return ReviewCommentAnchoring.isOutdated(comment, lines: lines)
  }
}

// MARK: - Problems

public struct ProblemList: Codable, Equatable, Sendable {
  public struct Entry: Codable, Equatable, Sendable {
    public var path: String
    public var line: Int
    public var column: Int
    public var severity: String
    public var message: String
    public var source: String?
    public var code: String?
  }

  public struct Counts: Codable, Equatable, Sendable {
    public var errors: Int
    public var warnings: Int
    public var others: Int
  }

  public var problems: [Entry]
  /// Problems left out past the limit.
  public var truncated: Int
  /// Everything reported in the workspace (not only what's listed).
  public var counts: Counts
  /// Files whose diagnostics are current.
  public var checkedFiles: [String]
  /// Why there's nothing to report, when nothing ran.
  public var note: String?

  public static let limit = 200

  /// Problems under `root` (relative paths), worst first, at most `limit`.
  /// `minimum` keeps that severity and worse.
  public init(
    problems all: [Problem], root: String, path: String? = nil, minimum: Problem.Severity? = nil,
    checkedFiles: [String], note: String? = nil
  ) {
    func relative(_ path: String) -> String {
      path.hasPrefix(root + "/") ? String(path.dropFirst(root.count + 1)) : path
    }
    let inside = all.filter { $0.path == root || $0.path.hasPrefix(root + "/") }
    let counted = Problems.counts(inside)
    counts = Counts(errors: counted.errors, warnings: counted.warnings, others: counted.others)
    let chosen = inside.filter { problem in
      (path == nil || problem.path == path) && (minimum.map { problem.severity <= $0 } ?? true)
    }
    let sorted = Problems.grouped(chosen).flatMap(\.problems).sorted { a, b in
      a.severity != b.severity ? a.severity < b.severity : a.path != b.path ? a.path < b.path : a.line < b.line
    }
    problems = sorted.prefix(Self.limit).map {
      Entry(
        path: relative($0.path), line: $0.line, column: $0.column, severity: $0.severity.label, message: $0.message,
        source: $0.source, code: $0.code)
    }
    truncated = max(0, sorted.count - Self.limit)
    self.checkedFiles = checkedFiles.map(relative).sorted()
    self.note = note
  }

  public func text() -> String {
    if problems.isEmpty {
      if let note { return note }
      return checkedFiles.isEmpty
        ? "No language server has checked a file in this workspace yet."
        : "No problems in the \(checkedFiles.count) file\(checkedFiles.count == 1 ? "" : "s") the language servers checked."
    }
    var out = "\(counts.errors) error\(counts.errors == 1 ? "" : "s"), \(counts.warnings) warning\(counts.warnings == 1 ? "" : "s")\n"
    for problem in problems {
      var line = "\(problem.path):\(problem.line):\(problem.column) \(problem.severity)"
      if let source = problem.source, !source.isEmpty { line += " [\(source)\(problem.code.map { " \($0)" } ?? "")]" }
      out += line + ": " + problem.message.replacingOccurrences(of: "\n", with: " ") + "\n"
    }
    if truncated > 0 { out += "…and \(truncated) more.\n" }
    return out
  }
}

// MARK: - Terminal commands

public struct BlockList: Codable, Equatable, Sendable {
  public struct Block: Codable, Equatable, Sendable {
    /// "<terminal>:<block>": what get_block_output takes.
    public var id: String
    public var command: String?
    public var folder: String?
    public var exitCode: Int32?
    public var started: Date
    public var durationMs: UInt64?
    public var running: Bool
    public init(
      id: String, command: String?, folder: String?, exitCode: Int32?, started: Date, durationMs: UInt64?, running: Bool
    ) {
      self.id = id
      self.command = command
      self.folder = folder
      self.exitCode = exitCode
      self.started = started
      self.durationMs = durationMs
      self.running = running
    }
  }

  public struct Terminal: Codable, Equatable, Sendable {
    public var id: String
    public var title: String
    public var folder: String?
    /// The caller's own terminal.
    public var isCaller: Bool
    public var agent: String?
    public var blocks: [Block]
    public init(id: String, title: String, folder: String?, isCaller: Bool, agent: String?, blocks: [Block]) {
      self.id = id
      self.title = title
      self.folder = folder
      self.isCaller = isCaller
      self.agent = agent
      self.blocks = blocks
    }
  }

  public var terminals: [Terminal]
  public init(terminals: [Terminal]) { self.terminals = terminals }

  /// The most recent blocks listed per terminal.
  public static let blocksPerTerminal = 20

  public func text() -> String {
    guard !terminals.isEmpty else { return "No terminals in this workspace." }
    var out: [String] = []
    for terminal in terminals {
      var title = "\(terminal.title) [\(terminal.id)]"
      if terminal.isCaller { title += " ← you" }
      if let agent = terminal.agent { title += " (\(agent))" }
      out.append(title)
      if terminal.blocks.isEmpty { out.append("  no commands yet") }
      for block in terminal.blocks {
        var line = "  \(block.id)  \(block.command ?? "(no command)")"
        if block.running {
          line += "  (running)"
        } else if let code = block.exitCode {
          line += code == 0 ? "  ok" : "  exit \(code)"
        }
        out.append(line)
      }
      out.append("")
    }
    return out.joined(separator: "\n").trimmingCharacters(in: .newlines)
  }

  /// "<terminal>:<block>" → its parts.
  public static func parse(_ id: String) -> (terminal: String, block: UInt64)? {
    let parts = id.split(separator: ":", omittingEmptySubsequences: false)
    guard parts.count == 2, !parts[0].isEmpty, let block = UInt64(parts[1]) else { return nil }
    return (String(parts[0]), block)
  }
}

public struct BlockOutput: Codable, Equatable, Sendable {
  public var id: String
  public var command: String?
  public var folder: String?
  public var exitCode: Int32?
  public var running: Bool
  /// Lines the block's output has, and how many are returned (its last).
  public var totalLines: Int
  public var shownLines: Int
  public var output: String

  public static let defaultLines = 200
  public static let maxLines = 2000

  public init(
    id: String, command: String?, folder: String?, exitCode: Int32?, running: Bool, output: String, lines: Int
  ) {
    self.id = id
    self.command = command
    self.folder = folder
    self.exitCode = exitCode
    self.running = running
    let all = output.trimmingCharacters(in: .newlines).components(separatedBy: "\n")
    let count = output.trimmingCharacters(in: .newlines).isEmpty ? 0 : all.count
    let shown = min(count, max(1, min(Self.maxLines, lines)))
    totalLines = count
    shownLines = shown
    self.output = count == 0 ? "" : all.suffix(shown).joined(separator: "\n")
  }

  public func text() -> String {
    var head = "`\(command ?? "a command")`"
    if let folder { head += " in \(folder)" }
    if running {
      head += " (still running)"
    } else if let exitCode {
      head += exitCode == 0 ? " (succeeded)" : " (exit status \(exitCode))"
    }
    if totalLines > shownLines { head += ", last \(shownLines) of \(totalLines) lines" }
    return output.isEmpty ? head + ": no output." : head + ":\n" + output
  }
}

// MARK: - Unsaved files

public struct UnsavedList: Codable, Equatable, Sendable {
  public struct File: Codable, Equatable, Sendable {
    public var path: String
    /// The file on disk changed too since it was opened or last saved.
    public var changedOnDisk: Bool
    public init(path: String, changedOnDisk: Bool) {
      self.path = path
      self.changedOnDisk = changedOnDisk
    }
  }

  public var files: [File]
  public init(files: [File]) { self.files = files.sorted { $0.path < $1.path } }

  public func text() -> String {
    guard !files.isEmpty else { return "No files with unsaved edits." }
    return "Open with unsaved edits (ask the user to save before changing them):\n"
      + files.map { "  " + $0.path + ($0.changedOnDisk ? " (also changed on disk)" : "") }.joined(separator: "\n")
  }
}

// MARK: - Project settings

public struct ProjectSummary: Codable, Equatable, Sendable {
  public struct Action: Codable, Equatable, Sendable {
    public var name: String
    public var command: String
  }

  public struct Rule: Codable, Equatable, Sendable {
    public var file: String
    public var command: String
  }

  public struct Detected: Codable, Equatable, Sendable {
    public var lockFiles: [String]
    public var install: [String]
    public var composeFile: String?
    public init(lockFiles: [String], install: [String], composeFile: String?) {
      self.lockFiles = lockFiles
      self.install = install
      self.composeFile = composeFile
    }
  }

  /// The settings' commands are listed: every file with commands is
  /// trusted.
  public var trusted: Bool
  /// Settings files found (relative to the repository).
  public var files: [String]
  public var check: String?
  public var setup: String?
  public var archive: String?
  public var actions: [Action]
  public var onChange: [Rule]
  public var envFile: String?
  public var slot: Int?
  public var ports: [String: Int]
  public var detected: Detected
  /// Why commands aren't listed, or that there are no settings.
  public var note: String?

  /// From settings the user trusted (`config`), or only what Impulse found
  /// itself (`config` nil).
  public init(
    config: ProjectConfig?, trusted: Bool, files: [String], slot: Int?, detected: Detected, note: String? = nil
  ) {
    self.trusted = trusted
    self.files = files
    let shown = trusted ? config : nil
    check = shown?.checkScript
    setup = shown?.setupScript
    archive = shown?.archiveScript
    actions = shown?.actions.map { Action(name: $0.name, command: $0.command) } ?? []
    onChange = (shown?.onChange ?? [:]).keys.sorted().map { Rule(file: $0, command: shown!.onChange[$0]!) }
    envFile = config.map(\.envFile)
    self.slot = slot
    if let config, let slot, trusted {
      ports = TaskEnvironment.ports(config.ports, slot: slot, offset: config.portOffset) ?? [:]
    } else {
      ports = [:]
    }
    self.detected = detected
    self.note = note
  }

  public func text() -> String {
    var lines: [String] = []
    if let note { lines.append(note) }
    if let check { lines.append("check: \(check)") }
    if let setup { lines.append("setup: \(setup)") }
    if let archive { lines.append("archive: \(archive)") }
    if !actions.isEmpty {
      lines.append("actions:")
      lines += actions.map { "  \($0.name): \($0.command)" }
    }
    if !onChange.isEmpty {
      lines.append("when these change:")
      lines += onChange.map { "  \($0.file): \($0.command)" }
    }
    if let envFile { lines.append("env file: \(envFile)") }
    if let slot {
      var line = "this task's slot: \(slot)"
      if !ports.isEmpty { line += " (" + ports.keys.sorted().map { "\($0) \(ports[$0]!)" }.joined(separator: ", ") + ")" }
      lines.append(line)
    }
    if !detected.lockFiles.isEmpty {
      lines.append("lock files: \(detected.lockFiles.joined(separator: ", ")) (install: \(detected.install.joined(separator: " && ")))")
    }
    if let compose = detected.composeFile { lines.append("compose file: \(compose)") }
    return lines.isEmpty ? "No project settings." : lines.joined(separator: "\n")
  }
}
