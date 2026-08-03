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

    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/dscl")
    process.arguments = [".", "-read", "/Users/\(username)", "UserShell"]
    let stdout = Pipe()
    process.standardOutput = stdout
    process.standardError = Pipe()
    do {
      try process.run()
    } catch {
      return nil
    }
    process.waitUntilExit()
    guard process.terminationStatus == 0 else { return nil }

    let data = stdout.fileHandleForReading.readDataToEndOfFile()
    guard let output = String(data: data, encoding: .utf8),
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

  /// Detect the shell type from a shell path. Unknown shells fall back to bash.
  public static func detectShellType(_ shellPath: String) -> ShellType {
    switch (shellPath as NSString).lastPathComponent {
    case "zsh": return .zsh
    case "fish": return .fish
    default: return .bash
    }
  }
}
