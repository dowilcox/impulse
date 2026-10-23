#if canImport(Testing)
  import Foundation
  @testable import ImpulseApp
  import ImpulseGit
  import ImpulseKit
  import Testing

  /// Finished tasks: telling a squash-merged task apart, archiving it with
  /// its branch, and Undo bringing all of it back. The git CLI is the oracle.
  @Suite(.serialized)
  struct TaskCleanupTests {
    private static let environment: [String: String] = [
      "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_SYSTEM": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1",
      "GIT_AUTHOR_NAME": "Impulse Test", "GIT_AUTHOR_EMAIL": "test@impulse.invalid",
      "GIT_AUTHOR_DATE": "2026-01-01T00:00:00Z", "GIT_COMMITTER_NAME": "Impulse Test",
      "GIT_COMMITTER_EMAIL": "test@impulse.invalid", "GIT_COMMITTER_DATE": "2026-01-01T00:00:00Z",
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

    @Test func aSquashMergedTaskIsArchivedWithItsBranchAndUndoBringsItBack() throws {
      let folder = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("impulse-cleanup-\(UUID().uuidString)")
      try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
      defer { try? FileManager.default.removeItem(at: folder) }
      let main = TaskRegistry.canonical(folder.appendingPathComponent("repo").path)
      try FileManager.default.createDirectory(atPath: main, withIntermediateDirectories: true)
      try git("init", "-q", "-b", "main", in: main)
      try "a\n".write(toFile: main + "/a.txt", atomically: true, encoding: .utf8)
      try git("add", "-A", in: main)
      try git("commit", "-q", "-m", "first", in: main)

      let task = WorktreeTasks.worktreePath(repoRoot: main, branch: "fix")
      try git("worktree", "add", "-q", "--no-track", "-b", "fix", task, "main", in: main)
      let record = try #require(TaskRegistryStore.recordCreated(path: task, branch: "fix", base: "main", root: main))
      #expect(record.start == (try git("rev-parse", "main", in: main)))
      #expect(OverlapMonitor.mergedTasks(root: main).isEmpty, "no work of its own yet")

      try "b\n".write(toFile: task + "/b.txt", atomically: true, encoding: .utf8)
      try git("add", "-A", in: task)
      try git("commit", "-q", "-m", "fix", in: task)
      #expect(OverlapMonitor.mergedTasks(root: main).isEmpty, "not merged yet")

      try git("merge", "-q", "--squash", "fix", in: main)
      try git("commit", "-q", "-m", "Squash fix", in: main)
      #expect(OverlapMonitor.mergedTasks(root: main) == [TaskRegistry.canonical(task)])

      // Archive it, uncommitted notes and branch included.
      try "notes\n".write(toFile: task + "/notes.txt", atomically: true, encoding: .utf8)
      let tip = try git("rev-parse", "fix", in: main)
      var archived = try MainWindowController.removeTask(root: task, branch: "fix", dirty: true, archiveScript: nil).result.get()
      #expect(!FileManager.default.fileExists(atPath: task))
      #expect(TaskRegistryStore.registry(root: main)?.record(forPath: task) == nil)
      _ = try GitOperations.deleteBranch("fix", force: true, root: main).get()
      archived.deletedBranchCommit = tip
      #expect((try? git("rev-parse", "--verify", "-q", "refs/heads/fix", in: main)) == nil)

      _ = try MainWindowController.restoreArchived(archived).get()
      #expect(try git("rev-parse", "fix", in: main) == tip)
      #expect(try String(contentsOfFile: task + "/notes.txt", encoding: .utf8) == "notes\n")
      #expect(TaskRegistryStore.registry(root: main)?.record(forPath: task)?.slot == record.slot)
      #expect(OverlapMonitor.mergedTasks(root: main) == [TaskRegistry.canonical(task)])
    }

    @Test func aTaskWithACheckedOutSubmoduleIsntArchived() throws {
      let folder = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("impulse-cleanup-\(UUID().uuidString)")
      try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
      defer { try? FileManager.default.removeItem(at: folder) }
      let lib = TaskRegistry.canonical(folder.appendingPathComponent("lib").path)
      let main = TaskRegistry.canonical(folder.appendingPathComponent("repo").path)
      for repo in [lib, main] {
        try FileManager.default.createDirectory(atPath: repo, withIntermediateDirectories: true)
        try git("init", "-q", "-b", "main", in: repo)
        try "x\n".write(toFile: repo + "/x.txt", atomically: true, encoding: .utf8)
        try git("add", "-A", in: repo)
        try git("commit", "-q", "-m", "first", in: repo)
      }
      try git("-c", "protocol.file.allow=always", "submodule", "add", "-q", lib, "sub", in: main)
      try git("commit", "-q", "-m", "add sub", in: main)
      let task = WorktreeTasks.worktreePath(repoRoot: main, branch: "fix")
      try git("worktree", "add", "-q", "-b", "fix", task, in: main)
      try git("-c", "protocol.file.allow=always", "submodule", "update", "-q", "--init", in: task)
      // Work that exists only in the task's copy of the submodule.
      try "unpushed\n".write(toFile: task + "/sub/unpushed.txt", atomically: true, encoding: .utf8)
      try git("add", "-A", in: task + "/sub")
      try git("commit", "-q", "-m", "unpushed", in: task + "/sub")
      let unpushed = try git("rev-parse", "HEAD", in: task + "/sub")

      guard case .failure = MainWindowController.removeTask(root: task, branch: "fix", dirty: true, archiveScript: nil).result
      else {
        Issue.record("expected the archive to refuse")
        return
      }
      #expect(try git("rev-parse", "HEAD", in: task + "/sub") == unpushed)
    }
  }
#endif
