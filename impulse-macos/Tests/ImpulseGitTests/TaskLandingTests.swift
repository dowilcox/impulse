#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseGit

  /// Finish Task's landing: a merge commit pushed as the base from a
  /// throwaway worktree, against a bare repository standing in for the
  /// remote. The git CLI is the oracle.
  @Suite(.serialized)
  struct TaskLandingTests {
    init() {
      GitOperations.environment = TempRepo.gitOverrides
    }

    /// A clone of a bare "origin" with one pushed commit on main, a task
    /// branch `fix` with one commit of its own, and a second clone
    /// ("someone else") that can push to origin too.
    private func setUp() throws -> (repo: TempRepo, origin: TempRepo, other: TempRepo) {
      let origin = try TempRepo.create()
      try origin.git("config", "core.bare", "true")
      let other = try TempRepo.create()
      try other.commit(["a.txt": "1\n2\n3\n"], message: "first")
      try other.git("remote", "add", "origin", origin.root + "/.git")
      try other.git("push", "-q", "-u", "origin", "main")
      let repo = try TempRepo.create()
      try repo.git("remote", "add", "origin", origin.root + "/.git")
      try repo.git("fetch", "-q", "origin")
      try repo.git("checkout", "-q", "-B", "main", "--track", "origin/main")
      try repo.git("branch", "-q", "--no-track", "fix", "origin/main")
      try repo.git("switch", "-q", "fix")
      try repo.commit(["b.txt": "fix\n"], message: "fix")
      try repo.git("switch", "-q", "main")
      return (repo, origin, other)
    }

    private func worktreeCount(_ repo: TempRepo) throws -> Int {
      try repo.git("worktree", "list", "--porcelain").components(separatedBy: "\n").filter { $0.hasPrefix("worktree ") }.count
    }

    @Test func landsAMergeCommitWithoutTouchingTheMainCheckout() throws {
      let (repo, origin, other) = try setUp()
      defer { [repo, origin, other].forEach { $0.destroy() } }
      try repo.write("a.txt", "local edit\n")
      let head = try repo.git("rev-parse", "HEAD")

      let landed = try TaskLanding.mergeAndPush(branch: "fix", base: "main", remote: "origin", root: repo.root).get()
      #expect(!landed.retried)
      #expect(try origin.git("rev-parse", "main") == landed.commit)
      #expect(try repo.git("rev-parse", "origin/main") == landed.commit, "the push updates the remote-tracking branch")
      #expect(try repo.git("rev-list", "--parents", "-n", "1", landed.commit).split(separator: " ").count == 3)
      #expect(try repo.git("log", "-1", "--format=%s", landed.commit) == "Merge branch 'fix'")
      #expect(try repo.git("rev-parse", "HEAD") == head)
      #expect(try repo.read("a.txt") == "local edit\n")
      #expect(try worktreeCount(repo) == 1, "the throwaway worktree is gone")
    }

    @Test func mergesAgainWhenTheBaseMovedMeanwhile() throws {
      let (repo, origin, other) = try setUp()
      defer { [repo, origin, other].forEach { $0.destroy() } }
      // Someone else pushes after this clone last fetched.
      try other.commit(["c.txt": "other\n"], message: "other")
      try other.git("push", "-q", "origin", "main")

      let landed = try TaskLanding.mergeAndPush(branch: "fix", base: "main", remote: "origin", root: repo.root).get()
      #expect(landed.retried)
      #expect(try origin.git("rev-parse", "main") == landed.commit)
      #expect(try origin.git("show", "\(landed.commit):c.txt") == "other")
      #expect(try origin.git("show", "\(landed.commit):b.txt") == "fix")
      #expect(try worktreeCount(repo) == 1)
    }

    @Test func conflictsChangeNothing() throws {
      let (repo, origin, other) = try setUp()
      defer { [repo, origin, other].forEach { $0.destroy() } }
      try other.commit(["b.txt": "theirs\n"], message: "theirs")
      try other.git("push", "-q", "origin", "main")
      try repo.git("fetch", "-q", "origin")
      let before = try origin.git("rev-parse", "main")

      let result = TaskLanding.mergeAndPush(branch: "fix", base: "main", remote: "origin", root: repo.root)
      #expect(result == .failure(.conflicts(["b.txt"])))
      #expect(try origin.git("rev-parse", "main") == before)
      #expect(try worktreeCount(repo) == 1)
    }

    @Test func aServerRefusalIsToldApart() throws {
      let (repo, origin, other) = try setUp()
      defer { [repo, origin, other].forEach { $0.destroy() } }
      try origin.write(".git/hooks/pre-receive", "#!/bin/sh\necho 'You are not allowed to push to main.'\nexit 1\n")
      try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: origin.root + "/.git/hooks/pre-receive")

      let result = TaskLanding.mergeAndPush(branch: "fix", base: "main", remote: "origin", root: repo.root)
      guard case .failure(.refused(let output)) = result else {
        Issue.record("expected a refusal, got \(result)")
        return
      }
      #expect(output.contains("You are not allowed to push to main."))
    }

    @Test func aBranchWithNothingNewHasNothingToLand() throws {
      let (repo, origin, other) = try setUp()
      defer { [repo, origin, other].forEach { $0.destroy() } }
      try repo.git("branch", "-q", "--no-track", "empty", "origin/main")
      #expect(
        TaskLanding.mergeAndPush(branch: "empty", base: "main", remote: "origin", root: repo.root)
          == .failure(.nothingToLand))
      #expect(try worktreeCount(repo) == 1)
    }

    @Test func withoutARemoteItMergesIntoACleanMainCheckoutOnly() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "1\n"], message: "first")
      try repo.git("switch", "-q", "-c", "fix")
      try repo.commit(["b.txt": "fix\n"], message: "fix")
      try repo.git("switch", "-q", "-c", "other")
      #expect(TaskLanding.mergeLocally(branch: "fix", base: "main", root: repo.root) == .failure(.notOnBase(current: "other")))
      try repo.git("switch", "-q", "main")
      try repo.write("notes.txt", "draft\n")
      #expect(TaskLanding.mergeLocally(branch: "fix", base: "main", root: repo.root) == .failure(.uncommitted(1)))
      try FileManager.default.removeItem(atPath: repo.root + "/notes.txt")

      let landed = try TaskLanding.mergeLocally(branch: "fix", base: "main", root: repo.root).get()
      #expect(try repo.git("rev-parse", "HEAD") == landed.commit)
      #expect(try repo.git("log", "-1", "--format=%s") == "Merge branch 'fix'")
      #expect(repo.exists("b.txt"))
      #expect(TaskLanding.mergeLocally(branch: "fix", base: "main", root: repo.root) == .failure(.nothingToLand))
    }
  }
#endif
