#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct TaskOverlapTests {
    private let main = TaskOverlap.Changes(
      path: "/code/app", name: "app", files: ["src/page.tsx", "src/link.tsx", "composer.lock", "README.md"])
    private let upgrade = TaskOverlap.Changes(
      path: "/code/app.worktrees/upgrade", name: "upgrade", files: ["src/page.tsx", "src/link.tsx", "composer.lock"])
    private let docs = TaskOverlap.Changes(path: "/code/app.worktrees/docs", name: "docs", files: ["README.md"])
    private let quiet = TaskOverlap.Changes(path: "/code/app.worktrees/quiet", name: "quiet", files: ["other.txt"])

    @Test func pairsShareFilesMostFirst() {
      let pairs = TaskOverlap.pairs([main, upgrade, docs, quiet])
      #expect(pairs.count == 2)
      #expect(pairs[0].files == ["composer.lock", "src/link.tsx", "src/page.tsx"], "lock files count")
      #expect(pairs[0].other(than: "/code/app").name == "upgrade")
      #expect(pairs[1].files == ["README.md"])
      #expect(pairs[0].key == TaskOverlap.Pair(a: upgrade, b: main, files: []).key, "the key ignores order")
      #expect(!pairs.contains { $0.contains("/code/app.worktrees/quiet") })
    }

    @Test func ignoredFilesDontCount() {
      let pairs = TaskOverlap.pairs([main, upgrade, docs], ignoring: ["*.lock", "README.md"])
      #expect(pairs.map(\.files) == [["src/link.tsx", "src/page.tsx"]])
      #expect(TaskOverlap.pairs([main, upgrade], ignoring: ["src/*"]).map(\.files) == [["composer.lock"]])
    }
  }
#endif
