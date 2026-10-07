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
}
