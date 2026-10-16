// A language server's completion answer (`CompletionItem[]` or a
// `CompletionList`) as plain values, with what each item inserts, and where,
// worked out the way LSP says. Positions are LSP's: zero-based lines and
// UTF-16 code-unit columns.

import Foundation

public struct LSPRange: Equatable, Sendable {
  public var startLine: Int
  public var startCharacter: Int
  public var endLine: Int
  public var endCharacter: Int

  public init(startLine: Int, startCharacter: Int, endLine: Int, endCharacter: Int) {
    self.startLine = startLine
    self.startCharacter = startCharacter
    self.endLine = endLine
    self.endCharacter = endCharacter
  }

  /// `{start: {line, character}, end: {line, character}}`.
  public init?(json: Any?) {
    guard let object = json as? [String: Any],
      let start = object["start"] as? [String: Any],
      let end = object["end"] as? [String: Any],
      let startLine = (start["line"] as? NSNumber)?.intValue,
      let startCharacter = (start["character"] as? NSNumber)?.intValue,
      let endLine = (end["line"] as? NSNumber)?.intValue,
      let endCharacter = (end["character"] as? NSNumber)?.intValue
    else { return nil }
    self.init(startLine: startLine, startCharacter: startCharacter, endLine: endLine, endCharacter: endCharacter)
  }
}

public struct LSPCompletionItem: Equatable, Sendable {
  public var label: String
  /// LSP's `CompletionItemKind` (1 = text when the server gives none).
  public var kind: Int
  public var detail: String?
  /// What picking it inserts (snippet syntax when `isSnippet`).
  public var text: String
  public var isSnippet: Bool
  /// The text it replaces (one line, up to the cursor or past it); nil: the
  /// word at the cursor, as the editor works it out.
  public var range: LSPRange?
  /// For an insert/replace edit, the longer range replace mode takes
  /// (`range` is the insert one).
  public var replaceRange: LSPRange?
  /// Edits elsewhere in the file (an auto-import), applied with it.
  public var additionalTextEdits: [LSPTextEdit]
  public var sortText: String?
  public var filterText: String?

  public init(
    label: String, kind: Int = 1, detail: String? = nil, text: String, isSnippet: Bool = false,
    range: LSPRange? = nil, replaceRange: LSPRange? = nil, additionalTextEdits: [LSPTextEdit] = [],
    sortText: String? = nil, filterText: String? = nil
  ) {
    self.label = label
    self.kind = kind
    self.detail = detail
    self.text = text
    self.isSnippet = isSnippet
    self.range = range
    self.replaceRange = replaceRange
    self.additionalTextEdits = additionalTextEdits
    self.sortText = sortText
    self.filterText = filterText
  }
}

public enum LSPCompletion {
  /// The items of a completion answer. An item's text and range come from
  /// its `textEdit` (a plain edit, or insert and replace ranges), else from
  /// the list's `itemDefaults.editRange` (with `textEditText`), else from
  /// `insertText` or the label at the word being typed. Its format is its
  /// own `insertTextFormat`, else the list's default.
  public static func parse(_ json: Any?) -> [LSPCompletionItem] {
    let rawItems: [Any]
    var defaults: [String: Any] = [:]
    if let list = json as? [String: Any] {
      rawItems = list["items"] as? [Any] ?? []
      defaults = list["itemDefaults"] as? [String: Any] ?? [:]
    } else {
      rawItems = json as? [Any] ?? []
    }
    let defaultFormat = (defaults["insertTextFormat"] as? NSNumber)?.intValue
    let defaultRanges = defaults["editRange"].flatMap(editRanges)

    return rawItems.compactMap { raw in
      guard let item = raw as? [String: Any], let label = item["label"] as? String else { return nil }
      let insertText = item["insertText"] as? String
      var text = insertText ?? label
      var ranges: (insert: LSPRange, replace: LSPRange?)?
      if let edit = item["textEdit"] as? [String: Any], let newText = edit["newText"] as? String {
        text = newText
        ranges = editRanges(edit)
      } else if let defaultRanges {
        // LSP says the label; servers that also send insertText mean it.
        text = item["textEditText"] as? String ?? insertText ?? label
        ranges = defaultRanges
      }
      let format = (item["insertTextFormat"] as? NSNumber)?.intValue ?? defaultFormat
      return LSPCompletionItem(
        label: label,
        kind: (item["kind"] as? NSNumber)?.intValue ?? 1,
        detail: item["detail"] as? String,
        text: text,
        isSnippet: format == 2,
        range: ranges?.insert,
        replaceRange: ranges?.replace,
        additionalTextEdits: (item["additionalTextEdits"] as? [Any] ?? []).compactMap(LSPTextEdit.init(json:)),
        sortText: item["sortText"] as? String,
        filterText: item["filterText"] as? String)
    }
  }

  /// A `Range`, or an `InsertReplaceEdit`'s (or default edit range's)
  /// `insert` and `replace`. A completion edits one line, and insert must
  /// start where replace does and end within it; ranges that don't are
  /// left out, and the editor uses the word at the cursor.
  private static func editRanges(_ json: Any) -> (insert: LSPRange, replace: LSPRange?)? {
    guard let object = json as? [String: Any] else { return nil }
    if let range = LSPRange(json: object["range"]) ?? LSPRange(json: object) {
      return isOneLine(range) ? (range, nil) : nil
    }
    guard let insert = LSPRange(json: object["insert"]), isOneLine(insert) else { return nil }
    guard let replace = LSPRange(json: object["replace"]), isOneLine(replace),
      replace.startLine == insert.startLine, replace.startCharacter == insert.startCharacter,
      replace.endCharacter >= insert.endCharacter
    else { return (insert, nil) }
    return (insert, replace)
  }

  private static func isOneLine(_ range: LSPRange) -> Bool {
    range.startLine == range.endLine && range.startCharacter <= range.endCharacter && range.startLine >= 0
      && range.startCharacter >= 0
  }
}
