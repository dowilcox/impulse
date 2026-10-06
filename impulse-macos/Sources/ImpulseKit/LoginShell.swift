import Foundation

// Ported from impulse-core/src/shell.rs (detection portion — the rc-file
// scaffolding already lives in the app's TerminalTab).

public enum ShellType: String {
  case bash
  case zsh
  case fish
}

public enum LoginShell {
  /// The user's login shell from Open Directory (`dscl`), since `/etc/passwd`
  /// only contains system accounts on macOS. Returns nil when it cannot be
  /// determined or the shell binary does not exist.
  public static func userLoginShell() -> String? {
    guard let username = ProcessInfo.processInfo.environment["USER"] else { return nil }
    // Validate username to prevent path traversal in the dscl argument.
    let allowed = username.allSatisfy { char in
      char.isLetter || char.isNumber || char == "_" || char == "-" || char == "."
    }
    guard allowed, !username.isEmpty else {
      NSLog("Refusing $USER with unexpected characters: %@", username)
      return nil
    }

    guard
      let result = try? ChildProcess.run(
        "/usr/bin/dscl", [".", "-read", "/Users/\(username)", "UserShell"], timeout: 5),
      result.status == 0, !result.timedOut,
      let output = String(data: result.stdout, encoding: .utf8),
      output.hasPrefix("UserShell:")
    else { return nil }
    let shell = String(output.dropFirst("UserShell:".count))
      .trimmingCharacters(in: .whitespacesAndNewlines)
    guard !shell.isEmpty, FileManager.default.fileExists(atPath: shell) else { return nil }
    return shell
  }

  /// The default shell path, preferring the system user database over `$SHELL`.
  public static func defaultShellPath() -> String {
    userLoginShell()
      ?? ProcessInfo.processInfo.environment["SHELL"]
      ?? "/bin/bash"
  }

  /// The short name of the user's default shell (e.g. "fish", "zsh", "bash").
  public static func defaultShellName() -> String {
    let name = (defaultShellPath() as NSString).lastPathComponent
    return name.isEmpty ? "shell" : name
  }

  // MARK: - Login-shell PATH

  private static let pathLock = NSLock()
  nonisolated(unsafe) private static var cachedPath: String?

  /// The `PATH` a login shell would see. GUI apps launched from Finder/Dock
  /// inherit launchd's minimal `PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`), which
  /// misses Homebrew, mise/asdf shims, `~/.local/bin` and so on — so tools like
  /// `git`, `gh` and language servers must be resolved against the user's real
  /// login environment. Captured once (blocking, with a timeout; the app
  /// warms it at launch) and cached. Falls back to the process `PATH` plus
  /// common install prefixes.
  public static func loginPath() -> String {
    pathLock.lock()
    defer { pathLock.unlock() }
    if let cachedPath { return cachedPath }
    let captured = captureLoginPath() ?? ""
    let fallback = ProcessInfo.processInfo.environment["PATH"] ?? "/usr/bin:/bin:/usr/sbin:/sbin"
    let path = mergePaths(captured, fallback, "/opt/homebrew/bin:/usr/local/bin")
    cachedPath = path
    return path
  }

  /// Joins colon-separated PATH strings, dropping empty and duplicate entries
  /// while preserving first-seen order.
  public static func mergePaths(_ paths: String...) -> String {
    var seen = Set<String>()
    var out: [String] = []
    for path in paths {
      for entry in path.split(separator: ":").map(String.init) where !entry.isEmpty {
        if seen.insert(entry).inserted { out.append(entry) }
      }
    }
    return out.joined(separator: ":")
  }

  /// Resolve an executable name against the login `PATH`.
  public static func which(_ name: String) -> String? {
    if name.contains("/") {
      return FileManager.default.isExecutableFile(atPath: name) ? name : nil
    }
    for dir in loginPath().split(separator: ":") {
      let candidate = "\(dir)/\(name)"
      if FileManager.default.isExecutableFile(atPath: candidate) { return candidate }
    }
    return nil
  }

  /// The PATH of an interactive login shell, like a terminal's, so entries
  /// added in `.zshrc` or `.bashrc` (nvm, pyenv, cargo, …) count too. An rc
  /// file that never finishes (one that execs tmux, say) falls back to a
  /// login-only shell. `IMPULSE_RESOLVING_ENVIRONMENT=1` lets rc files skip
  /// slow or interactive setup.
  private static func captureLoginPath() -> String? {
    let shell = defaultShellPath()
    // `printf '%s' "$PATH"` works in bash, zsh and fish (fish joins PATH
    // variables with ':' when quoted). Markers isolate it from rc-file noise.
    let script = "printf '__IMPULSE_PATH__%s__IMPULSE_PATH__' \"$PATH\""
    return capturePath(shell, ["-i", "-l", "-c", script], timeout: 5)
      ?? capturePath(shell, ["-l", "-c", script], timeout: 3)
  }

  private static func capturePath(_ shell: String, _ arguments: [String], timeout: TimeInterval) -> String? {
    var environment = ProcessInfo.processInfo.environment
    environment["IMPULSE_RESOLVING_ENVIRONMENT"] = "1"
    guard
      let result = try? ChildProcess.run(shell, arguments, environment: environment, timeout: timeout),
      !result.timedOut
    else { return nil }
    let output = String(decoding: result.stdout, as: UTF8.self)
    let parts = output.components(separatedBy: "__IMPULSE_PATH__")
    guard parts.count >= 3 else { return nil }
    let path = parts[1].trimmingCharacters(in: .whitespacesAndNewlines)
    return path.isEmpty ? nil : path
  }

  /// Detect the shell type from a shell path. Unknown shells fall back to bash.
  public static func detectShellType(_ shellPath: String) -> ShellType {
    switch (shellPath as NSString).lastPathComponent {
    case "zsh": return .zsh
    case "fish": return .fish
    default: return .bash
    }
  }
}
