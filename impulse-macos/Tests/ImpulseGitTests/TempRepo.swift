// A throwaway git repository for tests of the newer git APIs. Unlike
// ScenarioRepo (which reproduces the golden-fixture scenario exactly), this is
// a scratchpad: tests build whatever state they need with the git CLI, which
// also serves as the oracle for expected results.

import Foundation

struct TempRepo {
  /// Canonicalized (symlink-resolved) absolute path of the repo root.
  let root: String

  /// Pinned identity/config so commits are deterministic and the user's
  /// global config (hooks, signing, aliases) can't leak into tests.
  static let environment: [String: String] = {
    var env = ProcessInfo.processInfo.environment
    env["GIT_CONFIG_GLOBAL"] = "/dev/null"
    env["GIT_CONFIG_SYSTEM"] = "/dev/null"
    env["GIT_CONFIG_NOSYSTEM"] = "1"
    env["GIT_AUTHOR_NAME"] = "Impulse Test"
    env["GIT_AUTHOR_EMAIL"] = "test@impulse.invalid"
    env["GIT_AUTHOR_DATE"] = "2026-01-01T00:00:00Z"
    env["GIT_COMMITTER_NAME"] = "Impulse Test"
    env["GIT_COMMITTER_EMAIL"] = "test@impulse.invalid"
    env["GIT_COMMITTER_DATE"] = "2026-01-01T00:00:00Z"
    return env
  }()

  /// Environment overrides for code under test that shells out to git.
  static var gitOverrides: [String: String] {
    environment.filter { $0.key.hasPrefix("GIT_") }
  }

  static func create(initialBranch: String = "main") throws -> TempRepo {
    let fileManager = FileManager.default
    let dir = fileManager.temporaryDirectory
      .appendingPathComponent("impulse-git-test-\(UUID().uuidString)")
    try fileManager.createDirectory(at: dir, withIntermediateDirectories: true)
    var buffer = [CChar](repeating: 0, count: Int(PATH_MAX) + 1)
    guard realpath(dir.path, &buffer) != nil else {
      throw NSError(domain: "TempRepo", code: 1)
    }
    let repo = TempRepo(root: String(cString: buffer))
    try repo.git("init", "-q", "-b", initialBranch)
    return repo
  }

  /// Run git in the repo, returning trimmed stdout. Throws on failure.
  @discardableResult
  func git(_ arguments: String...) throws -> String {
    try git(arguments)
  }

  @discardableResult
  func git(_ arguments: [String], stdin: String? = nil) throws -> String {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
    process.arguments = ["git", "-C", root] + arguments
    process.environment = Self.environment
    let out = Pipe()
    let err = Pipe()
    process.standardOutput = out
    process.standardError = err
    let input = Pipe()
    process.standardInput = stdin == nil ? FileHandle.nullDevice : input
    try process.run()
    if let stdin {
      input.fileHandleForWriting.write(Data(stdin.utf8))
      try? input.fileHandleForWriting.close()
    }
    let outData = out.fileHandleForReading.readDataToEndOfFile()
    let errData = err.fileHandleForReading.readDataToEndOfFile()
    process.waitUntilExit()
    guard process.terminationStatus == 0 else {
      throw NSError(
        domain: "TempRepo", code: Int(process.terminationStatus),
        userInfo: [
          NSLocalizedDescriptionKey:
            "git \(arguments) failed: \(String(decoding: errData, as: UTF8.self))"
        ])
    }
    return String(decoding: outData, as: UTF8.self)
      .trimmingCharacters(in: .whitespacesAndNewlines)
  }

  func write(_ relativePath: String, _ content: String) throws {
    let url = URL(fileURLWithPath: root).appendingPathComponent(relativePath)
    try FileManager.default.createDirectory(
      at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
    try content.write(to: url, atomically: true, encoding: .utf8)
  }

  func read(_ relativePath: String) throws -> String {
    try String(contentsOfFile: root + "/" + relativePath, encoding: .utf8)
  }

  func exists(_ relativePath: String) -> Bool {
    FileManager.default.fileExists(atPath: root + "/" + relativePath)
  }

  /// Write files and commit them.
  func commit(_ files: [String: String], message: String = "commit") throws {
    for (path, content) in files { try write(path, content) }
    try git("add", "-A")
    try git("commit", "-q", "-m", message)
  }

  func destroy() {
    try? FileManager.default.removeItem(atPath: root)
  }
}
