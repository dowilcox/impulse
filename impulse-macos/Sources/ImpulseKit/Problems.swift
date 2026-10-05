// Diagnostics from every language server, across files: the Problems
// panel's model (filter, group, count) and the prompt it hands an agent.

import Foundation

public struct Problem: Equatable, Hashable, Sendable {
  public enum Severity: Int, Comparable, Sendable, CaseIterable {
    case error = 1, warning = 2, info = 3, hint = 4

    public static func < (a: Severity, b: Severity) -> Bool { a.rawValue < b.rawValue }

    public var label: String {
      switch self {
      case .error: return "error"
      case .warning: return "warning"
      case .info: return "info"
      case .hint: return "hint"
      }
    }
  }

  public let path: String
  /// 1-based.
  public let line: Int
  public let column: Int
  public let severity: Severity
  public let message: String
  public let source: String?
  public let code: String?

  public init(
    path: String, line: Int, column: Int, severity: Severity, message: String, source: String? = nil,
    code: String? = nil
  ) {
    self.path = path
    self.line = line
    self.column = column
    self.severity = severity
    self.message = message
    self.source = source
    self.code = code
  }
}

public struct ProblemCounts: Equatable, Sendable {
  public var errors = 0
  public var warnings = 0
  public var others = 0

  public init(errors: Int = 0, warnings: Int = 0, others: Int = 0) {
    self.errors = errors
    self.warnings = warnings
    self.others = others
  }

  public var total: Int { errors + warnings + others }
}

public enum Problems {
  public static func counts(_ problems: [Problem]) -> ProblemCounts {
    var counts = ProblemCounts()
    for problem in problems {
      switch problem.severity {
      case .error: counts.errors += 1
      case .warning: counts.warnings += 1
      case .info, .hint: counts.others += 1
      }
    }
    return counts
  }

  /// Problems with one of `severities` whose message, source, code or path
  /// contains `text` (case-insensitive).
  public static func filter(_ problems: [Problem], severities: Set<Problem.Severity>, text: String) -> [Problem] {
    let query = text.trimmingCharacters(in: .whitespaces).lowercased()
    return problems.filter { problem in
      guard severities.contains(problem.severity) else { return false }
      guard !query.isEmpty else { return true }
      return problem.message.lowercased().contains(query) || problem.path.lowercased().contains(query)
        || (problem.source?.lowercased().contains(query) ?? false)
        || (problem.code?.lowercased().contains(query) ?? false)
    }
  }

  /// By file — files with errors first, then by path — each sorted by
  /// position.
  public static func grouped(_ problems: [Problem]) -> [(path: String, problems: [Problem])] {
    let byPath = Dictionary(grouping: problems, by: \.path)
    return byPath.map { path, list in
      (path, list.sorted { ($0.line, $0.column, $0.severity) < ($1.line, $1.column, $1.severity) })
    }
    .sorted { a, b in
      let aWorst = a.problems.map(\.severity).min() ?? .hint
      let bWorst = b.problems.map(\.severity).min() ?? .hint
      return aWorst != bWorst ? aWorst < bWorst : a.path < b.path
    }
  }

  /// A prompt asking an agent to fix these, paths relative to `root`.
  public static func prompt(_ problems: [Problem], root: String?, limit: Int = 100) -> String {
    guard !problems.isEmpty else { return "" }
    func relative(_ path: String) -> String {
      guard let root, path.hasPrefix(root + "/") else { return path }
      return String(path.dropFirst(root.count + 1))
    }
    let shown = grouped(problems).flatMap(\.problems).prefix(limit)
    var out = "Please fix these problems reported by the language servers:\n\n"
    for problem in shown {
      var line = "- \(relative(problem.path)):\(problem.line):\(problem.column) \(problem.severity.label)"
      if let source = problem.source, !source.isEmpty {
        line += " [\(source)\(problem.code.map { " \($0)" } ?? "")]"
      }
      out += line + ": " + problem.message.replacingOccurrences(of: "\n", with: " ") + "\n"
    }
    if problems.count > shown.count {
      out += "\n…and \(problems.count - shown.count) more.\n"
    }
    return out
  }
}
