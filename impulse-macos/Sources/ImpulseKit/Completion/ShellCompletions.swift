// The shell's own completions (opt-in): fish's `complete -C` answers with
// everything its completion scripts know, one `value<TAB>description` per
// line. bash and zsh have no equivalent that works outside an interactive
// session, so they keep the built-in specs.

import Foundation

public enum ShellCompletions {
  /// Whether `shellPath` is a shell this can ask.
  public static func supports(shellPath: String) -> Bool {
    (shellPath as NSString).lastPathComponent == "fish"
  }

  /// Ask fish to complete `line` (the text up to the cursor) in `cwd`.
  /// Empty after `timeout` or on any failure.
  public static func fish(
    line: String, cwd: String?, fishPath: String, timeout: TimeInterval = 1.5
  ) -> [CompletionCandidate] {
    // The line goes in an environment variable, so it's never parsed as code.
    var environment = ProcessInfo.processInfo.environment
    environment["IMPULSE_COMPLETE_LINE"] = line
    // A timeout stops the whole process group, so a completion script's
    // helper that hangs can't outlive the request.
    guard
      let result = try? ChildProcess.run(
        fishPath, ["-c", "complete -C -- \"$IMPULSE_COMPLETE_LINE\""], in: cwd,
        environment: environment, timeout: timeout),
      !result.timedOut
    else { return [] }
    return parseFish(String(decoding: result.stdout, as: UTF8.self))
  }

  /// `value<TAB>description` lines → candidates (kind "shell"; a trailing
  /// `/` marks a directory).
  public static func parseFish(_ output: String, limit: Int = 200) -> [CompletionCandidate] {
    var seen = Set<String>()
    var out: [CompletionCandidate] = []
    for line in output.split(separator: "\n", omittingEmptySubsequences: true) {
      let parts = line.split(separator: "\t", maxSplits: 1, omittingEmptySubsequences: false)
      let value = String(parts[0])
      guard !value.isEmpty, seen.insert(value).inserted else { continue }
      let isDir = value.hasSuffix("/")
      var display = value
      if value.contains("/") {
        let trimmed = isDir ? String(value.dropLast()) : value
        display = (trimmed as NSString).lastPathComponent + (isDir ? "/" : "")
      }
      let detail = parts.count > 1 ? String(parts[1]).trimmingCharacters(in: .whitespaces) : nil
      out.append(
        CompletionCandidate(
          value: value, display: display, kind: isDir ? "path" : "shell", isDir: isDir, gitStatus: nil,
          detail: detail?.isEmpty == false ? detail : nil, keptPrefix: nil))
      if out.count >= limit { break }
    }
    return out
  }

  /// `primary` first (the built-in specs), then what only the shell knew.
  public static func merge(
    _ primary: [CompletionCandidate], _ shell: [CompletionCandidate], limit: Int
  ) -> [CompletionCandidate] {
    var seen = Set(primary.map(\.value))
    var out = primary
    for candidate in shell where seen.insert(candidate.value).inserted {
      out.append(candidate)
    }
    return Array(out.prefix(limit))
  }
}
