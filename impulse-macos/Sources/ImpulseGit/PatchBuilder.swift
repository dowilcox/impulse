// PatchBuilder — turns one file's unified diff plus a hunk/line selection into
// a smaller patch for `git apply`. This is how hunk and line staging,
// unstaging and reverting work: the selected changes stay changes, the rest
// of the hunk is neutralised.
//
// Pure text processing (no repository access); tested by round-tripping the
// result through the real `git apply`.

import Foundation

/// Which parts of a file's diff to act on.
public struct PatchSelection: Equatable, Sendable {
  /// Hunk index → selected line indices within the hunk (indices count every
  /// context/added/removed line, but not "\ No newline at end of file"
  /// markers). A nil set selects the whole hunk.
  public var hunks: [Int: Set<Int>?]

  public init(hunks: [Int: Set<Int>?]) {
    self.hunks = hunks
  }

  public static func wholeHunks(_ indices: [Int]) -> PatchSelection {
    PatchSelection(hunks: Dictionary(uniqueKeysWithValues: indices.map { ($0, nil) }))
  }

  public static func lines(_ indices: Set<Int>, inHunk hunk: Int) -> PatchSelection {
    PatchSelection(hunks: [hunk: indices])
  }
}

public enum PatchBuilder {
  public enum BuildError: Error, Equatable, CustomStringConvertible {
    case nothingSelected
    case partialAddOrDelete
    case partialRename
    case malformedPatch
    /// The file isn't UTF-8 text, so it can't be split byte-exactly.
    case notUTF8

    public var description: String {
      switch self {
      case .nothingSelected: return "No changes selected."
      case .partialAddOrDelete:
        return "Part of a new or deleted file can't be staged on its own; select the whole file."
      case .partialRename:
        return "Part of a renamed file can't be staged or unstaged on its own (it would undo the rename); select the whole file."
      case .malformedPatch: return "The diff couldn't be read."
      case .notUTF8:
        return "This file isn't UTF-8 text, so its hunks can't be staged, unstaged or reverted separately. Use the whole file instead."
      }
    }
  }

  struct Line {
    var kind: Character  // " ", "+", "-"
    var text: String
    /// Followed by "\ No newline at end of file".
    var noNewline: Bool
  }

  struct Hunk {
    var oldStart: Int
    var oldCount: Int
    var newStart: Int
    var newCount: Int
    /// Text after the closing "@@" (function context), including its space.
    var section: String
    var lines: [Line]
  }

  struct Parsed {
    var header: [String]
    var hunks: [Hunk]
    var isNewFile: Bool { header.contains { $0.hasPrefix("new file mode") || $0 == "--- /dev/null" } }
    var isDeletedFile: Bool {
      header.contains { $0.hasPrefix("deleted file mode") || $0 == "+++ /dev/null" }
    }
    var isRenameOrCopy: Bool {
      header.contains { $0.hasPrefix("rename from ") || $0.hasPrefix("copy from ") }
    }
  }

  /// Build a patch containing only the selected changes.
  ///
  /// - Parameter reverse: build for `git apply --reverse` (unstaging from a
  ///   staged diff, or reverting working-tree changes). Unselected additions
  ///   then become context and unselected removals are dropped; forward
  ///   patches do the opposite.
  public static func build(patch: String, selection: PatchSelection, reverse: Bool) throws
    -> String
  {
    guard let parsed = parse(patch) else { throw BuildError.malformedPatch }
    guard !selection.hunks.isEmpty else { throw BuildError.nothingSelected }

    // A patch with only some of a renamed file's changes keeps the rename
    // header, so applying it would move the rest back to the old path and
    // leave the new one untracked: such a file is taken whole or not at all.
    if parsed.isRenameOrCopy {
      let whole = parsed.hunks.indices.allSatisfy { index in
        guard let chosen = selection.hunks[index] else { return false }
        guard let lines = chosen else { return true }
        return parsed.hunks[index].lines.indices.allSatisfy { parsed.hunks[index].lines[$0].kind == " " || lines.contains($0) }
      }
      if !whole { throw BuildError.partialRename }
    }

    var output = parsed.header
    var offset = 0
    var emitted = 0
    for (index, hunk) in parsed.hunks.enumerated() {
      guard let lineSelection = selection.hunks[index] else { continue }
      let partial: Bool
      if let chosen = lineSelection {
        let changed = hunk.lines.indices.filter { hunk.lines[$0].kind != " " }
        partial = !changed.allSatisfy { chosen.contains($0) }
      } else {
        partial = false
      }
      if partial && (parsed.isNewFile || parsed.isDeletedFile) {
        throw BuildError.partialAddOrDelete
      }


      var lines: [Line] = []
      for (lineIndex, line) in hunk.lines.enumerated() {
        let selected = lineSelection?.contains(lineIndex) ?? true
        switch line.kind {
        case "+":
          if selected {
            lines.append(line)
          } else if reverse {
            lines.append(Line(kind: " ", text: line.text, noNewline: line.noNewline))
          }
        case "-":
          if selected {
            lines.append(line)
          } else if !reverse {
            lines.append(Line(kind: " ", text: line.text, noNewline: line.noNewline))
          }
        default:
          lines.append(line)
        }
      }
      guard lines.contains(where: { $0.kind != " " }) else { continue }

      let oldCount = lines.filter { $0.kind != "+" }.count
      let newCount = lines.filter { $0.kind != "-" }.count
      // git searches for each hunk starting at its post-image position in
      // the file as already changed by the hunks before it. Forward, that's
      // the old position shifted by the emitted hunks. Reversed (the sides
      // swap), the target is the diff's new side: start from the hunk's new
      // position, shifted by the emitted hunks being undone — never from
      // the old position, which is off by every unselected earlier hunk and
      // can land the change in an identical block elsewhere.
      // A hunk with nothing to match (a zero-context insertion, or reverting
      // a zero-context deletion) goes exactly there, and an empty range's
      // start names the line before it: insert after that line.
      let oldStart: Int
      let newStart: Int
      if reverse {
        newStart = hunk.newStart
        if newCount == 0 {
          oldStart = hunk.newStart + 1 + offset
        } else {
          oldStart = oldCount == 0 ? max(hunk.newStart - 1 + offset, 0) : hunk.newStart + offset
        }
        offset += oldCount - newCount
      } else {
        oldStart = hunk.oldStart
        newStart = oldCount == 0 ? hunk.oldStart + 1 + offset : oldStart + offset
        offset += newCount - oldCount
      }
      output.append(
        "@@ -\(range(oldStart, oldCount)) +\(range(newStart, newCount)) @@\(hunk.section)")
      for line in lines {
        output.append(String(line.kind) + line.text)
        if line.noNewline { output.append("\\ No newline at end of file") }
      }
      emitted += 1
    }
    guard emitted > 0 else { throw BuildError.nothingSelected }
    return output.joined(separator: "\n") + "\n"
  }

  private static func range(_ start: Int, _ count: Int) -> String {
    count == 1 ? "\(start)" : "\(start),\(count)"
  }

  // MARK: - Parsing

  static func parse(_ patch: String) -> Parsed? {
    var lines = patch.components(separatedBy: "\n")
    if lines.last == "" { lines.removeLast() }
    var header: [String] = []
    var hunks: [Hunk] = []
    var index = 0
    while index < lines.count, !lines[index].hasPrefix("@@") {
      header.append(lines[index])
      index += 1
    }
    while index < lines.count {
      guard let hunk = parseHunkHeader(lines[index]) else { return nil }
      var current = hunk
      index += 1
      while index < lines.count, !lines[index].hasPrefix("@@") {
        let raw = lines[index]
        if raw.hasPrefix("\\") {
          // "\ No newline at end of file" belongs to the previous line.
          if !current.lines.isEmpty { current.lines[current.lines.count - 1].noNewline = true }
        } else if let first = raw.first, first == " " || first == "+" || first == "-" {
          current.lines.append(Line(kind: first, text: String(raw.dropFirst()), noNewline: false))
        } else if raw.isEmpty {
          // Some tools drop the leading space of empty context lines.
          current.lines.append(Line(kind: " ", text: "", noNewline: false))
        } else {
          return nil
        }
        index += 1
      }
      hunks.append(current)
    }
    return Parsed(header: header, hunks: hunks)
  }

  /// "@@ -a,b +c,d @@ section" → counts default to 1 when omitted.
  static func parseHunkHeader(_ line: String) -> Hunk? {
    guard line.hasPrefix("@@ -") else { return nil }
    let afterAt = line.dropFirst(3)
    guard let close = afterAt.range(of: " @@") else { return nil }
    let ranges = afterAt[afterAt.startIndex..<close.lowerBound].split(separator: " ")
    guard ranges.count == 2, ranges[0].hasPrefix("-"), ranges[1].hasPrefix("+") else {
      return nil
    }
    func parseRange(_ text: Substring) -> (Int, Int)? {
      let parts = text.dropFirst().split(separator: ",", omittingEmptySubsequences: false)
      guard let start = Int(parts[0]) else { return nil }
      let count = parts.count > 1 ? Int(parts[1]) : 1
      guard let count else { return nil }
      return (start, count)
    }
    guard let old = parseRange(ranges[0]), let new = parseRange(ranges[1]) else { return nil }
    let section = String(afterAt[close.upperBound...])
    return Hunk(
      oldStart: old.0, oldCount: old.1, newStart: new.0, newCount: new.1, section: section,
      lines: [])
  }
}
