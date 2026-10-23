#if canImport(Testing)
  import Foundation
  @testable import ImpulseApp
  import ImpulseGit
  import ImpulseKit
  import Testing

  /// What each workspace changes (answers about commit pairs are cached),
  /// and which snapshot changes are worth recomputing for. The git CLI
  /// builds the repositories.
  @Suite(.serialized)
  struct OverlapMonitorTests {
    private static let environment: [String: String] = [
      "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_SYSTEM": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1",
      "GIT_AUTHOR_NAME": "Impulse Test", "GIT_AUTHOR_EMAIL": "test@impulse.invalid",
      "GIT_COMMITTER_NAME": "Impulse Test", "GIT_COMMITTER_EMAIL": "test@impulse.invalid",
    ]

    init() {
      GitOperations.environment = Self.environment
    }

    @discardableResult
    private func git(_ arguments: String..., in root: String) throws -> String {
      let process = Process()
      process.executableURL = URL(fileURLWithPath: GitCLI.gitPath() ?? "/usr/bin/git")
      process.arguments = ["-C", root] + arguments
      process.environment = ProcessInfo.processInfo.environment.merging(Self.environment) { $1 }
      let out = Pipe()
      process.standardOutput = out
      process.standardError = FileHandle.nullDevice
      try process.run()
      let data = out.fileHandleForReading.readDataToEndOfFile()
      process.waitUntilExit()
      guard process.terminationStatus == 0 else {
        throw NSError(domain: "git", code: Int(process.terminationStatus), userInfo: [NSLocalizedDescriptionKey: "\(arguments)"])
      }
      return String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private func write(_ text: String, _ path: String) throws {
      try text.write(toFile: path, atomically: true, encoding: .utf8)
    }

    private func makeRepository() throws -> (folder: URL, main: String) {
      let folder = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("impulse-overlap-\(UUID().uuidString)")
      try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
      let main = TaskRegistry.canonical(folder.appendingPathComponent("repo").path)
      try FileManager.default.createDirectory(atPath: main, withIntermediateDirectories: true)
      try git("init", "-q", "-b", "main", in: main)
      try write("a\n", main + "/a.txt")
      try git("add", "-A", in: main)
      try git("commit", "-q", "-m", "first", in: main)
      return (folder, main)
    }

    @Test func onlyWorkChangesAreWorthARecompute() throws {
      let (folder, main) = try makeRepository()
      defer { try? FileManager.default.removeItem(at: folder) }
      try write("a\nb\n", main + "/a.txt")
      let edited = try #require(GitClient.snapshot(forPath: main))
      #expect(OverlapMonitor.mayChangeOverlaps(from: nil, to: edited))

      // More lines in a file already changed, then staging it: same files.
      try write("a\nb\nc\n", main + "/a.txt")
      let more = try #require(GitClient.snapshot(forPath: main))
      #expect(more != edited)
      #expect(!OverlapMonitor.mayChangeOverlaps(from: edited, to: more))
      try git("add", "a.txt", in: main)
      let staged = try #require(GitClient.snapshot(forPath: main))
      #expect(!OverlapMonitor.mayChangeOverlaps(from: more, to: staged))

      // Another file, or a commit, is.
      try write("new\n", main + "/b.txt")
      let another = try #require(GitClient.snapshot(forPath: main))
      #expect(OverlapMonitor.mayChangeOverlaps(from: staged, to: another))
      try git("add", "-A", in: main)
      try git("commit", "-q", "-m", "second", in: main)
      let committed = try #require(GitClient.snapshot(forPath: main))
      #expect(OverlapMonitor.mayChangeOverlaps(from: another, to: committed))
    }

    @Test func changesFollowCommitsOnEitherSide() throws {
      let (folder, main) = try makeRepository()
      defer { try? FileManager.default.removeItem(at: folder) }
      let task = WorktreeTasks.worktreePath(repoRoot: main, branch: "fix")
      try git("worktree", "add", "-q", "--no-track", "-b", "fix", task, "main", in: main)
      try #require(TaskRegistryStore.recordCreated(path: task, branch: "fix", base: "main", root: main) != nil)

      func files() -> [String: Set<String>] {
        Dictionary(uniqueKeysWithValues: OverlapMonitor.changes(root: task).map { ($0.name, $0.files) })
      }
      let taskName = (task as NSString).lastPathComponent
      #expect(files() == ["repo": [], taskName: []])

      try write("b\n", task + "/b.txt")
      try git("add", "-A", in: task)
      try git("commit", "-q", "-m", "b", in: task)
      try write("c\n", task + "/c.txt")
      try write("a2\n", main + "/a.txt")
      #expect(files() == ["repo": ["a.txt"], taskName: ["b.txt", "c.txt"]])

      // The task's next commit, and its base moving on, both count.
      try git("add", "-A", in: task)
      try git("commit", "-q", "-m", "c", in: task)
      try write("d\n", task + "/d.txt")
      try git("add", "-A", in: task)
      try git("commit", "-q", "-m", "d", in: task)
      #expect(files()[taskName] == ["b.txt", "c.txt", "d.txt"])
      try git("merge", "-q", "fix", in: main)
      try git("checkout", "-q", "--", "a.txt", in: main)
      #expect(files() == ["repo": [], taskName: []])
    }
  }
#endif
