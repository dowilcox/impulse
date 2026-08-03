// Port of `file_hunks` from impulse-core/src/git.rs: unified-diff hunks for a
// single repo-relative file (HEAD vs index + working tree), with size guards
// and intra-line word-diff spans.

import Clibgit2
import Foundation

/// The kind of a single line in a unified diff hunk.
public enum DiffLineKind: String, Codable, Equatable, Sendable {
  case context
  case added
  case removed
}

/// A changed sub-range within a diff line, expressed in UTF-16 code units so it
/// maps directly onto JavaScript string offsets in the WebView renderer.
public struct WordSpan: Codable, Equatable, Hashable, Sendable {
  /// Inclusive start offset (UTF-16 code units).
  public let start: UInt32
  /// Exclusive end offset (UTF-16 code units).
  public let end: UInt32

  public init(start: UInt32, end: UInt32) {
    self.start = start
    self.end = end
  }
}

/// A single line within a `DiffHunk`.
public struct DiffLine: Codable, Equatable, Sendable {
  public let kind: DiffLineKind
  /// 1-based old-file line number (present for context and removed lines).
  public let oldLineno: UInt32?
  /// 1-based new-file line number (present for context and added lines).
  public let newLineno: UInt32?
  /// Line text without the trailing newline.
  public let content: String
  /// Intra-line word-diff ranges for changed lines (empty otherwise).
  public var spans: [WordSpan]

  public init(
    kind: DiffLineKind, oldLineno: UInt32?, newLineno: UInt32?, content: String,
    spans: [WordSpan]
  ) {
    self.kind = kind
    self.oldLineno = oldLineno
    self.newLineno = newLineno
    self.content = content
    self.spans = spans
  }

  enum CodingKeys: String, CodingKey {
    case kind, content, spans
    case oldLineno = "old_lineno"
    case newLineno = "new_lineno"
  }

  public func encode(to encoder: Encoder) throws {
    var container = encoder.container(keyedBy: CodingKeys.self)
    try container.encode(kind, forKey: .kind)
    // Explicit nulls to match serde's serialization of Option::None.
    try container.encode(oldLineno, forKey: .oldLineno)
    try container.encode(newLineno, forKey: .newLineno)
    try container.encode(content, forKey: .content)
    try container.encode(spans, forKey: .spans)
  }
}

/// A contiguous hunk of a unified diff (changed lines plus surrounding context).
public struct DiffHunk: Codable, Equatable, Sendable {
  public let oldStart: UInt32
  public let oldLines: UInt32
  public let newStart: UInt32
  public let newLines: UInt32
  /// The `@@ ... @@` header, including any trailing function-context text.
  public let header: String
  public let lines: [DiffLine]

  public init(
    oldStart: UInt32, oldLines: UInt32, newStart: UInt32, newLines: UInt32, header: String,
    lines: [DiffLine]
  ) {
    self.oldStart = oldStart
    self.oldLines = oldLines
    self.newStart = newStart
    self.newLines = newLines
    self.header = header
    self.lines = lines
  }

  enum CodingKeys: String, CodingKey {
    case header, lines
    case oldStart = "old_start"
    case oldLines = "old_lines"
    case newStart = "new_start"
    case newLines = "new_lines"
  }
}

/// The unified-diff hunks for a single file (HEAD vs index + working tree).
public struct FileHunks: Codable, Equatable, Sendable {
  /// Monaco language id.
  public let language: String
  /// Whether either side is binary/non-UTF-8 (hunks blanked).
  public let isBinary: Bool
  /// Whether the file exceeded a size/complexity guard (hunks blanked).
  public let tooLarge: Bool
  /// Whether the diff was capped (more hunks/lines exist than were emitted).
  public let truncated: Bool
  public let added: UInt32
  public let removed: UInt32
  /// The diff hunks (empty when binary/tooLarge).
  public let hunks: [DiffHunk]

  public init(
    language: String, isBinary: Bool, tooLarge: Bool, truncated: Bool, added: UInt32,
    removed: UInt32, hunks: [DiffHunk]
  ) {
    self.language = language
    self.isBinary = isBinary
    self.tooLarge = tooLarge
    self.truncated = truncated
    self.added = added
    self.removed = removed
    self.hunks = hunks
  }

  enum CodingKeys: String, CodingKey {
    case language, truncated, added, removed, hunks
    case isBinary = "is_binary"
    case tooLarge = "too_large"
  }
}

extension GitClient {
  /// Compute the unified-diff hunks for a single repo-relative `filePath`
  /// (HEAD vs index + working tree). Only changed regions plus a few context
  /// lines are materialized. Nil on error (matching the FFI's null result).
  public static func fileHunks(repoPath: String, filePath: String) -> FileHunks? {
    guard let repo = try? openRepo(at: repoPath) else { return nil }
    guard let workdir = try? repo.workdir() else { return nil }

    // Path-traversal guard: validate lexically (deleted files are valid
    // targets here, so no disk-based validation).
    guard (try? validateRelPathLexically(root: workdir, rel: filePath)) != nil else {
      return nil
    }

    let absolute = joinPath(workdir, filePath)
    let language = languageForPath(absolute)

    func blank(isBinary: Bool, tooLarge: Bool) -> FileHunks {
      FileHunks(
        language: language, isBinary: isBinary, tooLarge: tooLarge, truncated: false,
        added: 0, removed: 0, hunks: [])
    }

    let headTree = repo.headTree()
    defer {
      if let tree = headTree { git_object_free(tree) }
    }

    var options = git_diff_options()
    git_diff_options_init(&options, UInt32(GIT_DIFF_OPTIONS_VERSION))
    options.flags |=
      GIT_DIFF_INCLUDE_UNTRACKED.rawValue | GIT_DIFF_RECURSE_UNTRACKED_DIRS.rawValue
      | GIT_DIFF_SHOW_UNTRACKED_CONTENT.rawValue

    var diffPointer: OpaquePointer?
    guard git_diff_tree_to_workdir_with_index(&diffPointer, repo.raw, headTree, &options) == 0,
      let diff = diffPointer
    else { return nil }
    defer { git_diff_free(diff) }

    // Pair renames so a rename diffs old -> new content instead of all-added.
    _ = git_diff_find_similar(diff, nil)

    // Locate the delta for this path (new side for most, old side for deletions).
    var foundIndex: Int?
    var foundDelta: UnsafePointer<git_diff_delta>?
    for index in 0..<git_diff_num_deltas(diff) {
      guard let delta = git_diff_get_delta(diff, index) else { continue }
      let newPath = delta.pointee.new_file.path.map { String(cString: $0) }
      let oldPath = delta.pointee.old_file.path.map { String(cString: $0) }
      if newPath == filePath || oldPath == filePath {
        foundIndex = index
        foundDelta = delta
        break
      }
    }
    guard let index = foundIndex, let delta = foundDelta else {
      // No delta for this path: nothing changed (or already committed).
      return blank(isBinary: false, tooLarge: false)
    }

    if delta.pointee.flags & GIT_DIFF_FLAG_BINARY.rawValue != 0 {
      return blank(isBinary: true, tooLarge: false)
    }

    // Size guard: skip diffing when either side exceeds the byte limit.
    let worktreeTooBig =
      delta.pointee.new_file.path
      .flatMap { fileSize(joinPath(workdir, String(cString: $0))) }
      .map { $0 > maxDiffContentSize } ?? false
    var blobTooBig = false
    if !oidIsZero(delta.pointee.old_file.id) {
      var oid = delta.pointee.old_file.id
      var blobPointer: OpaquePointer?
      if git_blob_lookup(&blobPointer, repo.raw, &oid) == 0, let blob = blobPointer {
        defer { git_blob_free(blob) }
        blobTooBig = git_blob_rawsize(blob) > maxDiffContentSize
      }
    }
    if worktreeTooBig || blobTooBig {
      return blank(isBinary: false, tooLarge: true)
    }

    var patchPointer: OpaquePointer?
    guard git_patch_from_diff(&patchPointer, diff, index) == 0 else { return nil }
    // libgit2 yields no patch for binary deltas.
    guard let patch = patchPointer else {
      return blank(isBinary: true, tooLarge: false)
    }
    defer { git_patch_free(patch) }
    // The binary flag is only reliable once the patch content is computed.
    if let patchDelta = git_patch_get_delta(patch),
      patchDelta.pointee.flags & GIT_DIFF_FLAG_BINARY.rawValue != 0
    {
      return blank(isBinary: true, tooLarge: false)
    }

    var hunks: [DiffHunk] = []
    var added: UInt32 = 0
    var removed: UInt32 = 0
    var totalLines = 0
    var truncated = false

    outer: for hunkIndex in 0..<git_patch_num_hunks(patch) {
      if hunks.count >= maxDiffHunks || totalLines >= maxDiffTotalLines {
        truncated = true
        break
      }
      var hunkPointer: UnsafePointer<git_diff_hunk>?
      var linesInHunk = 0
      guard git_patch_get_hunk(&hunkPointer, &linesInHunk, patch, hunkIndex) == 0,
        let rawHunk = hunkPointer
      else { return nil }
      let header = hunkHeader(rawHunk.pointee)
      let oldStart = UInt32(max(0, rawHunk.pointee.old_start))
      let oldLines = UInt32(max(0, rawHunk.pointee.old_lines))
      let newStart = UInt32(max(0, rawHunk.pointee.new_start))
      let newLines = UInt32(max(0, rawHunk.pointee.new_lines))

      var lines: [DiffLine] = []
      for lineIndex in 0..<linesInHunk {
        if totalLines >= maxDiffTotalLines {
          truncated = true
          hunks.append(
            DiffHunk(
              oldStart: oldStart, oldLines: oldLines, newStart: newStart,
              newLines: newLines, header: header, lines: lines))
          break outer
        }
        var linePointer: UnsafePointer<git_diff_line>?
        guard git_patch_get_line_in_hunk(&linePointer, patch, hunkIndex, lineIndex) == 0,
          let rawLine = linePointer
        else { return nil }

        let kind: DiffLineKind
        switch UInt8(bitPattern: rawLine.pointee.origin) {
        case UInt8(ascii: "+"), UInt8(ascii: ">"): kind = .added
        case UInt8(ascii: "-"), UInt8(ascii: "<"): kind = .removed
        default: kind = .context
        }
        let content = lineContent(rawLine.pointee)
        // Overlong-line guard: bail out to a too-large placeholder rather than
        // ship a line that would choke the WebView renderer.
        if content.utf8.count > maxDiffLineLength {
          return blank(isBinary: false, tooLarge: true)
        }
        switch kind {
        case .added: added += 1
        case .removed: removed += 1
        case .context: break
        }
        lines.append(
          DiffLine(
            kind: kind,
            oldLineno: rawLine.pointee.old_lineno < 0
              ? nil : UInt32(rawLine.pointee.old_lineno),
            newLineno: rawLine.pointee.new_lineno < 0
              ? nil : UInt32(rawLine.pointee.new_lineno),
            content: content,
            spans: []))
        totalLines += 1
      }
      WordDiff.assignWordSpans(&lines)
      hunks.append(
        DiffHunk(
          oldStart: oldStart, oldLines: oldLines, newStart: newStart, newLines: newLines,
          header: header, lines: lines))
    }

    return FileHunks(
      language: language, isBinary: false, tooLarge: false, truncated: truncated,
      added: added, removed: removed, hunks: hunks)
  }
}

/// The `@@ ... @@` header text of a hunk, with trailing newlines trimmed.
func hunkHeader(_ hunk: git_diff_hunk) -> String {
  var copy = hunk
  let length = min(hunk.header_len, MemoryLayout.size(ofValue: copy.header))
  let bytes: [UInt8] = withUnsafeBytes(of: &copy.header) { raw in
    Array(raw.prefix(length))
  }
  var header = String(decoding: bytes, as: UTF8.self)
  while header.hasSuffix("\n") {
    header.removeLast()
  }
  return header
}

/// Line content as lossy UTF-8 with the trailing newline (and a preceding CR)
/// stripped, matching the Rust port.
func lineContent(_ line: git_diff_line) -> String {
  guard let pointer = line.content else { return "" }
  let buffer = UnsafeRawBufferPointer(start: pointer, count: line.content_len)
  var content = String(decoding: buffer, as: UTF8.self)
  if content.hasSuffix("\n") {
    content.removeLast()
    if content.hasSuffix("\r") {
      content.removeLast()
    }
  }
  return content
}
