// LSP `WorkspaceEdit`s (from rename, code actions and `workspace/applyEdit`)
// parsed into plain values, and text edits applied to a string. Positions are
// LSP's: zero-based lines and UTF-16 code-unit columns.

import Foundation

public struct LSPTextEdit: Equatable, Sendable {
  public var startLine: Int
  public var startCharacter: Int
  public var endLine: Int
  public var endCharacter: Int
  public var newText: String

  public init(startLine: Int, startCharacter: Int, endLine: Int, endCharacter: Int, newText: String) {
    self.startLine = startLine
    self.startCharacter = startCharacter
    self.endLine = endLine
    self.endCharacter = endCharacter
    self.newText = newText
  }

  /// `{range: {start, end}, newText}` (also an AnnotatedTextEdit or a
  /// snippet edit's plain text).
  public init?(json: Any?) {
    guard let object = json as? [String: Any],
      let range = object["range"] as? [String: Any],
      let start = range["start"] as? [String: Any],
      let end = range["end"] as? [String: Any],
      let startLine = (start["line"] as? NSNumber)?.intValue,
      let startCharacter = (start["character"] as? NSNumber)?.intValue,
      let endLine = (end["line"] as? NSNumber)?.intValue,
      let endCharacter = (end["character"] as? NSNumber)?.intValue
    else { return nil }
    let text: String
    if let plain = object["newText"] as? String {
      text = plain
    } else if let snippet = object["snippet"] as? [String: Any], let value = snippet["value"] as? String {
      text = value
    } else {
      return nil
    }
    self.init(
      startLine: startLine, startCharacter: startCharacter, endLine: endLine, endCharacter: endCharacter,
      newText: text)
  }
}

public enum WorkspaceEditOperation: Equatable, Sendable {
  case edit(uri: String, edits: [LSPTextEdit])
  case create(uri: String, overwrite: Bool, ignoreIfExists: Bool)
  case rename(oldUri: String, newUri: String, overwrite: Bool, ignoreIfExists: Bool)
  case delete(uri: String, recursive: Bool, ignoreIfNotExists: Bool)
}

public struct WorkspaceEdit: Equatable, Sendable {
  public var operations: [WorkspaceEditOperation]
  /// The document versions the server computed its edits against, by URI
  /// (`documentChanges` only; none means any version). An open document at
  /// another version has changed since: the edits would land in the wrong
  /// places.
  public var versions: [String: Int32]

  public init(operations: [WorkspaceEditOperation], versions: [String: Int32] = [:]) {
    self.operations = operations
    self.versions = versions
  }

  /// `documentChanges` (in order) when present, else `changes`.
  public static func parse(_ json: Any?) -> WorkspaceEdit? {
    guard let object = json as? [String: Any] else { return nil }
    var operations: [WorkspaceEditOperation] = []
    var versions: [String: Int32] = [:]
    if let documentChanges = object["documentChanges"] as? [Any] {
      for case let change as [String: Any] in documentChanges {
        let options = change["options"] as? [String: Any]
        let overwrite = options?["overwrite"] as? Bool ?? false
        switch change["kind"] as? String {
        case "create":
          guard let uri = change["uri"] as? String else { continue }
          operations.append(
            .create(uri: uri, overwrite: overwrite, ignoreIfExists: options?["ignoreIfExists"] as? Bool ?? false))
        case "rename":
          guard let old = change["oldUri"] as? String, let new = change["newUri"] as? String else { continue }
          operations.append(
            .rename(
              oldUri: old, newUri: new, overwrite: overwrite,
              ignoreIfExists: options?["ignoreIfExists"] as? Bool ?? false))
        case "delete":
          guard let uri = change["uri"] as? String else { continue }
          operations.append(
            .delete(
              uri: uri, recursive: options?["recursive"] as? Bool ?? false,
              ignoreIfNotExists: options?["ignoreIfNotExists"] as? Bool ?? false))
        default:
          guard let document = change["textDocument"] as? [String: Any],
            let uri = document["uri"] as? String,
            let edits = change["edits"] as? [Any]
          else { continue }
          operations.append(.edit(uri: uri, edits: edits.compactMap(LSPTextEdit.init(json:))))
          if let version = (document["version"] as? NSNumber)?.int32Value { versions[uri] = version }
        }
      }
    } else if let changes = object["changes"] as? [String: Any] {
      // A JSON object has no order; sort for a stable result.
      for uri in changes.keys.sorted() {
        guard let edits = changes[uri] as? [Any] else { continue }
        operations.append(.edit(uri: uri, edits: edits.compactMap(LSPTextEdit.init(json:))))
      }
    } else {
      return WorkspaceEdit(operations: [])
    }
    return WorkspaceEdit(operations: operations, versions: versions)
  }

  /// Every file the edit touches, in order, without repeats.
  public var uris: [String] {
    var seen = Set<String>()
    var out: [String] = []
    for operation in operations {
      let touched: [String]
      switch operation {
      case .edit(let uri, _), .create(let uri, _, _), .delete(let uri, _, _): touched = [uri]
      case .rename(let old, let new, _, _): touched = [old, new]
      }
      for uri in touched where seen.insert(uri).inserted { out.append(uri) }
    }
    return out
  }

  /// Text edits only, grouped by file (several entries for one file merge).
  public var textEdits: [(uri: String, edits: [LSPTextEdit])] {
    var order: [String] = []
    var byUri: [String: [LSPTextEdit]] = [:]
    for case .edit(let uri, let edits) in operations {
      if byUri[uri] == nil { order.append(uri) }
      byUri[uri, default: []] += edits
    }
    return order.map { ($0, byUri[$0] ?? []) }
  }

  public var hasResourceOperations: Bool {
    operations.contains {
      if case .edit = $0 { return false }
      return true
    }
  }
}

public enum TextEditApplier {
  /// `text` with `edits` applied, as LSP specifies: all ranges refer to the
  /// original text, and inserts at the same position keep their order.
  /// Positions past the end of a line or the document clamp to it. Nil when
  /// two edits overlap.
  public static func apply(_ edits: [LSPTextEdit], to text: String) -> String? {
    let source = text as NSString
    let lineStarts = lineStartOffsets(source)
    func offset(_ line: Int, _ character: Int) -> Int {
      guard line >= 0 else { return 0 }
      guard line < lineStarts.count else { return source.length }
      let start = lineStarts[line]
      let end = line + 1 < lineStarts.count ? contentEnd(source, from: start, to: lineStarts[line + 1]) : source.length
      return min(start + max(0, character), end)
    }
    let ranges = edits.enumerated().map { index, edit -> (start: Int, end: Int, index: Int) in
      let start = offset(edit.startLine, edit.startCharacter)
      let end = max(start, offset(edit.endLine, edit.endCharacter))
      return (start, end, index)
    }
    let ordered = ranges.sorted { ($0.start, $0.end, $0.index) < ($1.start, $1.end, $1.index) }
    for (previous, next) in zip(ordered, ordered.dropFirst()) where next.start < previous.end {
      return nil
    }
    let result = NSMutableString(string: text)
    // Back to front so earlier offsets stay valid; same-position inserts in
    // reverse so they end up in their original order.
    for range in ordered.reversed() {
      result.replaceCharacters(
        in: NSRange(location: range.start, length: range.end - range.start), with: edits[range.index].newText)
    }
    return result as String
  }

  /// UTF-16 offset of the start of every line (`\n`, `\r\n` or `\r`).
  static func lineStartOffsets(_ text: NSString) -> [Int] {
    var starts = [0]
    var index = 0
    let length = text.length
    while index < length {
      let unit = text.character(at: index)
      if unit == 0x0A {
        starts.append(index + 1)
      } else if unit == 0x0D {
        if index + 1 < length, text.character(at: index + 1) == 0x0A { index += 1 }
        starts.append(index + 1)
      }
      index += 1
    }
    return starts
  }

  /// End of a line's content (before its line break).
  private static func contentEnd(_ text: NSString, from start: Int, to nextStart: Int) -> Int {
    var end = nextStart
    if end > start, text.character(at: end - 1) == 0x0A { end -= 1 }
    if end > start, text.character(at: end - 1) == 0x0D { end -= 1 }
    return end
  }
}
