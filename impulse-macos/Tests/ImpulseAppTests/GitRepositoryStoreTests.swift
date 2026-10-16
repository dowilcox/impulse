#if canImport(Testing)
  import Foundation
  @testable import ImpulseApp
  import ImpulseGit
  import ImpulseKit
  import Testing

  /// The store lets go of repositories nothing shows, and keeps one that's
  /// shown through its folder going missing and coming back.
  @MainActor
  @Suite(.serialized)
  struct GitRepositoryStoreTests {
    private func makeRepository(in folder: URL, named name: String) throws -> String {
      let root = TaskRegistry.canonical(folder.appendingPathComponent(name).path)
      try FileManager.default.createDirectory(atPath: root, withIntermediateDirectories: true)
      let process = Process()
      process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
      process.arguments = ["git", "-C", root, "init", "-q", "-b", "main"]
      process.environment = ProcessInfo.processInfo.environment.merging([
        "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_SYSTEM": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1",
      ]) { $1 }
      process.standardOutput = FileHandle.nullDevice
      process.standardError = FileHandle.nullDevice
      try process.run()
      process.waitUntilExit()
      try "a\n".write(toFile: root + "/a.txt", atomically: true, encoding: .utf8)
      return root
    }

    private func waitUntil(_ condition: () -> Bool) async throws {
      for _ in 0..<50 where !condition() {
        try await Task.sleep(for: .milliseconds(100))
      }
    }

    @Test func sweepDropsWhatNothingShowsAndKeepsWhatsShownThroughAMissingFolder() async throws {
      let folder = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("impulse-store-\(UUID().uuidString)")
      try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
      defer { try? FileManager.default.removeItem(at: folder) }
      let shownRoot = try makeRepository(in: folder, named: "shown")
      let unusedRoot = try makeRepository(in: folder, named: "unused")

      let store = GitRepositoryStore()
      let shown = store.state(forRoot: shownRoot)
      weak var unused = store.state(forRoot: unusedRoot)
      try await waitUntil { !shown.isBusy && shown.isWatching && unused?.isBusy == false }
      #expect(shown.isWatching)

      store.sweep()
      #expect(unused == nil, "nothing shows it")
      #expect(store.state(forRoot: shownRoot) === shown)

      // Its folder goes away (to the Trash, say) while it's shown.
      let away = folder.appendingPathComponent("away").path
      try FileManager.default.moveItem(atPath: shownRoot, toPath: away)
      store.sweep()
      #expect(!shown.isWatching)
      #expect(store.state(forRoot: shownRoot) === shown, "still the one the window holds")

      // And comes back.
      try FileManager.default.moveItem(atPath: away, toPath: shownRoot)
      store.sweep()
      try await waitUntil { shown.isWatching }
      #expect(shown.isWatching)
      #expect(shown.snapshot?.untracked.map(\.path) == ["a.txt"])
      #expect(store.state(forRoot: shownRoot) === shown)
    }
  }
#endif
