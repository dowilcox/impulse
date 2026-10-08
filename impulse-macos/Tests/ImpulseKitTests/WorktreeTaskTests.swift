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

    @Test func tasksStartFromTheRemoteBranch() {
      #expect(WorktreeTasks.defaultBase(branch: "main", upstream: "origin/main", ahead: 0) == ("origin/main", nil))
      #expect(WorktreeTasks.defaultBase(branch: "main", upstream: nil, ahead: 0) == ("main", nil))
      #expect(WorktreeTasks.defaultBase(branch: nil, upstream: nil, ahead: 0) == ("HEAD", nil))
      let one = WorktreeTasks.defaultBase(branch: "main", upstream: "origin/main", ahead: 1)
      #expect(one.base == "origin/main")
      #expect(one.note == "main has 1 unpushed commit that isn't included; type main to include it.")
      #expect(
        WorktreeTasks.defaultBase(branch: "main", upstream: "upstream/main", ahead: 2).note
          == "main has 2 unpushed commits that aren't included; type main to include them.")
    }

    @Test func worktreesLiveBesideTheRepository() {
      #expect(
        WorktreeTasks.worktreePath(repoRoot: "/Users/me/Code/app", branch: "feat/search")
          == "/Users/me/Code/app.worktrees/feat-search")
      #expect(
        WorktreeTasks.worktreePath(repoRoot: "/Users/me/Code/app/", branch: "fix")
          == "/Users/me/Code/app.worktrees/fix")
    }

    @Test func includePatternsDefaultToEnvFilesAndProjectHooks() {
      #expect(
        WorktreeTasks.includePatterns(fromFile: nil) == [".env", ".env.local", ".claude/settings.local.json"])
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
      // Nothing outside the repository.
      let outside = (root as NSString).lastPathComponent
      #expect(
        WorktreeTasks.matchingFiles(patterns: ["../\(outside)/.env", "/etc/hosts", "config/../.env"], root: root)
          .isEmpty)
    }

    @Test func defaultsCopyProjectAgentHooks() throws {
      let root = FileManager.default.temporaryDirectory
        .appendingPathComponent("impulse-wt-\(UUID().uuidString)").path
      defer { try? FileManager.default.removeItem(atPath: root) }
      try FileManager.default.createDirectory(atPath: root + "/.claude", withIntermediateDirectories: true)
      for file in [".env", ".claude/settings.local.json", ".claude/settings.json"] {
        try Data("x".utf8).write(to: URL(fileURLWithPath: root + "/" + file))
      }
      #expect(
        WorktreeTasks.matchingFiles(patterns: WorktreeTasks.includePatterns(fromFile: nil), root: root)
          == [".env", ".claude/settings.local.json"])
    }
  }
#endif
