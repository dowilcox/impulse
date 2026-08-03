// Port of `get_line_blame` and `format_timestamp` from impulse-core/src/git.rs.
// The JSON keys of `BlameInfo` match the FFI's camelCase output
// (`author`, `date`, `commitHash`, `summary`).

import Clibgit2
import Foundation

/// Blame information for a single line.
public struct BlameInfo: Codable, Equatable, Sendable {
  public let author: String
  /// "YYYY-MM-DD", formatted in the commit's own timezone.
  public let date: String
  /// Abbreviated (7-char) commit hash.
  public let commitHash: String
  public let summary: String

  public init(author: String, date: String, commitHash: String, summary: String) {
    self.author = author
    self.date = date
    self.commitHash = commitHash
    self.summary = summary
  }
}

extension GitClient {
  /// Blame information for a specific 1-based line in a file, or nil on error
  /// (matching the FFI's null result).
  public static func lineBlame(filePath: String, line: UInt32) -> BlameInfo? {
    guard let repo = try? openRepo(at: filePath) else { return nil }
    guard let workdir = try? repo.workdir() else { return nil }
    guard let relComponents = relativePathComponents(of: filePath, under: workdir),
      !relComponents.isEmpty
    else { return nil }  // "File not in repo"
    let rel = relComponents.joined(separator: "/")

    var options = git_blame_options()
    git_blame_options_init(&options, UInt32(GIT_BLAME_OPTIONS_VERSION))
    var blamePointer: OpaquePointer?
    guard git_blame_file(&blamePointer, repo.raw, rel, &options) == 0,
      let blame = blamePointer
    else { return nil }
    defer { git_blame_free(blame) }

    guard let hunk = git_blame_get_hunk_byline(blame, Int(line)) else { return nil }

    var author = "Unknown"
    var timestamp: Int64 = 0
    var tzOffsetMinutes: Int32 = 0
    if let signature = hunk.pointee.final_signature {
      if let name = signature.pointee.name {
        author = String(cString: name)
      }
      timestamp = signature.pointee.when.time
      tzOffsetMinutes = signature.pointee.when.offset
    }
    let date = formatTimestamp(timestamp, tzOffsetMinutes: tzOffsetMinutes)

    let fullHash = oidHex(hunk.pointee.final_commit_id)
    let shortHash = String(fullHash.prefix(7))

    var summary = ""
    var oid = hunk.pointee.final_commit_id
    var commitPointer: OpaquePointer?
    if git_commit_lookup(&commitPointer, repo.raw, &oid) == 0, let commit = commitPointer {
      defer { git_commit_free(commit) }
      if let text = git_commit_summary(commit) {
        summary = String(cString: text)
      }
    }

    return BlameInfo(author: author, date: date, commitHash: shortHash, summary: summary)
  }
}

/// Format a unix timestamp into "YYYY-MM-DD", applying the commit's timezone
/// offset (minutes). Exact port of the Rust `format_timestamp`.
func formatTimestamp(_ timestamp: Int64, tzOffsetMinutes: Int32) -> String {
  let localTimestamp = timestamp + Int64(tzOffsetMinutes) * 60

  // Handle negative timestamps (pre-epoch) gracefully.
  if localTimestamp < 0 {
    return "1970-01-01"
  }

  let secsPerDay: Int64 = 86_400
  let daysSinceEpoch = localTimestamp / secsPerDay

  var year: Int32 = 1970
  var remainingDays = daysSinceEpoch
  while true {
    let daysInYear: Int64 = isLeapYear(year) ? 366 : 365
    if remainingDays < daysInYear { break }
    remainingDays -= daysInYear
    year += 1
  }

  let daysInMonths: [Int64] =
    isLeapYear(year)
    ? [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    : [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]

  var month = 1
  for days in daysInMonths {
    if remainingDays < days { break }
    remainingDays -= days
    month += 1
  }
  let day = remainingDays + 1

  return String(format: "%d-%02d-%02d", year, month, day)
}

func isLeapYear(_ year: Int32) -> Bool {
  (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}
