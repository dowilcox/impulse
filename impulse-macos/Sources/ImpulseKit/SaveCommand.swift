import Foundation

/// The programs Impulse runs after saving a file: a file type's formatter
/// and the commands on save.
public enum SaveCommand {
  /// The placeholder replaced with the saved file's path in arguments.
  public static let filePlaceholder = "{file}"

  /// `args` with every `{file}` replaced by `path` (also inside a longer
  /// argument, such as `--stdin-filepath={file}`).
  public static func expandArguments(_ args: [String], file path: String) -> [String] {
    args.map { $0.replacingOccurrences(of: filePlaceholder, with: path) }
  }

  /// Why a run failed, in one line: the first non-blank line of its error
  /// output, else of its output, else its exit status. Long lines are cut.
  public static func failureSummary(status: Int32, stdout: Data, stderr: Data, timedOut: Bool) -> String {
    if timedOut { return "It didn't finish in time and was stopped." }
    for data in [stderr, stdout] {
      let text = String(decoding: data, as: UTF8.self)
      if let line = text.split(whereSeparator: \.isNewline)
        .map({ $0.trimmingCharacters(in: .whitespaces) })
        .first(where: { !$0.isEmpty })
      {
        return line.count > maxSummaryLength ? String(line.prefix(maxSummaryLength - 1)) + "…" : line
      }
    }
    return "It exited with status \(status)."
  }

  static let maxSummaryLength = 200

  /// Whether the editor should show what a formatter or command on save
  /// left in the file (`rewritten`) in place of what it saved: only when it
  /// succeeded, and not when it emptied a file that had text (a tool that
  /// died mid-write, or printed its result instead of writing it).
  public static func shouldAdopt(rewritten: String, saved: String, succeeded: Bool) -> Bool {
    guard succeeded else { return false }
    let blank = { (text: String) in text.allSatisfy(\.isWhitespace) }
    return !blank(rewritten) || blank(saved)
  }

  /// What the editor does once a formatter or command on save finished.
  public enum RewriteOutcome: Equatable, Sendable {
    /// Show the file's new text as the buffer (one undoable edit).
    case adopt
    /// The file holds what was saved, or the command's good result:
    /// remember it as what's on disk. `dirty` when the buffer isn't that.
    case record(dirty: Bool)
    /// The file holds something nobody can vouch for: what a failed or
    /// emptying run left, or another program's write while it ran. Keep
    /// what the save knew about the disk, so the next save asks before
    /// replacing it; the buffer isn't what's on disk.
    case keepSavedState
  }

  /// `saved` is what the save wrote, `disk` what the file holds now and
  /// `buffer` what the editor holds.
  public static func rewriteOutcome(saved: String, disk: String, buffer: String, succeeded: Bool)
    -> RewriteOutcome
  {
    if disk == saved { return .record(dirty: buffer != disk) }
    guard shouldAdopt(rewritten: disk, saved: saved, succeeded: succeeded) else { return .keepSavedState }
    // Typing since the save is kept (unsaved) instead of the result.
    return buffer == saved ? .adopt : .record(dirty: buffer != disk)
  }
}
