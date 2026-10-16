#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseGit

  struct GitLogDecorationTests {
    init() {
      GitOperations.environment = TempRepo.gitOverrides
    }

    /// The decorations read from ref tips match what `git log` puts on the
    /// same commits, before and after refs move.
    @Test func decorationsMatchTheLog() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\n"])
      try repo.git("checkout", "-q", "-b", "topic")
      try repo.commit(["b.txt": "topic\n"])
      try repo.git("checkout", "-q", "main")
      try repo.commit(["a.txt": "two\n"])
      try repo.git("tag", "v1")
      try repo.git("tag", "-a", "v1-notes", "-m", "Annotated", "HEAD~1")
      try repo.git("update-ref", "refs/remotes/origin/main", "HEAD~1")
      try repo.git("update-ref", "refs/impulse/snapshots/test", "HEAD")

      func expectMatch() throws {
        let entries = try GitLog.entries(root: repo.root, scope: .all).get()
        let decorations = try GitLog.decorations(root: repo.root).get()
        for entry in entries {
          #expect(decorations[entry.sha] ?? [] == entry.refs, "\(entry.subject)")
        }
        #expect(entries.contains { !$0.refs.isEmpty })
      }
      try expectMatch()

      // A new tag, a moved remote branch, a deleted branch, detached HEAD.
      try repo.git("tag", "v2", "topic")
      try repo.git("update-ref", "refs/remotes/origin/main", "HEAD")
      try repo.git("branch", "-D", "topic")
      try repo.git("checkout", "-q", "--detach", "HEAD~1")
      try expectMatch()
    }

    /// Tips change with the commits history starts from, not with refs
    /// added to commits already in it.
    @Test func tipsFollowWhereHistoryStarts() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\n"])
      try repo.commit(["a.txt": "two\n"])
      let head = try repo.git("rev-parse", "HEAD")
      #expect(GitLog.tips(root: repo.root, scope: .head) == [head])

      let all = try #require(GitLog.tips(root: repo.root, scope: .all))
      try repo.git("branch", "old", "HEAD~1")
      #expect(GitLog.tips(root: repo.root, scope: .head) == [head], "a branch elsewhere")
      #expect(GitLog.tips(root: repo.root, scope: .all) != all, "a new branch is a new start")
      #expect(GitLog.tips(root: repo.root, scope: .branch("old")) == [try repo.git("rev-parse", "HEAD~1")])

      try repo.commit(["a.txt": "three\n"])
      #expect(GitLog.tips(root: repo.root, scope: .head) == [try repo.git("rev-parse", "HEAD")])
      #expect(GitLog.tips(root: repo.root, scope: .branch("missing")) == nil)
    }
  }
#endif
