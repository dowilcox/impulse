// Builds the fixture scenario repository (Fixtures/scenario.json) in a temp
// directory using the git CLI with a pinned author/committer/date environment,
// so every run reproduces the exact commit hash recorded in the fixtures.

import Foundation

struct ScenarioRepo {
  /// Canonicalized (symlink-resolved) absolute path of the repo root.
  let root: String

  private struct Scenario: Decodable {
    struct Author: Decodable {
      let date: String
      let email: String
      let name: String
    }

    let author: Author
    let baseFiles: [String: String]
    let worktreeChanges: [String: String?]

    enum CodingKeys: String, CodingKey {
      case author
      case baseFiles = "base_files"
      case worktreeChanges = "worktree_changes"
    }
  }

  static func create() throws -> ScenarioRepo {
    let scenario = try JSONDecoder().decode(Scenario.self, from: Fixtures.data("scenario.json"))

    let fileManager = FileManager.default
    let dir = fileManager.temporaryDirectory
      .appendingPathComponent("impulse-git-parity-\(UUID().uuidString)")
    try fileManager.createDirectory(at: dir, withIntermediateDirectories: true)
    // Canonicalize via realpath (resolves /var -> /private/var on macOS) so
    // paths match libgit2's realpath'd workdir. Note that
    // URL.resolvingSymlinksInPath() does NOT do this — it strips /private.
    var buffer = [CChar](repeating: 0, count: Int(PATH_MAX) + 1)
    guard realpath(dir.path, &buffer) != nil else {
      throw NSError(
        domain: "ScenarioRepo", code: 1,
        userInfo: [NSLocalizedDescriptionKey: "realpath failed for \(dir.path)"])
    }
    let root = String(cString: buffer)

    var environment = ProcessInfo.processInfo.environment
    environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
    environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
    environment["GIT_CONFIG_NOSYSTEM"] = "1"
    environment["GIT_AUTHOR_NAME"] = scenario.author.name
    environment["GIT_AUTHOR_EMAIL"] = scenario.author.email
    environment["GIT_AUTHOR_DATE"] = scenario.author.date
    environment["GIT_COMMITTER_NAME"] = scenario.author.name
    environment["GIT_COMMITTER_EMAIL"] = scenario.author.email
    environment["GIT_COMMITTER_DATE"] = scenario.author.date

    func git(_ arguments: [String]) throws {
      let process = Process()
      process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
      process.arguments = ["git", "-C", root] + arguments
      process.environment = environment
      let pipe = Pipe()
      process.standardOutput = pipe
      process.standardError = pipe
      try process.run()
      process.waitUntilExit()
      guard process.terminationStatus == 0 else {
        let output =
          String(data: pipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
        throw NSError(
          domain: "ScenarioRepo", code: Int(process.terminationStatus),
          userInfo: [NSLocalizedDescriptionKey: "git \(arguments) failed: \(output)"])
      }
    }

    func write(_ relativePath: String, _ content: String) throws {
      let url = URL(fileURLWithPath: root).appendingPathComponent(relativePath)
      try fileManager.createDirectory(
        at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
      try content.write(to: url, atomically: true, encoding: .utf8)
    }

    try git(["init", "-b", "main"])
    // Local identity so libgit2's git_signature_default works for commitAll
    // tests regardless of the user's global config.
    try git(["config", "user.name", scenario.author.name])
    try git(["config", "user.email", scenario.author.email])

    for (relativePath, content) in scenario.baseFiles.sorted(by: { $0.key < $1.key }) {
      try write(relativePath, content)
    }
    try git(["add", "-A"])
    try git(["commit", "-m", "base"])

    for (relativePath, content) in scenario.worktreeChanges.sorted(by: { $0.key < $1.key }) {
      if let content {
        try write(relativePath, content)
      } else {
        try fileManager.removeItem(
          at: URL(fileURLWithPath: root).appendingPathComponent(relativePath))
      }
    }

    return ScenarioRepo(root: root)
  }

  func destroy() {
    try? FileManager.default.removeItem(atPath: root)
  }
}
