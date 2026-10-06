import AppKit
import ImpulseGit
import ImpulseKit

/// What the review shows for one changed file, and its view state.
final class ReviewFile {
  var change: FileChange
  var path: String { change.path }
  var viewed = false
  /// Viewed earlier, but the diff changed since.
  var changedSinceViewed = false
  var expanded = true
  var diff: FileDiff?
  var error: String?
  /// A diff request is out.
  var loading = false
  /// The file list was re-read since `diff`: read it again when shown.
  var stale = true
  /// A stage/revert is running on it.
  var busy = false
  /// This file's comments, and which of them no longer match their lines.
  var comments: [ReviewComment] = []
  var outdated: Set<String> = []
  var selection: LineSelection?
  var composer: Composer?
  /// The comment being edited inline.
  var editingComment: String?
  /// The inline edit's text so far.
  var editingDraft: String?
  /// Syntax token spans per hunk and line (nil until highlighted).
  var syntax: [[[SyntaxHighlighter.Span]]]?

  /// Changed lines picked in one hunk (gutter clicks), for staging part of
  /// it or commenting on a range.
  struct LineSelection: Equatable {
    var hunk: Int
    var lines: Set<Int>
    var anchor: Int
  }

  /// A comment being written, anchored after one line of a hunk.
  struct Composer: Equatable {
    var hunk: Int
    /// The line the composer shows below.
    var lineIndex: Int
    var side: ReviewComment.Side
    var line: Int
    var endLine: Int
    var snippet: String
    /// What's typed so far (kept here so a rebuilt row keeps it).
    var draft = ""
  }

  init(change: FileChange) {
    self.change = change
  }
}

enum ReviewLayout: String {
  case unified, split
}

/// Why a file shows a message instead of hunks.
enum ReviewNotice: Hashable {
  case loading, binary, tooLarge, noChanges, truncated
  case error(String)

  var text: String {
    switch self {
    case .loading: return "Loading…"
    case .binary: return "Binary file — no diff shown"
    case .tooLarge: return "File too large to display — open it in the editor"
    case .noChanges: return "No textual changes"
    case .truncated: return "Diff truncated — the file has more changes than shown."
    case .error(let message): return message
    }
  }
}

/// One row of the review's list. Rows are identified by path (and hunk and
/// line indexes), so the scroll position survives reloads.
enum ReviewRow: Hashable {
  case fileHeader(String)
  case notice(String, ReviewNotice)
  case outdatedTitle(String)
  case comment(String, id: String)
  case hunkHeader(String, hunk: Int)
  /// Unified layout: one diff line.
  case line(String, hunk: Int, line: Int)
  /// Split layout: an old-side line beside a new-side line (either may be
  /// missing; context lines are on both sides).
  case split(String, hunk: Int, old: Int?, new: Int?)
  case composer(String)
  /// The bottom edge of a file's card and the gap below it.
  case fileEnd(String)
  /// Space above the first card. A row rather than a scroll inset, so a
  /// file's floating header pins flush to the top and nothing scrolls by
  /// above it.
  case pageTop

  var path: String {
    switch self {
    case .fileHeader(let p), .notice(let p, _), .outdatedTitle(let p), .comment(let p, _),
      .hunkHeader(let p, _), .line(let p, _, _), .split(let p, _, _, _), .composer(let p),
      .fileEnd(let p):
      return p
    case .pageTop:
      return ""
    }
  }

  /// The hunk the row belongs to, for focus and busy styling.
  var hunk: Int? {
    switch self {
    case .hunkHeader(_, let h), .line(_, let h, _), .split(_, let h, _, _): return h
    default: return nil
    }
  }
}

enum ReviewRowBuilder {
  /// The rows for `files` in order.
  static func rows(_ files: [ReviewFile], layout: ReviewLayout) -> [ReviewRow] {
    var rows: [ReviewRow] = []
    for file in files {
      let path = file.path
      rows.append(.fileHeader(path))
      defer { rows.append(.fileEnd(path)) }
      guard file.expanded else { continue }
      guard let diff = file.diff else {
        rows.append(.notice(path, file.error.map(ReviewNotice.error) ?? .loading))
        continue
      }
      if diff.isBinary {
        rows.append(.notice(path, .binary))
        continue
      }
      if diff.tooLarge {
        rows.append(.notice(path, .tooLarge))
        continue
      }
      if diff.hunks.isEmpty {
        rows.append(.notice(path, .noChanges))
        continue
      }
      // Comments whose lines changed float at the top.
      let outdated = file.comments.filter { file.outdated.contains($0.id) }
      if !outdated.isEmpty {
        rows.append(.outdatedTitle(path))
        rows += outdated.map { .comment(path, id: $0.id) }
      }
      let anchored = commentsByEnd(file.comments.filter { !file.outdated.contains($0.id) })
      for (hunkIndex, hunk) in diff.hunks.enumerated() {
        rows.append(.hunkHeader(path, hunk: hunkIndex))
        func extras(after lineIndex: Int) {
          let line = hunk.lines[lineIndex]
          for key in anchorKeys(line) {
            rows += (anchored[key] ?? []).map { .comment(path, id: $0.id) }
          }
          if let composer = file.composer, composer.hunk == hunkIndex, composer.lineIndex == lineIndex {
            rows.append(.composer(path))
          }
        }
        switch layout {
        case .unified:
          for index in hunk.lines.indices {
            rows.append(.line(path, hunk: hunkIndex, line: index))
            extras(after: index)
          }
        case .split:
          for pair in splitPairs(hunk.lines) {
            rows.append(.split(path, hunk: hunkIndex, old: pair.old, new: pair.new))
            if let old = pair.old { extras(after: old) }
            if let new = pair.new, new != pair.old { extras(after: new) }
          }
        }
      }
      if diff.truncated { rows.append(.notice(path, .truncated)) }
    }
    return rows
  }

  /// Context lines on both sides; each run of removed lines paired with the
  /// following run of added lines.
  static func splitPairs(_ lines: [DiffLine]) -> [(old: Int?, new: Int?)] {
    var pairs: [(old: Int?, new: Int?)] = []
    var i = 0
    while i < lines.count {
      if lines[i].kind == .context {
        pairs.append((i, i))
        i += 1
        continue
      }
      var removed: [Int] = []
      var added: [Int] = []
      while i < lines.count, lines[i].kind == .removed {
        removed.append(i)
        i += 1
      }
      while i < lines.count, lines[i].kind == .added {
        added.append(i)
        i += 1
      }
      for k in 0..<max(removed.count, added.count) {
        pairs.append((k < removed.count ? removed[k] : nil, k < added.count ? added[k] : nil))
      }
    }
    return pairs
  }

  /// Where a comment shows: after the last line it covers, on its side.
  static func commentsByEnd(_ comments: [ReviewComment]) -> [String: [ReviewComment]] {
    Dictionary(grouping: comments) { "\($0.side.rawValue):\($0.endLine)" }
  }

  static func anchorKeys(_ line: DiffLine) -> [String] {
    var keys: [String] = []
    if let new = line.newLineno, line.kind != .removed { keys.append("new:\(new)") }
    if let old = line.oldLineno, line.kind == .removed { keys.append("old:\(old)") }
    return keys
  }
}

// MARK: - Metrics

/// Sizes for the review's rows. Code is monospaced and wraps at any
/// character, so a line's height follows from its display width.
struct ReviewMetrics {
  let codeFont: NSFont
  let gutterFont: NSFont
  let charWidth: CGFloat
  let lineHeight: CGFloat
  let tabWidth = 4

  /// Page margin on each side of a file card.
  static let cardInset: CGFloat = 12
  static let gutterWidth: CGFloat = 46
  static let markerWidth: CGFloat = 18
  static let codeTrailing: CGFloat = 12
  static let fileHeaderHeight: CGFloat = 34
  static let hunkHeaderHeight: CGFloat = 26
  static let noticeHeight: CGFloat = 42
  static let outdatedTitleHeight: CGFloat = 26
  static let composerHeight: CGFloat = 136
  static let editingCommentHeight: CGFloat = 132
  static let fileEndHeight: CGFloat = 12
  static let pageTopHeight: CGFloat = 10
  /// Comments sit indented under the gutters.
  static func commentIndent(_ layout: ReviewLayout) -> CGFloat { layout == .split ? 64 : 100 }

  init(fontFamily: String, size: CGFloat = 12) {
    let font =
      NSFont(name: fontFamily, size: size) ?? NSFont(name: "JetBrains Mono", size: size)
      ?? .monospacedSystemFont(ofSize: size, weight: .regular)
    codeFont = font
    gutterFont = NSFont(descriptor: font.fontDescriptor, size: size - 1) ?? font
    charWidth = max(1, ("M" as NSString).size(withAttributes: [.font: font]).width)
    lineHeight = ceil(max(19, font.ascender - font.descender + font.leading + 4))
  }

  /// A code line's own height; the rest of `lineHeight` is split around it.
  var naturalLineHeight: CGFloat { codeFont.ascender - codeFont.descender + codeFont.leading }

  /// Top of the first code line within a row (flipped).
  var codeTop: CGFloat { ((lineHeight - naturalLineHeight) / 2).rounded() }

  /// The first code line's baseline within a row (flipped): gutters and
  /// markers sit on it too.
  var baseline: CGFloat { codeTop + codeFont.ascender }

  /// Width of a code column for a table this wide.
  func codeWidth(tableWidth: CGFloat, layout: ReviewLayout) -> CGFloat {
    let card = max(200, tableWidth - Self.cardInset * 2)
    switch layout {
    case .unified:
      return max(40, card - Self.gutterWidth * 2 - Self.markerWidth - Self.codeTrailing)
    case .split:
      return max(40, card / 2 - Self.gutterWidth - Self.markerWidth - Self.codeTrailing)
    }
  }

  /// Visual lines `text` takes in a code column this wide.
  func wrappedLineCount(_ text: String, width: CGFloat) -> Int {
    let perLine = max(1, Int(width / charWidth))
    let columns = Self.displayColumns(text, tabWidth: tabWidth)
    return max(1, (columns + perLine - 1) / perLine)
  }

  /// Monospaced columns: tabs to the next stop, wide East Asian characters
  /// and emoji two, combining marks none.
  static func displayColumns(_ text: String, tabWidth: Int) -> Int {
    var columns = 0
    for scalar in text.unicodeScalars {
      let v = scalar.value
      if v == 0x09 {
        columns += tabWidth - columns % tabWidth
      } else if scalar.properties.generalCategory == .nonspacingMark
        || scalar.properties.generalCategory == .enclosingMark || v == 0x200D || (0xFE00...0xFE0F).contains(v)
      {
        continue
      } else if isWide(v) {
        columns += 2
      } else {
        columns += 1
      }
    }
    return columns
  }

  private static func isWide(_ v: UInt32) -> Bool {
    (0x1100...0x115F).contains(v) || (0x2E80...0xA4CF).contains(v) || (0xAC00...0xD7A3).contains(v)
      || (0xF900...0xFAFF).contains(v) || (0xFE30...0xFE4F).contains(v) || (0xFF00...0xFF60).contains(v)
      || (0xFFE0...0xFFE6).contains(v) || (0x1F300...0x1F64F).contains(v) || (0x1F900...0x1F9FF).contains(v)
      || (0x20000...0x3FFFD).contains(v)
  }

  /// Height of a comment card holding `text` in a table this wide.
  func commentHeight(_ text: String, tableWidth: CGFloat, layout: ReviewLayout) -> CGFloat {
    let width = max(
      80, tableWidth - Self.cardInset * 2 - Self.commentIndent(layout) - Self.codeTrailing - 26)
    let body = (text as NSString).boundingRect(
      with: NSSize(width: width, height: .greatestFiniteMagnitude),
      options: [.usesLineFragmentOrigin, .usesFontLeading],
      attributes: [.font: NSFont.systemFont(ofSize: 12)]
    ).height
    // Margins 4 + 8, padding 8 + 8, meta line 18.
    return ceil(body) + 46
  }
}
