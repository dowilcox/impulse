// Whether a command word names something the terminal's shell can run, for
// the input bar's unknown-command underline. The shell integration reports
// its aliases, functions, builtins and keywords (OSC 6973;Names) and its
// PATH (OSC 6973;Path); executables on that PATH are listed in the background.

import Foundation

public final class CommandLookup: @unchecked Sendable {
  /// Always runnable, whatever the shell reported (and before it has).
  static let baseNames: Set<String> = [
    "!", ".", ":", "[", "[[", "]]", "{", "}", "alias", "bg", "builtin", "case", "cd", "command", "do", "done",
    "echo", "elif", "else", "esac", "eval", "exec", "exit", "export", "false", "fc", "fg", "fi", "for",
    "function", "hash", "history", "if", "jobs", "kill", "local", "popd", "printf", "pushd", "pwd", "read",
    "return", "select", "set", "shift", "source", "test", "then", "time", "trap", "true", "type", "ulimit",
    "umask", "unalias", "unset", "until", "wait", "while",
  ]

  private let lock = NSLock()
  private var names: Set<String> = []
  private var path: String?
  private var executables: Set<String> = []
  private var scannedPath: String?
  private var scannedAt = Date.distantPast
  private var scanning = false
  private let scanQueue = DispatchQueue(label: "dev.impulse.command-lookup", qos: .utility)
  /// Lists a directory (a seam for tests).
  private let listDirectory: (String) -> [String]

  public init(listDirectory: @escaping (String) -> [String] = {
    (try? FileManager.default.contentsOfDirectory(atPath: $0)) ?? []
  }) {
    self.listDirectory = listDirectory
  }

  public func setNames(_ names: [String]) {
    lock.lock()
    self.names = Set(names)
    lock.unlock()
  }

  /// The shell's PATH; its executables are listed in the background.
  public func setPath(_ path: String) {
    lock.lock()
    let changed = self.path != path
    self.path = path
    lock.unlock()
    if changed { rescan() }
  }

  /// Wait for a pending PATH listing (tests).
  func waitForScan() {
    scanQueue.sync {}
  }

  /// True or false when it can tell; nil before the shell has reported (or
  /// while its PATH is still being listed), and for quoted or expanded words.
  public func isKnown(_ word: String, cwd: String?) -> Bool? {
    var command = word
    if command.hasPrefix("\\") { command.removeFirst() }  // \ls skips aliases
    guard !command.isEmpty else { return true }
    if command.contains(where: { "\"'$`*?(){}".contains($0) }) && command != "{" && command != "}" {
      return nil
    }
    if command.contains("/") {
      return Self.isRunnableFile(command, cwd: cwd)
    }
    if Self.baseNames.contains(command) { return true }

    lock.lock()
    let reported = !names.isEmpty || path != nil
    let ready = path != nil && scannedPath == path
    let known = names.contains(command) || executables.contains(command)
    let stale = Date().timeIntervalSince(scannedAt) > 10
    lock.unlock()

    guard reported else { return nil }
    if known { return true }
    guard ready else { return nil }
    // Maybe it was installed since the last listing.
    if stale { rescan() }
    return false
  }

  private func rescan() {
    lock.lock()
    guard !scanning, let path else {
      lock.unlock()
      return
    }
    scanning = true
    lock.unlock()
    scanQueue.async { [self] in
      var found = Set<String>()
      for directory in path.split(separator: ":") where !directory.isEmpty {
        found.formUnion(listDirectory(String(directory)))
      }
      lock.lock()
      executables = found
      scannedPath = path
      scannedAt = Date()
      scanning = false
      lock.unlock()
    }
  }

  private static func isRunnableFile(_ command: String, cwd: String?) -> Bool? {
    var path = command
    if path.hasPrefix("~/") {
      path = NSHomeDirectory() + path.dropFirst()
    } else if !path.hasPrefix("/") {
      guard let cwd else { return nil }
      path = (cwd as NSString).appendingPathComponent(path)
    }
    var isDirectory: ObjCBool = false
    guard FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory) else { return false }
    // A directory runs as `cd` in zsh and fish (auto_cd).
    return isDirectory.boolValue || FileManager.default.isExecutableFile(atPath: path)
  }
}
