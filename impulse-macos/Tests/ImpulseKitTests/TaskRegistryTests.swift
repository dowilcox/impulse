#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct TaskRegistryTests {
    private func temporaryGitDirectory() throws -> String {
      let path = FileManager.default.temporaryDirectory
        .appendingPathComponent("registry-\(UUID().uuidString)/repo/.git").path
      try FileManager.default.createDirectory(atPath: path, withIntermediateDirectories: true)
      return path
    }

    @Test func slotsAreTheLowestFreeNumberFromOne() {
      var registry = TaskRegistry()
      #expect(registry.nextSlot() == 1)
      registry.add(TaskRecord(path: "/r.worktrees/a", branch: "a", slot: 1))
      registry.add(TaskRecord(path: "/r.worktrees/b", branch: "b", slot: 2))
      registry.add(TaskRecord(path: "/r.worktrees/old", branch: "old"))
      #expect(registry.nextSlot() == 3, "adopted tasks hold no slot")
      registry.remove(path: "/r.worktrees/a")
      #expect(registry.nextSlot() == 1, "an archived task's slot is free again")
      #expect(registry.nextSlot(available: { $0 != 1 }) == 3, "a slot whose ports are busy is skipped")
    }

    @Test func addingTheSameFolderReplacesItsRecord() {
      var registry = TaskRegistry()
      registry.add(TaskRecord(path: "/r.worktrees/a", branch: "a", slot: 1))
      registry.add(TaskRecord(path: "/r.worktrees/./a", branch: "a2", slot: 4))
      #expect(registry.tasks.count == 1)
      #expect(registry.record(forPath: "/r.worktrees/a")?.branch == "a2")
      #expect(registry.record(forBranch: "a2")?.slot == 4)
    }

    @Test func baseRefNamesTheRemote() {
      #expect(TaskRecord(path: "/x", branch: "x", base: "main", remote: "origin").baseRef == "origin/main")
      #expect(TaskRecord(path: "/x", branch: "x", base: "main").baseRef == "main")
      #expect(TaskRecord(path: "/x", branch: "x").baseRef == nil)
    }

    @Test func reconcileDropsGoneTasksAndAdoptsOlderOnes() {
      var registry = TaskRegistry(tasks: [
        TaskRecord(path: "/code/app.worktrees/kept", branch: "kept", base: "main", slot: 1),
        TaskRecord(path: "/code/app.worktrees/gone", branch: "gone", slot: 2),
      ])
      let changed = registry.reconcile(
        worktrees: [
          (path: "/code/app", branch: "main"),
          (path: "/code/app.worktrees/kept", branch: "kept"),
          (path: "/code/app.worktrees/older", branch: "older"),
          (path: "/code/app.worktrees/detached", branch: nil),
          (path: "/code/app/.worktrees/nested", branch: "nested"),
          (path: "/elsewhere/manual", branch: "manual"),
        ],
        repoRoot: "/code/app")
      #expect(changed)
      #expect(registry.tasks.map(\.branch) == ["kept", "older"])
      #expect(registry.record(forBranch: "kept")?.base == "main", "existing records are kept as they are")
      #expect(registry.record(forBranch: "older")?.slot == nil)
      let changedAgain = registry.reconcile(
        worktrees: [(path: "/code/app.worktrees/kept", branch: "kept"), (path: "/code/app.worktrees/older", branch: "older")],
        repoRoot: "/code/app")
      #expect(!changedAgain)
    }

    @Test func updateWritesAndLoadReads() throws {
      let git = try temporaryGitDirectory()
      defer { try? FileManager.default.removeItem(atPath: (git as NSString).deletingLastPathComponent) }
      #expect(TaskRegistry.load(commonGitDirectory: git) == TaskRegistry())
      let created = Date(timeIntervalSince1970: 1_760_000_000)
      let slot = try TaskRegistry.update(commonGitDirectory: git) { registry -> Int in
        let slot = registry.nextSlot()
        registry.add(TaskRecord(path: "/r.worktrees/a", branch: "a", base: "main", remote: "origin", slot: slot, created: created))
        return slot
      }
      #expect(slot == 1)
      #expect(TaskRegistry.filePath(commonGitDirectory: git) == git + "/impulse/tasks.json")
      let loaded = TaskRegistry.load(commonGitDirectory: git)
      #expect(loaded.tasks == [
        TaskRecord(path: "/r.worktrees/a", branch: "a", base: "main", remote: "origin", slot: 1, created: created)
      ])
    }

    @Test func aBrokenFileIsKeptAsideAndReplaced() throws {
      let git = try temporaryGitDirectory()
      defer { try? FileManager.default.removeItem(atPath: (git as NSString).deletingLastPathComponent) }
      let path = TaskRegistry.filePath(commonGitDirectory: git)
      try FileManager.default.createDirectory(
        atPath: (path as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
      try "{ not json".write(toFile: path, atomically: true, encoding: .utf8)
      #expect(TaskRegistry.load(commonGitDirectory: git) == TaskRegistry())
      try TaskRegistry.update(commonGitDirectory: git) { $0.add(TaskRecord(path: "/r.worktrees/a", branch: "a", slot: 1)) }
      #expect(TaskRegistry.load(commonGitDirectory: git).tasks.count == 1)
      #expect(try String(contentsOfFile: path + ".bad", encoding: .utf8) == "{ not json")
    }

    @Test func canonicalPathsResolveSymlinksEvenForMissingFolders() throws {
      let git = try temporaryGitDirectory()
      defer { try? FileManager.default.removeItem(atPath: (git as NSString).deletingLastPathComponent) }
      // The temporary folder is under /var, a symlink to /private/var.
      let missing = (git as NSString).appendingPathComponent("gone/folder")
      #expect(TaskRegistry.canonical(missing).hasSuffix("/repo/.git/gone/folder"))
      #expect(TaskRegistry.canonical(missing) == TaskRegistry.canonical(missing.replacingOccurrences(of: "/.git/", with: "/.git/./")))
      #expect(TaskRegistry.canonical(git) == URL(fileURLWithPath: git).resolvingSymlinksInPath().path)
    }
  }
#endif
