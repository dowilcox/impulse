#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct WorktreeTaskTests {
    @Test func branchNamesAreSlugsAndUnique() {
      #expect(WorktreeTasks.branchName(for: "Fix the login bug!", taken: []) == "fix-the-login-bug")
      #expect(WorktreeTasks.branchName(for: "  Ünïcode — dashes  ", taken: []) == "unicode-dashes")
      #expect(WorktreeTasks.branchName(for: "feat/Search UI", taken: []) == "feat/search-ui")
      #expect(WorktreeTasks.branchName(for: "!!!", taken: []) == "task")
      #expect(
        WorktreeTasks.branchName(for: "fix bug", taken: ["fix-bug", "fix-bug-2"]) == "fix-bug-3")
      let long = String(repeating: "word ", count: 30)
      #expect(WorktreeTasks.branchName(for: long, taken: []).count <= 48)
    }

    @Test func worktreesLiveBesideTheRepository() {
      #expect(
        WorktreeTasks.worktreePath(repoRoot: "/Users/me/Code/app", branch: "feat/search")
          == "/Users/me/Code/app.worktrees/feat-search")
      #expect(
        WorktreeTasks.worktreePath(repoRoot: "/Users/me/Code/app/", branch: "fix")
          == "/Users/me/Code/app.worktrees/fix")
    }

    @Test func includePatternsDefaultToEnvFiles() {
      #expect(WorktreeTasks.includePatterns(fromFile: nil) == [".env", ".env.local"])
      #expect(
        WorktreeTasks.includePatterns(fromFile: "# secrets\n.env*\n\n/config/local.json\n")
          == [".env*", "config/local.json"])
    }

    @Test func matchesFilesAndSingleFolderWildcards() throws {
      let root = FileManager.default.temporaryDirectory
        .appendingPathComponent("impulse-wt-\(UUID().uuidString)").path
      defer { try? FileManager.default.removeItem(atPath: root) }
      let fm = FileManager.default
      try fm.createDirectory(atPath: root + "/config/nested", withIntermediateDirectories: true)
      for file in [".env", ".env.test", "config/a.local.json", "config/b.json", "config/nested/c.local.json"] {
        try Data("x".utf8).write(to: URL(fileURLWithPath: root + "/" + file))
      }
      #expect(WorktreeTasks.matchingFiles(patterns: [".env*"], root: root).sorted() == [".env", ".env.test"])
      #expect(WorktreeTasks.matchingFiles(patterns: ["config/*.local.json"], root: root) == ["config/a.local.json"])
      #expect(WorktreeTasks.matchingFiles(patterns: [".env", "missing.txt", "config"], root: root) == [".env"])
    }
  }
#endif
