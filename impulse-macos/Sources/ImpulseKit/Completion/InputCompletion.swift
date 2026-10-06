import Foundation

// Ported from impulse-core/src/completion.rs.
//
// Inline command completion for the terminal input bar. Given the current
// input and working directory, `complete` returns the single best completed
// line to show as dimmed ghost text. Sources, in priority order:
//
// 1. Command history — the most recent command that extends the full input
//    (Warp-style autosuggest).
// 2. Context-aware word completion of the token at the cursor, driven by the
//    shell parser: executables on `PATH` (plus a curated common-command list
//    and shell builtins) for the command word, a built-in subcommand/flag
//    table for popular tools, and filesystem entries for path arguments.
//
// The result is the full completed line; the caller renders the suffix after
// what the user has already typed.
//
// String comparisons and ordering deliberately work on UTF-8 bytes to match
// Rust `str` semantics (byte-wise `starts_with`, `Ord`, and `len`).

public enum InputCompletion {
  /// Compute the best inline completion for `input`.
  ///
  /// `history` should be ordered newest-first. Returns the full completed
  /// line (which always starts with `input`), or `nil` when there's no
  /// useful completion.
  public static func complete(input: String, cwd: String?, history: [String]) -> String? {
    if input.isEmpty || input.unicodeScalars.allSatisfy({ $0.properties.isWhitespace }) {
      return nil
    }

    // 1. History continuation — autosuggest the most recent command that
    //    extends the full input verbatim.
    if let found = historyContinuation(input: input, history: history) {
      return found
    }

    // 2. Context-aware word completion.
    let parsed = parseShellInput(input, cursor: input.utf8.count)
    // Splicing raw candidates back into quoted/escaped tokens is ambiguous;
    // history already covers those cases, so bail out.
    if parsed.incomplete {
      return nil
    }
    let comp = parsed.completion
    if comp.prefix.isEmpty {
      return nil
    }

    let cwdPath = cwd.flatMap { $0.isEmpty ? nil : $0 }
    let candidate: String?
    switch comp.kind {
    case .command:
      candidate = completeCommand(comp.prefix)
    case .redirectTarget:
      candidate = completePath(comp.prefix, cwd: cwdPath)
    case .envAssignment:
      candidate = nil
    case .argument:
      candidate = completeArgument(
        command: comp.command,
        argumentIndex: comp.argumentIndex,
        prefix: comp.prefix,
        cwd: cwdPath)
    }
    guard let candidate else { return nil }

    return splice(input, span: comp.span, candidate: candidate)
  }

  /// The most recent command in `history` (newest first) that extends
  /// `input`. Only compares strings, so it's cheap enough per keystroke on
  /// the main thread; the rest of `complete` reads the filesystem.
  public static func historyContinuation(input: String, history: [String]) -> String? {
    guard !input.isEmpty else { return nil }
    return history.first { $0.utf8.count > input.utf8.count && utf8HasPrefix($0, input) }
  }

  /// Eagerly populate the `PATH` executable cache off the hot path, so the
  /// first completion keystroke doesn't pay for the directory scan. Safe to
  /// call from a background thread.
  public static func warmCache() {
    _ = pathExecutables
  }

  // -------------------------------------------------------------------------
  // Multi-candidate completion (dropdown)
  // -------------------------------------------------------------------------

  /// Compute path completion candidates for the active token in `input`.
  ///
  /// Only argument and redirect-target tokens enumerate filesystem
  /// candidates; command words and other token kinds return an empty
  /// candidate list in v1. Candidates are directories-first, then files,
  /// each alphabetical, prefix matched (case-sensitive). Hidden (dot)
  /// entries appear only when the active token's basename starts with `.`.
  /// The list is capped at `limit`.
  ///
  /// `history` is accepted for forward compatibility (history-backed
  /// candidates are not part of v1) and is currently unused.
  public static func completeCandidates(
    input: String, cwd: String?, history: [String], limit: Int
  ) -> CompletionResult {
    _ = history
    let parsed = parseShellInput(input, cursor: input.utf8.count)
    let comp = parsed.completion
    let span = comp.span

    // Splicing into quoted/escaped tokens is ambiguous; don't offer
    // candidates.
    if parsed.incomplete {
      return CompletionResult(span: span, candidates: [])
    }

    // Only path-bearing token kinds enumerate filesystem candidates in v1.
    guard comp.kind == .argument || comp.kind == .redirectTarget else {
      return CompletionResult(span: span, candidates: [])
    }

    let cwdPath = cwd.flatMap { $0.isEmpty ? nil : $0 }
    let (dirPart, _) = splitPathPrefix(comp.prefix)

    var matches = pathMatches(comp.prefix, cwd: cwdPath)
    // Directories first, then files; each group alphabetical by name.
    matches.sort { a, b in
      if a.isDir != b.isDir { return a.isDir }
      return bytewiseLess(a.name, b.name)
    }

    let candidates = matches.prefix(limit).map { match -> CompletionCandidate in
      var value = dirPart + match.name
      if match.isDir {
        value += "/"
      }
      return CompletionCandidate(
        value: value,
        display: match.name,
        kind: "path",
        isDir: match.isDir,
        gitStatus: nil)
    }

    return CompletionResult(span: span, candidates: Array(candidates))
  }

  /// Replace the completion token in `input` with `candidate`, keeping the
  /// text before it verbatim. Returns `nil` unless the result genuinely
  /// extends what was typed (so the ghost suffix stays consistent).
  private static func splice(_ input: String, span: TextSpan, candidate: String) -> String? {
    let inputBytes = Array(input.utf8)
    let start = min(span.start, inputBytes.count)
    var resultBytes = Array(inputBytes[..<start])
    resultBytes.append(contentsOf: candidate.utf8)
    if resultBytes.count > inputBytes.count && resultBytes.starts(with: inputBytes) {
      return String(decoding: resultBytes, as: UTF8.self)
    }
    return nil
  }

  // -------------------------------------------------------------------------
  // Command-word completion
  // -------------------------------------------------------------------------

  private static func completeCommand(_ prefix: String) -> String? {
    // A curated common command makes the single guess stable and useful
    // (e.g. `g` -> `git`, not `gpg`); the list is ordered by commonness.
    if let common = commonCommands.first(where: { utf8HasPrefix($0, prefix) }) {
      return common
    }
    if let builtin = shellBuiltins.first(where: { utf8HasPrefix($0, prefix) }) {
      return builtin
    }
    // Otherwise the shortest matching executable on PATH is a sensible
    // default.
    return
      pathExecutables
      .filter { utf8HasPrefix($0, prefix) }
      .min { a, b in
        if a.utf8.count != b.utf8.count { return a.utf8.count < b.utf8.count }
        return bytewiseLess(a, b)
      }
  }

  /// Cached once per process — installing a new tool mid-session won't
  /// autocomplete until restart, which is an acceptable trade for not
  /// rescanning PATH on every keystroke. `static let` gives the same
  /// thread-safe once-only initialization as the Rust `OnceLock`.
  static let pathExecutables: [String] = scanPathExecutables()

  private static func scanPathExecutables() -> [String] {
    var names = Set<String>()
    guard let path = ProcessInfo.processInfo.environment["PATH"] else {
      return []
    }
    for dir in path.split(separator: ":", omittingEmptySubsequences: false) {
      if dir.isEmpty { continue }
      let dirPath = String(dir)
      guard let entries = try? FileManager.default.contentsOfDirectory(atPath: dirPath) else {
        continue
      }
      for name in entries where isExecutableEntry(directory: dirPath, name: name) {
        names.insert(name)
      }
    }
    return names.sorted { bytewiseLess($0, $1) }
  }

  /// Mirrors the Rust check: regular file or symlink (lstat semantics) with
  /// any execute bit set.
  private static func isExecutableEntry(directory: String, name: String) -> Bool {
    let fullPath = directory + "/" + name
    guard let attrs = try? FileManager.default.attributesOfItem(atPath: fullPath) else {
      return false
    }
    let type = attrs[.type] as? FileAttributeType
    guard type == .typeRegular || type == .typeSymbolicLink else { return false }
    let mode = (attrs[.posixPermissions] as? NSNumber)?.intValue ?? 0
    return mode & 0o111 != 0
  }

  // -------------------------------------------------------------------------
  // Argument completion (subcommands, flags, filesystem paths)
  // -------------------------------------------------------------------------

  private static func completeArgument(
    command: String?, argumentIndex: Int, prefix: String, cwd: String?
  ) -> String? {
    if utf8HasPrefix(prefix, "-") {
      return completeFlag(command: command, prefix: prefix)
    }
    // The first argument after a known command is usually a subcommand.
    if argumentIndex == 0, let command {
      if let sub = completeSubcommand(command: command, prefix: prefix) {
        return sub
      }
    }
    return completePath(prefix, cwd: cwd)
  }

  private static func completeSubcommand(command: String, prefix: String) -> String? {
    let base = commandBasename(command)
    guard let subs = subcommands.first(where: { $0.0 == base })?.1 else { return nil }
    return subs.first { utf8HasPrefix($0, prefix) }
  }

  private static func completeFlag(command: String?, prefix: String) -> String? {
    if let command = command.map(commandBasename) {
      if let commandFlags = flags.first(where: { $0.0 == command })?.1 {
        if let flag = commandFlags.first(where: { utf8HasPrefix($0, prefix) }) {
          return flag
        }
      }
    }
    return commonFlags.first { utf8HasPrefix($0, prefix) }
  }

  /// A filesystem entry that prefix-matches a path token, used by both the
  /// inline ghost completion and the multi-candidate dropdown.
  private struct PathMatch {
    /// The entry's file name (basename) as read from disk.
    var name: String
    var isDir: Bool
  }

  /// Split a path prefix into its directory part (kept verbatim) and the
  /// trailing basename being matched, e.g. `alpha/inn` -> (`alpha/`, `inn`).
  private static func splitPathPrefix(_ prefix: String) -> (String, String) {
    let bytes = Array(prefix.utf8)
    guard let index = bytes.lastIndex(of: UInt8(ascii: "/")) else {
      return ("", prefix)
    }
    let dirPart = String(decoding: bytes[...index], as: UTF8.self)
    let base = String(decoding: bytes[(index + 1)...], as: UTF8.self)
    return (dirPart, base)
  }

  /// Enumerate every filesystem entry in the directory implied by `prefix`
  /// whose name prefix-matches the trailing basename. Hidden (dot) entries
  /// surface only when the user typed a leading dot. Returns an empty array
  /// when the directory can't be resolved or read. Ordering is left to the
  /// caller.
  private static func pathMatches(_ prefix: String, cwd: String?) -> [PathMatch] {
    let (dirPart, base) = splitPathPrefix(prefix)
    guard let searchDir = resolveDir(dirPart, cwd: cwd) else { return [] }
    guard let entries = try? FileManager.default.contentsOfDirectory(atPath: searchDir) else {
      return []
    }

    var matches: [PathMatch] = []
    for name in entries {
      // Hidden entries only surface when the user typed a leading dot.
      if utf8HasPrefix(name, ".") && !utf8HasPrefix(base, ".") {
        continue
      }
      if !utf8HasPrefix(name, base) {
        continue
      }
      let fullPath = searchDir + "/" + name
      // Mirrors Rust `DirEntry::file_type()`: lstat semantics, so a symlink
      // to a directory is not a directory.
      let attrs = try? FileManager.default.attributesOfItem(atPath: fullPath)
      let isDir = (attrs?[.type] as? FileAttributeType) == .typeDirectory
      matches.append(PathMatch(name: name, isDir: isDir))
    }
    return matches
  }

  private static func completePath(_ prefix: String, cwd: String?) -> String? {
    // Keep the directory part exactly as typed; match only the trailing
    // name.
    let (dirPart, _) = splitPathPrefix(prefix)

    // Alphabetically-first match keeps the guess stable across keystrokes.
    guard let best = pathMatches(prefix, cwd: cwd).min(by: { bytewiseLess($0.name, $1.name) })
    else {
      return nil
    }

    var candidate = dirPart + best.name
    if best.isDir {
      candidate += "/"
    }
    return candidate
  }

  private static func resolveDir(_ dirPart: String, cwd: String?) -> String? {
    if utf8HasPrefix(dirPart, "~/") {
      guard let home = ProcessInfo.processInfo.environment["HOME"] else { return nil }
      let rest = String(dirPart.dropFirst(2))
      return rest.isEmpty ? home : home + "/" + rest
    }
    if dirPart == "~" || dirPart == "~/" {
      return ProcessInfo.processInfo.environment["HOME"]
    }
    if utf8HasPrefix(dirPart, "/") {
      return dirPart
    }
    // Relative (including the empty dir part) resolves against the cwd.
    guard let cwd else { return nil }
    return dirPart.isEmpty ? cwd : cwd + "/" + dirPart
  }

  static func commandBasename(_ command: String) -> String {
    command.split(separator: "/", omittingEmptySubsequences: false).last.map(String.init)
      ?? command
  }

  // -------------------------------------------------------------------------
  // Byte-wise string helpers (Rust `str` semantics)
  // -------------------------------------------------------------------------

  private static func utf8HasPrefix(_ value: String, _ prefix: String) -> Bool {
    value.utf8.starts(with: prefix.utf8)
  }

  private static func bytewiseLess(_ a: String, _ b: String) -> Bool {
    var ai = a.utf8.makeIterator()
    var bi = b.utf8.makeIterator()
    while true {
      switch (ai.next(), bi.next()) {
      case (nil, nil): return false
      case (nil, .some): return true
      case (.some, nil): return false
      case let (.some(x), .some(y)):
        if x != y { return x < y }
      }
    }
  }

  // -------------------------------------------------------------------------
  // Built-in tables
  // -------------------------------------------------------------------------

  /// Common commands, ordered by rough frequency so the first prefix match
  /// is usually the intended one.
  static let commonCommands: [String] = [
    "git", "cd", "ls", "cargo", "npm", "npx", "node", "pnpm", "yarn", "python", "python3", "pip",
    "pip3", "docker", "kubectl", "make", "cmake", "go", "rustc", "rustup", "ssh", "scp", "curl",
    "wget", "grep", "rg", "fd", "find", "cat", "bat", "less", "tail", "head", "echo", "touch",
    "mkdir", "rmdir", "rm", "cp", "mv", "ln", "chmod", "chown", "tar", "zip", "unzip", "brew",
    "code", "vim", "nvim", "nano", "open", "kill", "ps", "top", "htop", "df", "du", "tree",
    "source", "export", "sudo", "man", "which", "history", "clear", "exit",
  ]

  /// POSIX/zsh/fish shell builtins worth completing when nothing else
  /// matches.
  private static let shellBuiltins: [String] = [
    "alias", "bg", "bind", "builtin", "command", "declare", "dirs", "disown", "eval", "exec", "fg",
    "function", "getopts", "hash", "jobs", "let", "local", "popd", "printf", "pushd", "read",
    "readonly", "return", "set", "setenv", "test", "trap", "type", "typeset", "ulimit", "umask",
    "unalias", "unset", "unsetenv", "wait",
  ]

  /// First-argument subcommands for popular tools, ordered by rough
  /// frequency so the single inline guess is the commonly-intended one.
  private static let subcommands: [(String, [String])] = [
    (
      "git",
      [
        "status", "add", "commit", "checkout", "push", "pull", "branch", "log", "diff", "merge",
        "fetch", "clone", "rebase", "reset", "restore", "stash", "switch", "show", "remote",
        "tag", "config", "init", "revert", "cherry-pick", "mv", "rm", "worktree",
      ]
    ),
    (
      "cargo",
      [
        "build", "run", "test", "check", "clippy", "fmt", "add", "new", "init", "update",
        "doc", "clean", "bench", "fix", "install", "publish", "remove", "fetch", "tree",
      ]
    ),
    (
      "npm",
      [
        "install", "run", "start", "test", "init", "ci", "update", "audit", "publish", "link",
        "ls", "outdated", "pack", "uninstall", "version",
      ]
    ),
    (
      "pnpm",
      [
        "install", "run", "add", "start", "test", "build", "update", "exec", "dlx", "init",
        "link", "list", "outdated", "remove",
      ]
    ),
    (
      "yarn",
      [
        "install", "add", "run", "start", "test", "build", "dev", "init", "remove", "upgrade",
      ]
    ),
    (
      "docker",
      [
        "run", "build", "ps", "exec", "logs", "compose", "images", "pull", "push", "stop",
        "start", "rm", "rmi", "inspect", "tag", "volume",
      ]
    ),
    (
      "kubectl",
      [
        "get", "describe", "apply", "logs", "exec", "delete", "create", "rollout", "scale",
        "config", "port-forward",
      ]
    ),
    (
      "brew",
      [
        "install", "update", "upgrade", "list", "search", "info", "uninstall", "reinstall",
        "outdated", "cleanup", "doctor",
      ]
    ),
    (
      "rustup",
      [
        "update", "default", "show", "toolchain", "target", "component", "override",
      ]
    ),
    (
      "go",
      [
        "run", "build", "test", "get", "mod", "install", "vet", "clean",
      ]
    ),
  ]

  /// Per-command flags worth completing before the generic set.
  private static let flags: [(String, [String])] = [
    (
      "git",
      [
        "--all", "--amend", "--force", "--message", "--no-verify", "--set-upstream",
      ]
    ),
    (
      "cargo",
      [
        "--all-features", "--bin", "--features", "--lib", "--package", "--release",
        "--workspace",
      ]
    ),
    (
      "ls",
      [
        "--all", "--almost-all", "--color", "--human-readable", "--long", "--reverse",
      ]
    ),
    (
      "rg",
      [
        "--count", "--fixed-strings", "--glob", "--hidden", "--ignore-case", "--line-number",
        "--no-ignore",
      ]
    ),
    (
      "docker",
      [
        "--detach", "--file", "--interactive", "--name", "--publish", "--rm", "--tty",
        "--volume",
      ]
    ),
  ]

  /// Flags accepted by almost everything.
  private static let commonFlags: [String] = ["--help", "--version", "--verbose", "--quiet"]
}
